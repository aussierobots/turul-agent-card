mod common;

use std::fs;
use std::path::PathBuf;

use common::*;
use turul_ai_catalog::identity::EntryIdentifier;
use turul_ai_catalog::model::AiCatalog;
use turul_ai_catalog::resolve::latest;

const ID: &str = "urn:air:example.com:agent:x";

/// Entries as (version, updatedAt) pairs, all with identifier ID; url distinguishes them.
fn group(items: &[(Option<&str>, Option<&str>)]) -> AiCatalog {
    let entries: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, (v, u))| {
            let v = v.map(|v| format!(r#","version":"{v}""#)).unwrap_or_default();
            let u = u.map(|u| format!(r#","updatedAt":"{u}""#)).unwrap_or_default();
            format!(
                r#"{{"identifier":"{ID}","type":"application/mcp-server-card+json","url":"https://example.com/{i}.json"{v}{u}}}"#
            )
        })
        .collect();
    ok(&format!(
        r#"{{"specVersion":"1.0","entries":[{}]}}"#,
        entries.join(",")
    ))
}

fn id() -> EntryIdentifier {
    EntryIdentifier::new(ID)
}

/// The index of the selected entry (by url), or None.
fn pick(c: &AiCatalog) -> Option<usize> {
    latest(c, &id()).map(|e| {
        let s = format!("{:?}", e.source);
        (0..c.entries.len())
            .find(|i| s.contains(&format!("/{i}.json")))
            .unwrap()
    })
}

fn versions<'a>(vs: &[&'a str]) -> Vec<(Option<&'a str>, Option<&'a str>)> {
    vs.iter().map(|v| (Some(*v), None)).collect()
}

// #multi-version-entries: sort by version when parseable as a semantic version
#[test]
fn highest_semver_precedence_is_latest() {
    for (vs, want) in [
        (vec!["2.0.0", "2.1.0"], "2.1.0"),
        (vec!["1.0.0-rc.1", "1.0.0"], "1.0.0"),
        (vec!["1.9.0", "1.10.0"], "1.10.0"),
        (
            vec!["1.0.0-alpha", "1.0.0-alpha.1", "1.0.0-beta"],
            "1.0.0-beta",
        ),
        (vec!["1.0.0-1", "1.0.0-alpha"], "1.0.0-alpha"),
        (
            vec!["1.0.0-alpha.1", "1.0.0-alpha.beta"],
            "1.0.0-alpha.beta",
        ),
    ] {
        let items = versions(&vs);
        let c = group(&items);
        let got = latest(&c, &id()).unwrap().version.as_deref().unwrap();
        assert_eq!(got, want, "{vs:?}");
    }
}

// #multi-version-entries: fall back to updatedAt when versions are not all semantic versions
#[test]
fn non_semver_group_selects_by_updated_at() {
    let c = group(&[
        (Some("2026-a"), Some("2026-01-01T00:00:00Z")),
        (Some("2026-b"), Some("2026-03-01T00:00:00Z")),
        (None, Some("2026-02-01T00:00:00Z")),
    ]);
    assert_eq!(pick(&c), Some(1));
    // instants, not text: +10:00 is earlier in UTC
    let c = group(&[
        (None, Some("2026-01-01T10:00:00+10:00")),
        (None, Some("2026-01-01T05:00:00Z")),
    ]);
    assert_eq!(pick(&c), Some(1));
}

// #multi-version-entries: the result never depends on array order
#[test]
fn latest_never_depends_on_array_order() {
    let groups: Vec<Vec<(Option<&str>, Option<&str>)>> = vec![
        versions(&["1.0.0", "2.0.0", "1.5.0"]),
        vec![
            (None, Some("2026-01-01T00:00:00Z")),
            (None, Some("2026-03-01T00:00:00Z")),
            (None, Some("2026-02-01T00:00:00Z")),
        ],
        versions(&["1.0.0+a", "1.0.0+b"]),
    ];
    for g in groups {
        let fwd = group(&g);
        let mut rev = g.clone();
        rev.reverse();
        let rev = group(&rev);
        let a = latest(&fwd, &id()).map(|e| (e.version.clone(), e.updated_at.clone()));
        let b = latest(&rev, &id()).map(|e| (e.version.clone(), e.updated_at.clone()));
        assert_eq!(a, b, "{g:?}");
    }
}

