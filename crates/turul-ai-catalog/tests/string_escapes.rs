mod common;

use common::*;
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::{ArtifactSource, Members};
use turul_ai_catalog::parse::ParseErrorKind::*;

fn description_doc(json_string_body: &str) -> String {
    doc_with_entry_extra(&format!(r#""description":"{json_string_body}""#))
}

// #signed-payload-and-jws-construction: signed payloads are I-JSON, so no unpaired surrogate
#[test]
fn unpaired_surrogate_escape_is_parse_error() {
    for body in [
        r"\ud800",
        r"\udbff",
        r"\udc00x",
        r"\udfff",
        r"\ud800x",
        r"\ud800A",
        r"\ud800\ud800",
        r"a\udc00",
    ] {
        assert_err!(
            &description_doc(body),
            LoneSurrogate,
            "/entries/0/description"
        );
    }
    // Member name: the pointer is the containing object.
    assert_err!(
        &doc_with_entry_extra(r#""x":{"\ud800":1}"#),
        LoneSurrogate,
        "/entries/0/x"
    );
    // Inside data.
    assert_err!(
        &doc_with_entry_body(r#""identifier":"i","type":"t","data":["\udc00"]"#),
        LoneSurrogate,
        "/entries/0/data/0"
    );
}

// #signed-payload-and-jws-construction: a valid surrogate pair is one scalar value
#[test]
fn surrogate_pair_decodes_to_one_scalar() {
    let c = ok(&description_doc(r"\ud83d\ude00"));
    assert_eq!(c.entries[0].description.as_deref(), Some("\u{1F600}"));
    let raw = ok(&description_doc("\u{1F600}"));
    assert_eq!(raw.entries[0].description, c.entries[0].description);
}

// RFC 8259 section 8.2 allows noncharacters. I-JSON (RFC 7493 section 2.1) forbids them only
// where #signed-payload-and-jws-construction requires I-JSON, which is not a parse rule.
#[test]
fn noncharacters_parse_and_are_kept_unchanged() {
    let esc = |hex: &str| format!("\\u{hex}");
    for (body, expected) in [
        ("\u{FDD0}".to_string(), "\u{FDD0}"),
        ("\u{FDEF}".to_string(), "\u{FDEF}"),
        ("\u{FFFE}".to_string(), "\u{FFFE}"),
        ("\u{FFFF}".to_string(), "\u{FFFF}"),
        ("\u{1FFFE}".to_string(), "\u{1FFFE}"),
        ("\u{10FFFF}".to_string(), "\u{10FFFF}"),
        (esc("fdd0"), "\u{FDD0}"),
        (esc("FFFF"), "\u{FFFF}"),
        (esc("d83f") + &esc("dffe"), "\u{1FFFE}"),
        (esc("dbff") + &esc("dfff"), "\u{10FFFF}"),
    ] {
        let c = ok(&description_doc(&body));
        assert_eq!(
            c.entries[0].description.as_deref(),
            Some(expected),
            "{body:?}"
        );
    }
    // Member name and data.
    let c = ok(&doc_with_entry_body(&format!(
        r#""identifier":"i","type":"t","data":{{"{}":"{}"}},"x{}":1"#,
        esc("fffe"),
        esc("ffff"),
        esc("fdd0")
    )));
    let mut data = Members::default();
    data.insert("\u{FFFE}".to_string(), Json::String("\u{FFFF}".to_string()));
    assert_eq!(
        c.entries[0].source,
        ArtifactSource::Data(Json::Object(data))
    );
    assert!(c.entries[0].unrecognized.get("x\u{FDD0}").is_some());
}

// RFC 8259 section 7
#[test]
fn every_rfc_8259_escape_decodes() {
    let c = ok(&description_doc(
        r#"\"\\\/\b\f\n\r\t\u0041\u00e9\u00E9\u0000"#,
    ));
    assert_eq!(
        c.entries[0].description.as_deref(),
        Some("\"\\/\u{8}\u{c}\n\r\tA\u{e9}\u{e9}\u{0}")
    );
}

// RFC 8259 section 7: other escapes and raw control characters are not JSON
#[test]
fn invalid_escape_or_raw_control_character_is_syntax_error() {
    for body in [
        r"\x", r"\a", r"\u12", r"\u12G4", r"\", "\u{1}", "\n", "\t", "\u{1f}",
    ] {
        let e = err(&description_doc(body));
        assert!(matches!(e.kind, Syntax), "{body:?}: {:?}", e.kind);
        assert_eq!(e.pointer.to_string(), "/entries/0/description", "{body:?}");
    }
}
