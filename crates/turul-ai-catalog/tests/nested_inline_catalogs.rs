mod common;

use common::*;
use turul_ai_catalog::validate::{
    Conformance, ConformanceLevel, MAX_NESTING_DEPTH, NestingDepth, Severity::*, validate_at,
};

const CAT: &str = "application/ai-catalog+json";

fn nested_entry(id: &str, ty: &str, data: &str) -> String {
    format!(r#"{{"identifier":"{id}","type":"{ty}","data":{data}}}"#)
}

/// A catalog of `levels` document levels (the root is level 1), each holding one nested catalog;
/// the deepest holds the plain entry `leaf`.
fn chain(levels: usize, leaf: &str) -> String {
    let mut d = doc(&[leaf]);
    for _ in 1..levels {
        d = doc(&[&nested_entry("n", CAT, &d)]);
    }
    d
}

const DATA: &str = "/entries/0/data";

fn data_path(times: usize) -> String {
    "/entries/0/data".repeat(times)
}

// #nested-catalog-depth-and-circular-references: "A depth limit of 4 is RECOMMENDED"; the root is level 1
#[test]
fn four_levels_are_validated() {
    let r = report(&chain(4, &entry("leaf", "")));
    assert_clean(&r);
    assert_eq!(r.conformance(), Conformance::Met(ConformanceLevel::Minimal));
    // The deepest level is judged: its defect carries the full path.
    let r = report(&chain(4, &entry("", "")));
    assert_only(
        &r,
        "identifier-empty",
        Error,
        &format!("{}/entries/0/identifier", data_path(3)),
    );
}

// #nested-catalog-entries: clients "SHOULD impose a maximum nesting depth"; content past it is left unvalidated
#[test]
fn fifth_level_is_nested_depth_limit_and_undetermined() {
    let r = report(&chain(5, &entry("", "")));
    // The empty identifier on level 5 is not reported: that level is not descended.
    assert_only(&r, "nested-depth-limit", Warning, &data_path(4));
    assert_eq!(r.conformance(), Conformance::Undetermined);
    assert!(!r.has_errors());
    // A known defect is not undetermined: errors win.
    let text = doc(&[
        &entry("", ""),
        &nested_entry("n", CAT, &chain(4, &entry("leaf", ""))),
    ]);
    let r = report(&text);
    assert_eq!(r.conformance(), Conformance::NotMet);
    // Deeper still gives one warning, at the same place.
    let r = report(&chain(7, &entry("leaf", "")));
    assert_only(&r, "nested-depth-limit", Warning, &data_path(4));
}

// #nested-catalog-entries: a catalog fetched through a nested `url` starts at the depth it was reached at
#[test]
fn validate_at_starts_from_the_given_depth() {
    let d2 = NestingDepth::ROOT.child().unwrap();
    let d4 = d2.child().unwrap().child().unwrap();
    assert_eq!(d2.get(), 2);
    assert_eq!(d4.get(), 4);

    let three = ok(&chain(3, &entry("leaf", "")));
    assert_clean(&validate_at(&three, d2));
    let four = ok(&chain(4, &entry("leaf", "")));
    let r = validate_at(&four, d2);
    assert_only(&r, "nested-depth-limit", Warning, &data_path(3));
    assert_eq!(r.conformance(), Conformance::Undetermined);

    let two = ok(&chain(2, &entry("leaf", "")));
    let r = validate_at(&two, d4);
    assert_only(&r, "nested-depth-limit", Warning, DATA);
    // A catalog with no nested entry is unaffected by its depth.
    assert_clean(&validate_at(&ok(&chain(1, &entry("leaf", ""))), d4));
}

// #nested-catalog-entries: "MAY support deeper nesting but SHOULD document their limit"; the limit is 4
#[test]
fn max_nesting_depth_is_four() {
    assert_eq!(MAX_NESTING_DEPTH, 4);
    assert_eq!(NestingDepth::ROOT.get(), 1);
    let mut d = NestingDepth::ROOT;
    for expected in 2..=MAX_NESTING_DEPTH {
        d = d.child().expect("below the limit");
        assert_eq!(d.get(), expected);
    }
    assert!(d.child().is_none());
}

// #nested-catalog-entries: a nested catalog "MAY reuse the same `identifier`" as an entry of its parent
#[test]
fn nested_entry_may_reuse_a_parent_identifier() {
    let inner = doc(&[&entry("urn:x:a", ""), &entry("urn:x:b", "")]);
    let text = doc(&[
        &entry("urn:x:a", ""),
        &entry("urn:x:b", ""),
        &nested_entry("urn:x:a2", CAT, &inner),
    ]);
    assert_clean(&report(&text));
    // The parent's own scope is still checked, and so is the nested scope.
    let inner = doc(&[&entry("urn:x:a", ""), &entry("urn:x:a", "")]);
    let text = doc(&[&entry("urn:x:a", ""), &nested_entry("m", CAT, &inner)]);
    assert_only(
        &report(&text),
        "entry-duplicate",
        Error,
        "/entries/1/data/entries/1",
    );
}

// #nested-catalog-entries: an inline catalog that is not a catalog is one diagnostic at the failing pointer
#[test]
fn nested_shape_error_is_one_diagnostic_under_data() {
    let cases = [
        (r#"{"specVersion":"1.0"}"#, "/entries/0/data/entries"),
        (r#""text""#, "/entries/0/data"),
        ("null", "/entries/0/data"),
        ("[]", "/entries/0/data"),
        (
            r#"{"specVersion":"1.0","entries":[{"identifier":"i","type":"t"}]}"#,
            "/entries/0/data/entries/0",
        ),
        (
            r#"{"specVersion":"1.0","entries":[{"identifier":"i","type":"t","url":"u","data":1}]}"#,
            "/entries/0/data/entries/0",
        ),
        (
            r#"{"specVersion":"1.0","entries":[{"type":"t","url":"u"}]}"#,
            "/entries/0/data/entries/0/identifier",
        ),
    ];
    for (data, ptr) in cases {
        let r = report(&doc(&[&nested_entry("n", CAT, data)]));
        assert_only(&r, "nested-catalog-invalid", Error, ptr);
        assert_eq!(r.conformance(), Conformance::NotMet);
    }
}

// #compatibility-rules: a nested document of another major is rejected like a fetched one
#[test]
fn nested_unsupported_major_is_nested_catalog_invalid() {
    for v in ["2.0", "0.9", "x"] {
        let data = format!(r#"{{"specVersion":"{v}"}}"#);
        assert_only(
            &report(&doc(&[&nested_entry("n", CAT, &data)])),
            "nested-catalog-invalid",
            Error,
            "/entries/0/data/specVersion",
        );
    }
    // Any minor of major 1 passes.
    assert_clean(&report(&doc(&[&nested_entry(
        "n",
        CAT,
        r#"{"specVersion":"1.7","entries":[]}"#,
    )])));
}

// #nested-catalog-entries: a nested document's diagnostics are appended under `/entries/i/data`
#[test]
fn nested_diagnostics_carry_the_data_prefix() {
    let inner = format!(
        r#"{{"specVersion":"1.0","host":{{"displayName":"h","logoUrl":"https://x/y.png"}},"entries":[{},{},{}]}}"#,
        entry("a", ""),
        entry("", ""),
        entry("a", "")
    );
    let text = doc(&[&entry("p", ""), &nested_entry("n", CAT, &inner)]);
    let r = report(&text);
    assert_eq!(
        found(&r),
        vec![
            (
                "logo-url-not-data-uri".into(),
                Warning,
                "/entries/1/data/host/logoUrl".into()
            ),
            (
                "identifier-empty".into(),
                Error,
                "/entries/1/data/entries/1/identifier".into()
            ),
            (
                "entry-duplicate".into(),
                Error,
                "/entries/1/data/entries/2".into()
            ),
        ]
    );
}

// #level-1-minimal-catalog, #nested-catalog-entries: `host` is not required in a nested catalog, and its presence never sets the parent's level
#[test]
fn nested_catalog_needs_no_host() {
    let no_host = nested_entry("n", CAT, &doc(&[&entry("a", "")]));
    let text =
        format!(r#"{{"specVersion":"1.0","host":{{"displayName":"h"}},"entries":[{no_host}]}}"#);
    let r = report(&text);
    assert_clean(&r);
    assert_eq!(
        r.conformance(),
        Conformance::Met(ConformanceLevel::Discoverable)
    );

    let with_host = nested_entry(
        "n",
        CAT,
        r#"{"specVersion":"1.0","host":{"displayName":"h"},"entries":[]}"#,
    );
    let r = report(&doc(&[&with_host]));
    assert_clean(&r);
    assert_eq!(r.conformance(), Conformance::Met(ConformanceLevel::Minimal));
}

// #nested-catalog-entries: only the exact `type` embeds a catalog; any other `data` is opaque
#[test]
fn other_case_type_keeps_data_opaque() {
    for ty in [
        "Application/AI-Catalog+JSON",
        "application/ai-catalog+json ",
        "application/ai-catalog+json; charset=utf-8",
        "application/x-other",
    ] {
        let r = report(&doc(&[&nested_entry("n", ty, r#"{"not":"a catalog"}"#)]));
        assert_clean(&r);
    }
}