// #multi-version-entries: spec example selects the 2.1.0 entry
#[test]
fn spec_multi_version_example_selects_newest() {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/spec/blocks/03-multi-version-entries.json");
    let c = ok(&fs::read_to_string(p).unwrap());
    let e = latest(&c, &EntryIdentifier::new("urn:air:acme.com:agent:finance")).unwrap();
    assert_eq!(e.version.as_deref(), Some("2.1.0"));
}

// #multi-version-entries: equal precedence or equal instants have no single latest
#[test]
fn equal_precedence_or_instant_gives_none() {
    assert_eq!(pick(&group(&versions(&["1.0.0+a", "1.0.0+b"]))), None);
    let c = group(&[
        (None, Some("2026-01-01T10:00:00+10:00")),
        (None, Some("2026-01-01T00:00:00Z")),
    ]);
    assert_eq!(pick(&c), None);
}

// #multi-version-entries: a group with no usable order has no latest
#[test]
fn group_without_an_order_gives_none() {
    // one entry lacks updatedAt, versions not all semver
    let c = group(&[(Some("a"), Some("2026-01-01T00:00:00Z")), (Some("b"), None)]);
    assert_eq!(pick(&c), None);
    // neither version nor updatedAt
    assert_eq!(pick(&group(&[(None, None), (None, None)])), None);
    // mixed semver and non-semver versions, no updatedAt
    assert_eq!(pick(&group(&versions(&["1.0.0", "latest"]))), None);
}

// #multi-version-entries: a group of one is its own latest; an empty group has none
#[test]
fn single_entry_group_is_its_own_latest() {
    let c = group(&[(None, None)]);
    assert_eq!(pick(&c), Some(0));
    let c = group(&[(Some("not semver"), None)]);
    assert_eq!(pick(&c), Some(0));
    assert!(latest(&c, &EntryIdentifier::new("urn:air:example.com:agent:other")).is_none());
}

// #multi-version-entries: only entries with exactly the identifier form the group
#[test]
fn only_exact_identifier_matches_join_the_group() {
    let doc = format!(
        r#"{{"specVersion":"1.0","entries":[
{{"identifier":"{ID}","type":"application/mcp-server-card+json","url":"https://example.com/0.json","version":"1.0.0"}},
{{"identifier":"{ID}x","type":"application/mcp-server-card+json","url":"https://example.com/1.json","version":"9.0.0"}}]}}"#
    );
    let c = ok(&doc);
    assert_eq!(pick(&c), Some(0));
}

// #multi-version-entries: only strict SemVer 2.0.0 versions are ordered by version
#[test]
fn non_semver_versions_fall_back() {
    for bad in ["v1.0.0", "01.0.0", "1.0", "1.0.0-", "1.0.0+"] {
        // 9.0.0 would win by SemVer; the bad one has the later updatedAt
        let c = group(&[
            (Some("9.0.0"), Some("2026-01-01T00:00:00Z")),
            (Some(bad), Some("2026-06-01T00:00:00Z")),
        ]);
        assert_eq!(pick(&c), Some(1), "{bad}");
    }
}

// #multi-version-entries: numeric identifiers of any length compare without overflow
#[test]
fn semver_numeric_identifiers_of_any_length_compare() {
    let big = "9".repeat(40);
    let bigger = format!("1{}", "0".repeat(40));
    let (a, b) = (format!("{big}.0.0"), format!("{bigger}.0.0"));
    assert_eq!(pick(&group(&versions(&[&a, &b]))), Some(1));
    let (a, b) = (format!("1.0.0-{bigger}"), format!("1.0.0-{big}"));
    assert_eq!(pick(&group(&versions(&[&a, &b]))), Some(0));
}
