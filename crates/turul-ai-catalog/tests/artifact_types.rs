mod common;

use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use turul_ai_catalog::identity::{ArtifactType, MEDIA_TYPE, WELL_KNOWN_PATH};

// #catalog-entry `type`: the recognised "known types", in spec order
#[test]
fn known_type_consts_match_the_spec_list() {
    let expected = [
        (ArtifactType::AI_CATALOG, "application/ai-catalog+json"),
        (ArtifactType::AGENT_CARD, "application/agent-card+json"),
        (
            ArtifactType::A2A_AGENT_CARD,
            "application/a2a-agent-card+json",
        ),
        (
            ArtifactType::MCP_SERVER_CARD,
            "application/mcp-server-card+json",
        ),
        (
            ArtifactType::AGENT_SKILLS_JSON,
            "application/agent-skills+json",
        ),
        (ArtifactType::AGENT_SKILLS_MD, "application/agent-skills+md"),
        (
            ArtifactType::AGENT_SKILLS_ZIP,
            "application/agent-skills+zip",
        ),
        (
            ArtifactType::AGENT_SKILLS_GZIP,
            "application/agent-skills+gzip",
        ),
        (
            ArtifactType::AGENT_PLUGINS_ZIP,
            "application/agent-plugins+zip",
        ),
        (
            ArtifactType::AGENT_PLUGINS_GZIP,
            "application/agent-plugins+gzip",
        ),
    ];
    for (c, text) in &expected {
        assert_eq!(c.as_str(), *text);
        assert_eq!(*c, ArtifactType::new(*text));
    }
    for (i, (a, _)) in expected.iter().enumerate() {
        for (b, _) in &expected[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

// #catalog-entry `type`: "open text format, so any string value is accepted"
#[test]
fn any_string_is_an_artifact_type() {
    let doc_types = [
        "application/parquet",
        "SOC2-Type2",
        "x-vendor/thing",
        "application/agent-card+json",
        "application/agent-skills+md",
        "",
        " ",
        "no-slash",
        "Ünï\u{1F600}",
    ];
    for t in doc_types {
        assert_eq!(ArtifactType::new(t).as_str(), t);
        let cat = ok(&format!(
            r#"{{"specVersion":"1.0","entries":[{{"identifier":"i","type":"{t}","url":"u"}}]}}"#
        ));
        assert_eq!(cat.entries[0].artifact_type.as_str(), t);
    }
    assert_eq!(ArtifactType::new(String::from("a/b")).as_str(), "a/b");
}

// #catalog-entry `type`: type values are exact text, not case-folded media types
#[test]
fn type_comparison_is_exact() {
    assert_ne!(
        ArtifactType::new("Application/AI-Catalog+JSON"),
        ArtifactType::AI_CATALOG
    );
    assert_ne!(
        ArtifactType::new("application/ai-catalog+json "),
        ArtifactType::AI_CATALOG
    );
    assert_ne!(
        ArtifactType::new("application/ai-catalog+json; charset=utf-8"),
        ArtifactType::AI_CATALOG
    );
    assert_eq!(
        ArtifactType::new("application/ai-catalog+json"),
        ArtifactType::AI_CATALOG
    );
}

// #design-goals goal 2: entries are selected by `type` alone, without reading the artifact
#[test]
fn entries_are_selected_by_type_without_reading_the_artifact() {
    let bytes = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/spec/examples/ai-catalog.json"),
    )
    .unwrap();
    let cat = turul_ai_catalog::parse::parse(&bytes).expect("spec example parses");
    let a2a: Vec<_> = cat
        .entries
        .iter()
        .filter(|e| e.artifact_type == ArtifactType::A2A_AGENT_CARD)
        .map(|e| e.identifier.as_str())
        .collect();
    assert_eq!(a2a, ["urn:air:acme-corp.com:agent:finance"]);
    let plugins = cat
        .entries
        .iter()
        .filter(|e| e.artifact_type == ArtifactType::AGENT_PLUGINS_ZIP)
        .count();
    assert_eq!(plugins, 1);
    // An unknown type is selectable too.
    assert_eq!(
        cat.entries
            .iter()
            .filter(|e| e.artifact_type == ArtifactType::new("application/parquet"))
            .count(),
        1
    );
}

// #media-type, #well-known-uri
#[test]
fn media_type_and_well_known_path_match_the_spec() {
    assert_eq!(MEDIA_TYPE, "application/ai-catalog+json");
    assert_eq!(ArtifactType::AI_CATALOG.as_str(), MEDIA_TYPE);
    assert_eq!(WELL_KNOWN_PATH, "/.well-known/ai-catalog.json");
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rs_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

// #media-type: the literal is defined once in the workspace's sources
#[test]
fn media_type_literal_has_one_definition() {
    let crates = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for c in fs::read_dir(&crates).unwrap() {
        let src = c.unwrap().path().join("src");
        if src.is_dir() {
            rs_files(&src, &mut files);
        }
    }
    let needle = "\"application/ai-catalog+json\"";
    let hits: Vec<_> = files
        .iter()
        .flat_map(|f| {
            let n = fs::read_to_string(f).unwrap().matches(needle).count();
            std::iter::repeat_n(f.display().to_string(), n)
        })
        .collect();
    assert_eq!(hits.len(), 1, "literal found in: {hits:?}");
}

// #catalog-entry `type`: an empty type names nothing
#[test]
fn empty_type_is_type_empty() {
    let r = report(&doc_with_entry_body(
        r#""identifier":"i","type":"","url":"u""#,
    ));
    assert_only(
        &r,
        "type-empty",
        turul_ai_catalog::validate::Severity::Error,
        "/entries/0/type",
    );
}

// #catalog-entry `type`: "open text format, so any string value is accepted"
#[test]
fn unknown_and_reserved_types_give_no_diagnostic() {
    for t in [
        "application/parquet",
        "SOC2-Type2",
        "x-vendor/thing",
        "no-slash",
        " ",
        "application/agent-card+json",
        "application/agent-skills+md",
        "application/agent-plugins+zip",
        "application/ai-catalog+json",
        "Application/AI-Catalog+JSON",
    ] {
        assert_clean(&report(&doc_with_entry_body(&format!(
            r#""identifier":"i","type":"{t}","url":"u""#
        ))));
    }
}
