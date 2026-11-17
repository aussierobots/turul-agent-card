mod common;

use common::*;
use turul_ai_catalog::parse::ParseErrorKind::*;

// #signed-payload-and-jws-construction: "parsers MUST reject duplicate JSON member names"
#[test]
fn duplicate_member_name_at_any_depth_is_parse_error() {
    // Catalog root; pointer is the second occurrence.
    assert_err!(
        r#"{"specVersion":"1.0","entries":[],"entries":[]}"#,
        DuplicateMember,
        "/entries"
    );
    // Entry, host and publisher.
    assert_err!(
        &doc_with_entry_extra(r#""description":"a","description":"b""#),
        DuplicateMember,
        "/entries/0/description"
    );
    assert_err!(
        r#"{"specVersion":"1.0","host":{"displayName":"a","displayName":"b"},"entries":[]}"#,
        DuplicateMember,
        "/host/displayName"
    );
    // Inside `data`, `extensions` and unrecognised members.
    assert_err!(
        &doc_with_entry_body(
            r#""identifier":"i","type":"t","data":{"a":1,"b":{"c":[{"d":1,"d":2}]}}"#
        ),
        DuplicateMember,
        "/entries/0/data/b/c/0/d"
    );
    assert_err!(
        &doc_with_entry_extra(r#""extensions":{"https://x.example/e":{"k":1,"k":2}}"#),
        DuplicateMember,
        "/entries/0/extensions/https:~1~1x.example~1e/k"
    );
    assert_err!(
        &doc_with_entry_extra(r#""unknown":{"k":1,"k":2}"#),
        DuplicateMember,
        "/entries/0/unknown/k"
    );
    // Trust objects.
    assert_err!(
        &doc_with_manifest(
            r#"{"contributor":"c","trustSchema":{"identifier":"i","version":"1","version":"2"}}"#
        ),
        DuplicateMember,
        "/entries/0/trustManifests/0/trustSchema/version"
    );
    assert_err!(
        &doc_with_manifest(r#"{"contributor":"c","additionalSignatures":{"a":1,"a":1}}"#),
        DuplicateMember,
        "/entries/0/trustManifests/0/additionalSignatures/a"
    );
}

// RFC 8259 section 8.3: names are compared after unescaping
#[test]
fn duplicate_names_are_compared_after_unescaping() {
    assert_err!(
        &doc_with_entry_body(r#""identifier":"i","type":"t","data":{"a":1,"\u0061":2}"#),
        DuplicateMember,
        "/entries/0/data/a"
    );
    assert_err!(
        r#"{"specVersion":"1.0","entries":[],"\u0065ntries":[]}"#,
        DuplicateMember,
        "/entries"
    );
    assert_err!(
        &doc_with_entry_extra(concat!(
            r#""x":{"\ud83d\ude00":1,""#,
            "\u{1F600}",
            r#"":2}"#
        )),
        DuplicateMember,
        "/entries/0/x/\u{1F600}"
    );
    // Distinct code point sequences are not duplicates.
    ok(&doc_with_entry_body(
        r#""identifier":"i","type":"t","data":{"a":1,"A":2,"a ":3}"#,
    ));
}

// #consumer-behavior: reader errors come before the version gate
#[test]
fn duplicate_is_reported_before_the_spec_version_gate() {
    assert_err!(
        r#"{"specVersion":"2.0","specVersion":"1.0"}"#,
        DuplicateMember,
        "/specVersion"
    );
}
