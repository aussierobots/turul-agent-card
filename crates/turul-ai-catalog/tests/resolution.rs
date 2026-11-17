mod common;

use common::*;
use turul_ai_catalog::model::CatalogEntry;
use turul_ai_catalog::resolve::{description, display_name, version};

fn entry(id: &str, extra: &str) -> CatalogEntry {
    let doc = format!(
        r#"{{"specVersion":"1.0","entries":[{{"identifier":"{id}","type":"application/mcp-server-card+json","url":"https://example.com/a.json"{extra}}}]}}"#
    );
    ok(&doc).entries.into_iter().next().unwrap()
}

// #resolving-an-artifact-s-display-name: entry displayName, artifact name, identifier segment
#[test]
fn display_name_resolves_in_spec_order() {
    let named = entry(
        "urn:air:example.com:mcp:weather",
        r#","displayName":"Entry""#,
    );
    assert_eq!(display_name(&named, Some("Artifact")), "Entry");
    let plain = entry("urn:air:example.com:mcp:weather", "");
    assert_eq!(display_name(&plain, Some("Artifact")), "Artifact");
    assert_eq!(display_name(&plain, None), "weather");
    let brave = entry(
        "urn:air:anonymous.modelcontextprotocol.io:mcp:brave-search",
        "",
    );
    assert_eq!(display_name(&brave, None), "brave-search");
}

// #resolving-an-artifact-s-description: entry description, artifact description
#[test]
fn description_resolves_in_spec_order() {
    let with = entry("urn:air:example.com:mcp:a", r#","description":"Entry""#);
    assert_eq!(description(&with, Some("Artifact")), Some("Entry"));
    let without = entry("urn:air:example.com:mcp:a", "");
    assert_eq!(description(&without, Some("Artifact")), Some("Artifact"));
}

// #resolving-an-artifact-s-description: no identifier-derived fallback
#[test]
fn missing_description_stays_none() {
    let e = entry("urn:air:example.com:mcp:a", "");
    assert_eq!(description(&e, None), None);
}

// #resolving-an-artifact-s-version: entry version, artifact version
#[test]
fn version_resolves_in_spec_order() {
    let with = entry("urn:air:example.com:mcp:a", r#","version":"1.0.0""#);
    assert_eq!(version(&with, Some("2.0.0")), Some("1.0.0"));
    let without = entry("urn:air:example.com:mcp:a", "");
    assert_eq!(version(&without, Some("2.0.0")), Some("2.0.0"));
    assert_eq!(version(&without, None), None);
}

// #resolving-an-artifact-s-version: a mismatch is never resolved by preferring the artifact
#[test]
fn entry_version_wins_over_a_differing_artifact_version() {
    let e = entry("urn:air:example.com:mcp:a", r#","version":"1.0.0""#);
    assert_eq!(version(&e, Some("9.9.9")), Some("1.0.0"));
}

// #catalog-entry: displayName is rendered as given even when it differs
#[test]
fn entry_display_name_wins_over_a_differing_artifact_name() {
    let e = entry("urn:air:example.com:mcp:a", r#","displayName":"Mine""#);
    assert_eq!(display_name(&e, Some("Theirs")), "Mine");
}

// #catalog-entry: description is rendered as given even when it differs
#[test]
fn entry_description_wins_over_a_differing_artifact_description() {
    let e = entry("urn:air:example.com:mcp:a", r#","description":"Mine""#);
    assert_eq!(description(&e, Some("Theirs")), Some("Mine"));
}

// #resolving-an-artifact-s-display-name: step 2 is a value the caller supplies, nothing is fetched
#[test]
fn artifact_values_come_only_from_the_caller() {
    let e = entry("urn:air:example.com:mcp:a", "");
    assert_eq!(display_name(&e, None), "a");
    assert_eq!(description(&e, None), None);
    assert_eq!(version(&e, None), None);
    assert_eq!(display_name(&e, Some("Supplied")), "Supplied");
}

// #resolving-an-artifact-s-display-name: identifier segment after the last `:` or `/`
#[test]
fn identifier_segment_fallback() {
    for (id, want) in [("a:b/c", "c"), ("plain", "plain"), ("x:", "x:"), ("", "")] {
        let e = entry(id, "");
        assert_eq!(display_name(&e, None), want, "identifier {id:?}");
    }
    let slash = entry("https://example.com/agents/x/", "");
    assert_eq!(display_name(&slash, None), "https://example.com/agents/x/");
}

// #catalog-entry: a present member is authoritative whatever its text
#[test]
fn empty_present_values_win() {
    let e = entry(
        "urn:air:example.com:mcp:a",
        r#","displayName":"","description":"","version":"""#,
    );
    assert_eq!(display_name(&e, Some("Artifact")), "");
    assert_eq!(description(&e, Some("Artifact")), Some(""));
    assert_eq!(version(&e, Some("1.0.0")), Some(""));
    let plain = entry("urn:air:example.com:mcp:a", "");
    assert_eq!(display_name(&plain, Some("")), "");
    assert_eq!(description(&plain, Some("")), Some(""));
    assert_eq!(version(&plain, Some("")), Some(""));
}
