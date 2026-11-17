mod common;

use common::*;
use turul_ai_catalog::model::ArtifactSource;
use turul_ai_catalog::parse::ParseErrorKind::*;

// #catalog-entry: "MUST contain exactly one of" url, data. Both is refused.
#[test]
fn entry_with_both_url_and_data_is_parse_error() {
    assert_err!(
        &doc_with_entry_body(
            r#""identifier":"i","type":"t","url":"https://example.com/a","data":{}"#
        ),
        ArtifactSourceConflict,
        "/entries/0"
    );
    assert_err!(
        &format!(
            r#"{{"specVersion":"1.0","entries":[{ENTRY},{{"identifier":"j","type":"t","url":"u","data":1}}]}}"#
        ),
        ArtifactSourceConflict,
        "/entries/1"
    );
}

// #catalog-entry: "MUST contain exactly one of" url, data. Neither is refused.
#[test]
fn entry_with_neither_url_nor_data_is_parse_error() {
    assert_err!(
        &doc_with_entry_body(r#""identifier":"i","type":"t""#),
        ArtifactSourceMissing,
        "/entries/0"
    );
}

// #cddl-schema: `data: any`, so `data: null` is present
#[test]
fn data_null_is_present() {
    let c = ok(&doc_with_entry_body(
        r#""identifier":"i","type":"t","data":null"#,
    ));
    assert_eq!(
        c.entries[0].source,
        ArtifactSource::Data(turul_ai_catalog::json::Json::Null)
    );
}

// #catalog-entry: `url` is `text`; an empty string is present
#[test]
fn empty_url_is_present() {
    let c = ok(&doc_with_entry_body(
        r#""identifier":"i","type":"t","url":"""#,
    ));
    assert_eq!(c.entries[0].source, ArtifactSource::Url(String::new()));
}

// #design-goals: the index carries any artifact without interpreting its schema
#[test]
fn data_of_any_json_shape_with_unknown_type_parses() {
    for data in [
        "true",
        "0",
        r#""text""#,
        "[]",
        "[1,[2,[3]]]",
        "{}",
        r#"{"anything":{"goes":[null,false]}}"#,
        r#"{"specVersion":"9.9","entries":"not even a catalog"}"#,
    ] {
        let c = ok(&doc_with_entry_body(&format!(
            r#""identifier":"i","type":"application/x-unknown-thing","data":{data}"#
        )));
        assert!(
            matches!(c.entries[0].source, ArtifactSource::Data(_)),
            "{data}"
        );
    }
    // Opaque even when `type` says catalog: parse does not look inside.
    let c = ok(&doc_with_entry_body(
        r#""identifier":"i","type":"application/ai-catalog+json","data":{"not":"a catalog"}"#,
    ));
    assert!(matches!(c.entries[0].source, ArtifactSource::Data(_)));
}

// #embedded-content-safety: `data` is well-formed JSON, by construction
#[test]
fn data_is_well_formed_json_by_construction() {
    for bad in [
        r#"{"a":}"#,
        "[1,]",
        "{'a':1}",
        r#"{"a":1"#,
        "nul",
        r#"{"a" 1}"#,
    ] {
        let e = err(&doc_with_entry_body(&format!(
            r#""identifier":"i","type":"t","data":{bad}"#
        )));
        assert!(matches!(e.kind, Syntax), "{bad}: {:?}", e.kind);
        assert!(
            e.pointer.to_string().starts_with("/entries/0/data"),
            "{bad}: {}",
            e.pointer
        );
    }
}

// #catalog-entry `url`: an empty string is not a reference to anything
#[test]
fn empty_url_is_url_empty() {
    let r = report(&doc_with_entry_body(
        r#""identifier":"i","type":"t","url":"""#,
    ));
    assert_only(
        &r,
        "url-empty",
        turul_ai_catalog::validate::Severity::Error,
        "/entries/0/url",
    );
}
