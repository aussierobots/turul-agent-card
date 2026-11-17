mod common;
#[path = "support/trust.rs"]
mod trust;

use Severity::{Error, Warning};
use common::*;
use trust::*;
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::ArtifactSource;
use turul_ai_catalog::validate::{Conformance, ConformanceLevel, Severity};

const C: &str = "did:web:example.com";
/// No RFC 8785 form: the nearest double is not finite.
const BAD: &[&str] = &["1e400", "-1e400", "1.8e308"];
/// An RFC 8785 form that denotes another value: more precision than a double holds.
const INEXACT: &[&str] = &[
    "9007199254740993",
    "18446744073709551617",
    "1234567890123456789012345",
    "333333333.33333329",
    "1e-400",
];
const GOOD: &[&str] = &["1.0", "1e2", "0.1", "9007199254740991", "-3", "0"];

/// A valid `signature` member from `C`.
fn sig() -> String {
    format!(r#""signature":{}"#, good_signature(C))
}

/// One entry with `digest` and one substantive, unsigned manifest from `C` carrying `members`.
fn manifest_doc(members: &str) -> String {
    bound_doc(
        &entry_digest(),
        &[&manifest(C, &format!(",{members},{SUBSTANCE}"))],
    )
}

/// As `manifest_doc`, signed: the manifest also carries a binding `subject` and a valid `signature`.
fn signed_manifest_doc(members: &str) -> String {
    manifest_doc(&format!(
        r#"{members},"subject":{},{}"#,
        matching_subject(),
        sig()
    ))
}

// #signed-payload-and-jws-construction: "The payload MUST satisfy JCS's I-JSON constraints"; a signed manifest is the payload
#[test]
fn non_i_json_number_in_signed_manifest_is_an_error() {
    for n in BAD {
        let r = report(&signed_manifest_doc(&format!(r#""x":{n}"#)));
        assert_only(
            &r,
            "number-not-i-json",
            Error,
            "/entries/0/trustManifests/0/x",
        );
        assert_eq!(r.conformance(), Conformance::NotMet);
    }
    let r = report(&signed_manifest_doc(&format!(
        r#""x":{{"y":[1,{}]}}"#,
        BAD[0]
    )));
    assert_only(
        &r,
        "number-not-i-json",
        Error,
        "/entries/0/trustManifests/0/x/y/1",
    );
    for n in GOOD {
        assert_clean(&report(&signed_manifest_doc(&format!(r#""x":{n}"#))));
    }
}

// #signed-payload-and-jws-construction: a catalog signature covers the whole document
#[test]
fn non_i_json_number_in_signed_catalog_is_an_error() {
    for n in BAD {
        let text = format!(
            r#"{{"specVersion":"1.0","entries":[],"big":{n},{}}}"#,
            sig()
        );
        assert_only(&report(&text), "number-not-i-json", Error, "/big");
        // Inside an entry too: the whole snapshot is the payload.
        let text = format!(
            r#"{{"specVersion":"1.0","entries":[{}],{}}}"#,
            entry("a", &format!(r#","big":{n}"#)),
            sig()
        );
        assert_only(&report(&text), "number-not-i-json", Error, "/entries/0/big");
    }
}

// #catalog-entry `digest`: inline `data` "MUST satisfy JCS's input requirements for artifact verification to succeed"
#[test]
fn non_i_json_number_in_digested_data_is_an_error() {
    let d = format!("sha256:{}", hex(64));
    for n in BAD {
        let text = doc(&[&format!(
            r#"{{"identifier":"a","type":"t","digest":"{d}","data":{{"n":{n}}}}}"#
        )]);
        assert_only(
            &report(&text),
            "number-not-i-json",
            Error,
            "/entries/0/data/n",
        );
    }
}

// #signed-payload-and-jws-construction and #catalog-entry `digest` require I-JSON only of signed and digested values; unsigned documents are still conformant
#[test]
fn non_i_json_number_elsewhere_is_a_warning() {
    let d = format!("sha256:{}", hex(64));
    for n in BAD {
        // Unsigned catalog: unknown member, extension value, undigested data.
        let cases = [
            (
                format!(r#"{{"specVersion":"1.0","entries":[],"big":{n}}}"#),
                "/big".to_string(),
            ),
            (
                doc(&[&entry("a", &format!(r#","big":{n}"#))]),
                "/entries/0/big".to_string(),
            ),
            (
                doc(&[&entry(
                    "a",
                    &format!(r#","extensions":{{"com.x.y":[{n}]}}"#),
                )]),
                "/entries/0/extensions/com.x.y/0".to_string(),
            ),
            (
                doc(&[&format!(
                    r#"{{"identifier":"a","type":"t","data":{{"n":{n}}}}}"#
                )]),
                "/entries/0/data/n".to_string(),
            ),
            // A digest covers `data` only, not the other members of the entry.
            (
                doc(&[&format!(
                    r#"{{"identifier":"a","type":"t","digest":"{d}","data":{{}},"big":{n}}}"#
                )]),
                "/entries/0/big".to_string(),
            ),
            // A signature elsewhere does not make an unsigned manifest's numbers errors.
            (
                manifest_doc(&format!(r#""x":{n}"#)),
                "/entries/0/trustManifests/0/x".to_string(),
            ),
        ];
        for (text, ptr) in cases {
            let r = report(&text);
            assert_only(&r, "number-not-i-json", Warning, &ptr);
            assert!(!r.has_errors());
            assert_eq!(r.conformance(), Conformance::Met(ConformanceLevel::Minimal));
        }
    }
    for n in GOOD {
        assert_clean(&report(&format!(
            r#"{{"specVersion":"1.0","entries":[],"big":{n}}}"#
        )));
    }
}

// #signed-payload-and-jws-construction: large integers "SHOULD be encoded as strings", and RFC 7493
// section 2.2 excess precision is a SHOULD NOT; JCS still canonicalises such a number, so it is a
// warning even where a signature or digest covers it
#[test]
fn inexact_number_is_a_warning_in_every_context() {
    let d = format!("sha256:{}", hex(64));
    for n in INEXACT {
        let cases = [
            (
                signed_manifest_doc(&format!(r#""x":{n}"#)),
                "/entries/0/trustManifests/0/x".to_string(),
            ),
            (
                format!(
                    r#"{{"specVersion":"1.0","entries":[],"big":{n},{}}}"#,
                    sig()
                ),
                "/big".to_string(),
            ),
            (
                doc(&[&format!(
                    r#"{{"identifier":"a","type":"t","digest":"{d}","data":{{"n":{n}}}}}"#
                )]),
                "/entries/0/data/n".to_string(),
            ),
            (
                format!(r#"{{"specVersion":"1.0","entries":[],"big":{n}}}"#),
                "/big".to_string(),
            ),
        ];
        for (text, ptr) in cases {
            let r = report(&text);
            assert_only(&r, "number-inexact", Warning, &ptr);
            assert!(!r.has_errors(), "{n}");
        }
    }
}

// #signed-payload-and-jws-construction: the validator reads the document; it never rewrites or rounds a number
#[test]
fn validate_never_rewrites_numbers() {
    let text = doc(&[&format!(
        r#"{{"identifier":"a","type":"t","data":{{"n":{}}}}}"#,
        INEXACT[1]
    )]);
    let c = ok(&text);
    let before = c.clone();
    let r = turul_ai_catalog::validate::validate(&c);
    assert_eq!(found(&r).len(), 1);
    assert_eq!(c, before);
    let ArtifactSource::Data(Json::Object(m)) = &c.entries[0].source else {
        panic!("data object")
    };
    let Some(Json::Number(n)) = m.get("n") else {
        panic!("number")
    };
    assert_eq!(n.as_str(), INEXACT[1]);
}

// #attestation-object: the typed `size` is a `uint` that parses above 2^53, and the walk sees it.
// JCS writes it as another integer, so it is `number-inexact`, signed or not.
#[test]
fn typed_size_above_2_pow_53_is_reported() {
    let att = |size: &str| format!(r#""attestations":[{{"type":"t","uri":"u","size":{size}}}]"#);
    let ptr = "/entries/0/trustManifests/0/attestations/0/size";
    assert_only(
        &report(&manifest_doc(&att("9007199254740993"))),
        "number-inexact",
        Warning,
        ptr,
    );
    assert_only(
        &report(&signed_manifest_doc(&att("18446744073709551615"))),
        "number-inexact",
        Warning,
        ptr,
    );
    assert_clean(&report(&manifest_doc(&att("245760"))));
}
