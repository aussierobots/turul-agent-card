mod common;

use Conformance::{Met, NotMet};
use ConformanceLevel::{Discoverable, Minimal};
use common::*;
use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;
use turul_ai_catalog::validate::{Code, Conformance, ConformanceLevel, Severity};

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/spec")
            .join(name),
    )
    .unwrap_or_else(|e| panic!("{name}: {e}"))
}

// #conformance-levels: a document is judged by the level it meets; every valid spec example, with its outcome
#[test]
fn valid_corpus_documents_meet_their_recorded_level() {
    let documents: &[(&str, Conformance)] = &[
        // Level 2: `host` present.
        ("examples/ai-catalog.json", Met(Discoverable)),
        ("blocks/10-nested-catalog-entries.json", Met(Discoverable)),
        (
            "blocks/17-example-hierarchical-catalog.json",
            Met(Discoverable),
        ),
        // Level 1: no `host`.
        ("blocks/01-top-level-structure.json", Met(Minimal)),
        ("blocks/03-multi-version-entries.json", Met(Minimal)),
    ];
    for (name, want) in documents {
        let c = turul_ai_catalog::parse::parse(&fixture(name))
            .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let r = turul_ai_catalog::validate::validate(&c);
        assert_eq!(found(&r), vec![], "{name}");
        assert_eq!(r.conformance(), *want, "{name}");
    }
    // Entry fragments wrapped as one-entry catalogs.
    for name in [
        "blocks/11-nested-catalog-entries.json",
        "blocks/18-example-dual-protocol-agent-mcp-a2a.json",
    ] {
        let frag = String::from_utf8(fixture(name)).unwrap();
        let r = report(&doc(&[frag.trim()]));
        assert_eq!(found(&r), vec![], "{name}");
        assert_eq!(r.conformance(), Met(Minimal), "{name}");
    }
}

// #digest-format: "Digests in this specification use the format `algorithm:hex-value`"; the example's
// truncated `sourceDigest` placeholder "sha256:99998888..." is not one, so the document meets no level
#[test]
fn multi_artifact_example_has_placeholder_source_digest() {
    let name = "blocks/16-example-multi-artifact-catalog-with-nested-catalog.json";
    let c =
        turul_ai_catalog::parse::parse(&fixture(name)).unwrap_or_else(|e| panic!("{name}: {e:?}"));
    let r = turul_ai_catalog::validate::validate(&c);
    assert_only(
        &r,
        "digest-invalid",
        Severity::Error,
        "/entries/2/data/entries/2/trustManifests/0/provenance/0/sourceDigest",
    );
    assert_eq!(r.conformance(), NotMet);
}

// #level-1-minimal-catalog: "All other defined members are OPTIONAL"
#[test]
fn minimal_document_is_met_minimal() {
    let r = report(r#"{"specVersion":"1.0","entries":[]}"#);
    assert_clean(&r);
    assert_eq!(r.conformance(), Met(Minimal));
    assert!(!r.has_errors());
    assert!(Minimal < Discoverable);
}

// #level-2-discoverable-catalog: "Includes a `host` object"
#[test]
fn document_with_host_is_met_discoverable() {
    let r = report(r#"{"specVersion":"1.0","host":{"displayName":"h"},"entries":[]}"#);
    assert_clean(&r);
    assert_eq!(r.conformance(), Met(Discoverable));
}

// #level-1-minimal-catalog: a document with a defect is not a conformant catalog of any level
#[test]
fn any_error_is_not_met() {
    for text in [
        doc(&[&entry("", "")]),
        format!(
            r#"{{"specVersion":"1.0","host":{{"displayName":"h"}},"entries":[{}]}}"#,
            entry("a", r#","updatedAt":"x""#)
        ),
    ] {
        let r = report(&text);
        assert!(r.has_errors());
        assert_eq!(r.conformance(), NotMet);
    }
}

// #conformance-levels: warnings never change the outcome
#[test]
fn warnings_do_not_change_conformance() {
    let r = report(
        r#"{"specVersion":"01.0","host":{"displayName":"h","logoUrl":"https://x/y"},"entries":[]}"#,
    );
    assert_eq!(r.diagnostics().len(), 2);
    assert!(
        r.diagnostics()
            .iter()
            .all(|d| d.severity == Severity::Warning)
    );
    assert!(!r.has_errors());
    assert_eq!(r.conformance(), Met(Discoverable));
    let r = report(r#"{"specVersion":"1.00","entries":[]}"#);
    assert_eq!(r.conformance(), Met(Minimal));
}

// #level-3-trusted-catalog: consumers "SHOULD gracefully handle their absence"
#[test]
fn absent_trust_fields_give_no_diagnostic() {
    let r = report(&doc(&[&entry("a", ""), &entry("b", r#","version":"1""#)]));
    assert_clean(&r);
    assert_eq!(r.conformance(), Met(Minimal));
}

// Reports are stable: the same document gives the same diagnostics in one fixed order
#[test]
fn report_is_deterministic() {
    let text = format!(
        r#"{{"specVersion":"01.0","entries":[{},{},{}],"big":9007199254740993}}"#,
        entry("", ""),
        entry("b", r#","updatedAt":"x""#),
        entry("b", ""),
    );
    let first = found(&report(&text));
    assert_eq!(first, found(&report(&text)));
    let paths: Vec<_> = first
        .iter()
        .map(|(c, _, p)| (c.as_str(), p.as_str()))
        .collect();
    assert_eq!(
        paths,
        [
            ("spec-version-noncanonical", "/specVersion"),
            ("identifier-empty", "/entries/0/identifier"),
            ("timestamp-invalid", "/entries/1/updatedAt"),
            ("entry-duplicate", "/entries/2"),
            ("number-inexact", "/big"),
        ]
    );
}

// The code names are the contract of the CLI's JSON output: each is listed once
#[test]
fn every_code_name_is_listed_once() {
    let listed = [
        (Code::SpecVersionNoncanonical, "spec-version-noncanonical"),
        (Code::IdentifierEmpty, "identifier-empty"),
        (Code::TypeEmpty, "type-empty"),
        (Code::UrlEmpty, "url-empty"),
        (Code::EntryDuplicate, "entry-duplicate"),
        (Code::VersionMissingInGroup, "version-missing-in-group"),
        (Code::TimestampInvalid, "timestamp-invalid"),
        (Code::DigestInvalid, "digest-invalid"),
        (Code::DigestWeak, "digest-weak"),
        (Code::DigestAlgorithmUnknown, "digest-algorithm-unknown"),
        (Code::ExtensionKeyInvalid, "extension-key-invalid"),
        (Code::PolicyUrlsInvalid, "policy-urls-invalid"),
        (Code::LogoUrlNotDataUri, "logo-url-not-data-uri"),
        (Code::NestedCatalogInvalid, "nested-catalog-invalid"),
        (Code::NestedDepthLimit, "nested-depth-limit"),
        (Code::NumberNotIJson, "number-not-i-json"),
        (Code::NumberInexact, "number-inexact"),
        (Code::StringNotIJson, "string-not-i-json"),
    ];
    assert_eq!(listed.len(), 18);
    let mut names = BTreeSet::new();
    for (code, name) in listed {
        assert_eq!(code.as_str(), name);
        assert_eq!(code.to_string(), name);
        assert!(names.insert(name), "{name} listed twice");
    }
}
