mod common;
#[path = "support/trust.rs"]
mod trust;

use Severity::{Error, Warning};
use common::*;
use trust::*;
use turul_ai_catalog::validate::{Conformance, Severity};

const C: &str = "did:web:example.com";

/// A valid `signature` member from `C`.
fn sig() -> String {
    format!(r#""signature":{}"#, good_signature(C))
}

// JSON escapes of noncharacters: U+FDD0, U+FDEF, U+FFFE, U+FFFF, U+1FFFF (pair), U+10FFFF (pair).
const NONCHARS: &[&str] = &[r"﷐", r"﷯", r"￾", r"￿", r"🿿", r"􏿿"];

fn digested(data: &str) -> String {
    doc(&[&format!(
        r#"{{"identifier":"a","type":"t","digest":"sha256:{}","data":{data}}}"#,
        hex(64)
    )])
}

fn unsigned_member(value: &str) -> String {
    doc(&[&entry("a", &format!(r#","note":{value}"#))])
}

// #signed-payload-and-jws-construction: I-JSON forbids noncharacters; a signed or digested value cannot be computed over one
#[test]
fn noncharacter_in_signed_or_digested_context_is_an_error() {
    for c in NONCHARS {
        // value inside digested data
        let r = report(&digested(&format!(r#"{{"s":"a{c}b"}}"#)));
        assert_only(&r, "string-not-i-json", Error, "/entries/0/data/s");
        assert_eq!(r.conformance(), Conformance::NotMet);
        // signed root, unknown member
        let text = format!(
            r#"{{"specVersion":"1.0","entries":[],"note":"{c}",{}}}"#,
            sig()
        );
        assert_only(&report(&text), "string-not-i-json", Error, "/note");
        // signed manifest
        let m = manifest(
            C,
            &format!(
                r#","note":["{c}"],"subject":{},{}"#,
                matching_subject(),
                sig()
            ),
        );
        let text = bound_doc(&entry_digest(), &[&m]);
        assert_only(
            &report(&text),
            "string-not-i-json",
            Error,
            "/entries/0/trustManifests/0/note/0",
        );
    }
    // a member name holding one is reported at that member
    let name = '\u{FFFF}';
    let r = report(&digested(r#"{"￿":1}"#));
    assert_only(
        &r,
        "string-not-i-json",
        Error,
        &format!("/entries/0/data/{name}"),
    );
}

// #signed-payload-and-jws-construction: elsewhere an unsigned document is valid, so the diagnostic is a warning
#[test]
fn noncharacter_elsewhere_is_a_warning() {
    for c in NONCHARS {
        let r = report(&unsigned_member(&format!(r#""a{c}b""#)));
        assert_only(&r, "string-not-i-json", Warning, "/entries/0/note");
        assert!(!r.has_errors());
        // inline data without a digest
        let text = doc(&[&format!(
            r#"{{"identifier":"a","type":"t","data":{{"s":"{c}"}}}}"#
        )]);
        assert_only(
            &report(&text),
            "string-not-i-json",
            Warning,
            "/entries/0/data/s",
        );
    }
    let name = '\u{FFFE}';
    let text = doc(&[&entry("a", r#","￾":1"#)]);
    assert_only(
        &report(&text),
        "string-not-i-json",
        Warning,
        &format!("/entries/0/{name}"),
    );
    // Boundaries: neighbours of the noncharacter ranges are ordinary characters.
    for ok_char in [r"﷏", r"ﷰ", r"�", r"🿽", "\u{10FFFD}", "\u{FFFC}"] {
        assert_clean(&report(&unsigned_member(&format!(r#""{ok_char}""#))));
    }
}
