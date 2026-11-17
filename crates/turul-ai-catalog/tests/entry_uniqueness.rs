mod common;

use common::*;
use std::fs;
use std::path::PathBuf;
use turul_ai_catalog::validate::Severity::Error;

fn v(identifier: &str, version: &str) -> String {
    entry(identifier, &format!(r#","version":"{version}""#))
}

fn sorted(r: &turul_ai_catalog::validate::Report) -> Vec<(String, String)> {
    let mut f: Vec<_> = found(r).into_iter().map(|(c, _, p)| (c, p)).collect();
    f.sort();
    f
}

// #multi-version-entries: "MAY contain multiple entries with the same `identifier` and different `version` values"
#[test]
fn one_identifier_with_distinct_versions_is_valid() {
    let bytes = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/spec/blocks/03-multi-version-entries.json"),
    )
    .unwrap();
    let r = turul_ai_catalog::validate::validate(&turul_ai_catalog::parse::parse(&bytes).unwrap());
    assert_clean(&r);
    assert_clean(&report(&doc(&[&v("a", "1"), &v("a", "2"), &v("a", "10")])));
}

// #multi-version-entries: (`identifier`, `version`) "MUST be unique within the catalog"
#[test]
fn repeated_identifier_and_version_is_entry_duplicate() {
    assert_only(
        &report(&doc(&[&v("a", "1"), &v("a", "1")])),
        "entry-duplicate",
        Error,
        "/entries/1",
    );
    // Only later occurrences are flagged; other fields do not matter.
    let r = report(&doc(&[
        &v("a", "1"),
        &v("b", "1"),
        &v("a", "1"),
        &v("a", "1"),
    ]));
    assert_eq!(
        sorted(&r),
        [
            ("entry-duplicate".to_string(), "/entries/2".to_string()),
            ("entry-duplicate".to_string(), "/entries/3".to_string())
        ]
    );
}

// #multi-version-entries: without `version`, "`identifier` alone MUST be unique"
#[test]
fn repeated_versionless_identifier_is_entry_duplicate() {
    assert_only(
        &report(&doc(&[&entry("a", ""), &entry("a", "")])),
        "entry-duplicate",
        Error,
        "/entries/1",
    );
}

// #catalog-entry `version`: "REQUIRED when a catalog lists multiple versions of the same `identifier`"
#[test]
fn mixed_versioned_and_versionless_group_is_version_missing_in_group() {
    assert_only(
        &report(&doc(&[&v("a", "1"), &entry("a", "")])),
        "version-missing-in-group",
        Error,
        "/entries/1",
    );
    assert_only(
        &report(&doc(&[&entry("a", ""), &v("a", "1")])),
        "version-missing-in-group",
        Error,
        "/entries/0",
    );
    // Every versionless entry in the group is flagged; the repeated one is also a duplicate.
    let r = report(&doc(&[&entry("a", ""), &v("a", "1"), &entry("a", "")]));
    assert_eq!(
        sorted(&r),
        [
            ("entry-duplicate".to_string(), "/entries/2".to_string()),
            (
                "version-missing-in-group".to_string(),
                "/entries/0".to_string()
            ),
            (
                "version-missing-in-group".to_string(),
                "/entries/2".to_string()
            ),
        ]
    );
    // Other identifiers are unaffected.
    assert_clean(&report(&doc(&[
        &v("a", "1"),
        &entry("b", ""),
        &v("a", "2"),
    ])));
}

// #multi-version-entries: `version: ""` is a present version, distinct from absent
#[test]
fn empty_version_is_a_present_version() {
    assert_clean(&report(&doc(&[&v("a", ""), &v("a", "1")])));
    assert_only(
        &report(&doc(&[&v("a", ""), &entry("a", "")])),
        "version-missing-in-group",
        Error,
        "/entries/1",
    );
    assert_only(
        &report(&doc(&[&v("a", ""), &v("a", "")])),
        "entry-duplicate",
        Error,
        "/entries/1",
    );
}

// #catalog-entry `identifier`: an empty identifier names nothing
#[test]
fn empty_identifier_is_identifier_empty() {
    assert_only(
        &report(&doc(&[&entry("", "")])),
        "identifier-empty",
        Error,
        "/entries/0/identifier",
    );
}

// #catalog-entry `identifier`: open text format; no `:` is not a defect
#[test]
fn identifier_without_colon_gives_no_diagnostic() {
    assert_clean(&report(&doc(&[
        &entry("plainname", ""),
        &entry("not a urn", ""),
    ])));
}
