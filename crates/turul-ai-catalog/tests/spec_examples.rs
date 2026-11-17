//! Every example of the specification, fed to `parse` byte for byte.
//! Fragments are wrapped by byte concatenation: prefix ++ fragment bytes ++ suffix.

mod common;

use std::collections::BTreeSet;
use std::fs;

use common::spec_corpus::{Rule::*, SPEC_FIXTURES as FIXTURES, bytes_of, wrapped};

use turul_ai_catalog::canonical::to_canonical_vec;
use turul_ai_catalog::identity::ArtifactType;
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::{AiCatalog, ArtifactSource};
use turul_ai_catalog::parse::{ParseErrorKind, parse};

fn parsed(name: &str) -> AiCatalog {
    match parse(&wrapped(name)) {
        Ok(c) => c,
        Err(e) => panic!("{name}: {e} at {}", e.pointer),
    }
}

fn manifest_of(c: &AiCatalog) -> &turul_ai_catalog::model::TrustManifest {
    &c.entries[0].trust_manifests.as_ref().expect("manifests")[0]
}

fn listed_files() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for dir in ["examples", "blocks"] {
        for f in fs::read_dir(common::spec_corpus::fixture_dir().join(dir)).unwrap() {
            out.insert(format!(
                "{dir}/{}",
                f.unwrap().file_name().to_string_lossy()
            ));
        }
    }
    out
}

// #conformance-levels, corpus: each fixture has exactly one class; only block 09 is exempt
#[test]
fn every_spec_fixture_is_classified() {
    let names: Vec<&str> = FIXTURES.iter().map(|(n, _)| *n).collect();
    let unique: BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(unique.len(), names.len(), "a fixture is classified twice");
    let on_disk = listed_files();
    let classified: BTreeSet<String> = names.iter().map(|n| n.to_string()).collect();
    assert_eq!(
        on_disk, classified,
        "fixtures on disk and classified fixtures differ"
    );
    assert_eq!(
        on_disk.iter().filter(|n| n.starts_with("blocks/")).count(),
        18
    );
    let exempt: Vec<&str> = FIXTURES
        .iter()
        .filter(|(_, r)| *r == Exempt)
        .map(|(n, _)| *n)
        .collect();
    assert_eq!(
        exempt,
        ["blocks/09-signed-payload-and-jws-construction.json"]
    );
    // Every block ends with exactly one newline, and no fixture has a carriage return.
    for n in &names {
        let b = bytes_of(n);
        assert!(!b.contains(&b'\r'), "{n}");
        if n.starts_with("blocks/") {
            assert!(b.ends_with(b"}\n") && !b.ends_with(b"\n\n"), "{n}");
        }
    }
}

// Examples that are whole documents
#[test]
fn full_document_examples_parse() {
    for (name, rule) in FIXTURES {
        if *rule == Doc && *name != "blocks/12-format-and-key-naming.json" {
            let c = parsed(name);
            assert_eq!(c.spec_version, "1.0", "{name}");
        }
    }
}

// Examples that are fragments, wrapped by their rule
#[test]
fn fragment_examples_parse_when_wrapped() {
    for (name, rule) in FIXTURES {
        if !matches!(rule, Doc | Exempt) {
            parsed(name);
        }
    }

    // #host-info
    let host = parsed("blocks/02-host-info.json").host.unwrap();
    assert_eq!(host.display_name, "Acme Enterprise AI");

    // #optional-members: two manifests
    let c = parsed("blocks/05-optional-members.json");
    assert_eq!(c.entries[0].trust_manifests.as_ref().unwrap().len(), 2);
    assert_eq!(c.entries.len(), 1);

    // #trust-schema-object
    assert!(
        manifest_of(&parsed("blocks/06-trust-schema-object.json"))
            .trust_schema
            .is_some()
    );

    // #attestation-object
    let c = parsed("blocks/07-attestation-object.json");
    assert_eq!(
        manifest_of(&c).attestations.as_ref().unwrap()[0].size,
        Some(245760)
    );

    // #provenance-link-object
    let c = parsed("blocks/08-provenance-link-object.json");
    assert_eq!(manifest_of(&c).provenance.as_ref().unwrap().len(), 1);

    // #policy-urls-extension: the value is kept
    let c = parsed("blocks/13-policy-urls-extension.json");
    let ext = c.entries[0].extensions.as_ref().expect("extensions");
    let policy = ext
        .get("https://ai-catalog.org/extensions/policy-urls")
        .expect("policy-urls kept");
    match policy {
        Json::Object(m) => {
            assert!(m.get("privacyPolicyUrl").is_some());
            assert!(m.get("termsOfServiceUrl").is_some());
        }
        other => panic!("{other:?}"),
    }

    // #unrecognized-members, "Examples": unknown members kept
    let c = parsed("blocks/14-unrecognized-members-examples.json");
    for k in ["costCenter", "approvedBy", "reviewDate"] {
        assert!(c.entries[0].unrecognized.get(k).is_some(), "{k}");
    }
    let c = parsed("blocks/15-unrecognized-members-examples.json");
    for k in ["lifecycle", "supersededBy"] {
        assert!(c.entries[0].unrecognized.get(k).is_some(), "{k}");
    }
}

// #format-and-key-naming: the example's only entry has neither `url` nor `data`, and
// #catalog-entry says "MUST contain exactly one of", so the example is refused.
#[test]
fn format_and_key_naming_example_lacks_an_artifact_source() {
    let e = parse(&wrapped("blocks/12-format-and-key-naming.json")).unwrap_err();
    assert!(
        matches!(e.kind, ParseErrorKind::ArtifactSourceMissing),
        "{:?}",
        e.kind
    );
    assert_eq!(e.pointer.to_string(), "/entries/0");
}

// #example-hierarchical-catalog: "A catalog MAY contain both direct artifact entries and nested catalog entries"
#[test]
fn hierarchical_example_mixes_direct_and_nested_entries() {
    let c = parsed("blocks/17-example-hierarchical-catalog.json");
    let nested: Vec<_> = c
        .entries
        .iter()
        .filter(|e| e.artifact_type == ArtifactType::AI_CATALOG)
        .collect();
    let direct: Vec<_> = c
        .entries
        .iter()
        .filter(|e| e.artifact_type != ArtifactType::AI_CATALOG)
        .collect();
    assert_eq!(direct.len(), 1);
    assert_eq!(nested.len(), 3);
    assert!(
        nested
            .iter()
            .all(|e| matches!(e.source, ArtifactSource::Url(_)))
    );
    assert_eq!(
        direct[0].identifier.as_str(),
        "urn:air:acme.com:agent:assistant"
    );
}

// Canonical writer, law 1: every parsed spec example, wrapped as for parse, round-trips.
// Block 12 is refused at parse and block 09 is a payload vector, so neither is parsed here.
#[test]
fn every_parsed_spec_example_round_trips() {
    let mut round_tripped = 0;
    for (name, rule) in FIXTURES {
        if *rule == Exempt || *name == "blocks/12-format-and-key-naming.json" {
            continue;
        }
        let m = parsed(name);
        let c = to_canonical_vec(&m).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let back = parse(&c).unwrap_or_else(|e| panic!("{name}: {e} at {}", e.pointer));
        assert_eq!(back, m, "{name}");
        round_tripped += 1;
    }
    assert_eq!(round_tripped, FIXTURES.len() - 2);
}
