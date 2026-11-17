//! RFC 8785 canonical bytes, the round-trip laws and the model mapping.

mod common;

use std::fs;
use std::path::PathBuf;

use common::*;
use turul_ai_catalog::canonical::to_canonical_vec;
use turul_ai_catalog::json::{Json, read};
use turul_ai_catalog::parse::parse;

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn golden(name: &str) -> Vec<u8> {
    let p = tests_dir().join("fixtures/canonical").join(name);
    fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn json_of(bytes: &[u8]) -> Json {
    read(bytes).unwrap_or_else(|e| panic!("{e} at {}", e.pointer))
}

fn canon_of_golden_input(name: &str) -> Vec<u8> {
    json_of(&golden(name)).to_canonical_vec().unwrap()
}

// RFC 8785 section 3.2.3: names ordered by UTF-16 code units, not UTF-8 bytes
#[test]
fn rfc_8785_sorting_example_orders_names_by_utf16_code_units() {
    let expected = golden("rfc8785-sorting.jcs");
    assert_eq!(
        canon_of_golden_input("rfc8785-sorting.input.json"),
        expected
    );
    // The golden's order differs from UTF-8 byte order (U+FB33 sorts before U+1F600 by bytes), so a bytewise sort fails.
    let text = String::from_utf8(expected).unwrap();
    assert!(text.find('\u{1F600}').unwrap() < text.find('\u{FB33}').unwrap());
}

// RFC 8785 section 3.2.4: the sample output is a fixed point
#[test]
fn rfc_8785_sample_output_is_a_fixed_point() {
    let g = golden("rfc8785-sample.jcs");
    assert_eq!(g.len(), 118);
    assert_eq!(json_of(&g).to_canonical_vec().unwrap(), g);
}

// RFC 8785 sections 3.2.2 and 3.2.4: the sample input gives the sample output's bytes
// (literals, string escaping, numbers; `333333333.33333329` is written `333333333.3333333`)
#[test]
fn rfc_8785_sample_input_canonicalises_to_its_output() {
    assert_eq!(
        canon_of_golden_input("rfc8785-sample.input.json"),
        golden("rfc8785-sample.jcs")
    );
}

// RFC 8785 section 3.2.2.2: every control character, short forms, lowercase hex, no other escapes
#[test]
fn control_characters_escape_as_ecmascript() {
    assert_eq!(
        canon_of_golden_input("control-characters.input.json"),
        golden("control-characters.jcs")
    );
}

// RFC 8785 section 3.2.3: "array element order MUST NOT be changed"; #top-level-structure: `entries` is an ordered list
#[test]
fn array_order_is_preserved() {
    assert_eq!(
        json_of(br#"{"a":[3,1,2,"b","a"]}"#)
            .to_canonical_vec()
            .unwrap(),
        br#"{"a":[3,1,2,"b","a"]}"#
    );
    let doc = r#"{"specVersion":"1.0","entries":[
        {"identifier":"z","type":"t","url":"u"},{"identifier":"a","type":"t","url":"u"},{"identifier":"m","type":"t","url":"u"}]}"#;
    let m = ok(doc);
    let bytes = to_canonical_vec(&m).unwrap();
    let ids: Vec<String> = parse(&bytes)
        .unwrap()
        .entries
        .iter()
        .map(|e| e.identifier.as_str().to_owned())
        .collect();
    assert_eq!(ids, ["z", "a", "m"]);
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.find(r#""z""#).unwrap() < text.find(r#""a""#).unwrap());
}

// #contributor-identity: `contributor` "MUST be preserved exactly when forwarding"
#[test]
fn contributor_is_preserved_exactly() {
    let doc = doc_with_manifest(
        &r#"{"contributor":"did:web:Example.COM%3A8443:Path@u0041%2f"}"#.replace("@u", "\\u"),
    );
    let m = ok(&doc);
    let expected = "did:web:Example.COM%3A8443:PathA%2f";
    let contributor = |c: &turul_ai_catalog::model::AiCatalog| {
        c.entries[0].trust_manifests.as_ref().unwrap()[0]
            .contributor
            .to_string()
    };
    assert_eq!(contributor(&m), expected);
    let again = ok_bytes(&to_canonical_vec(&m).unwrap());
    assert_eq!(contributor(&again), expected);
}

fn ok_bytes(b: &[u8]) -> turul_ai_catalog::model::AiCatalog {
    parse(b).unwrap_or_else(|e| panic!("{e} at {}", e.pointer))
}

/// Corpus documents (not fragments) that parse, as bytes.
fn corpus_documents() -> Vec<(String, Vec<u8>)> {
    let spec = tests_dir().join("fixtures/spec");
    let mut out = Vec::new();
    for name in [
        "examples/ai-catalog.json",
        "blocks/01-top-level-structure.json",
        "blocks/03-multi-version-entries.json",
        "blocks/10-nested-catalog-entries.json",
        "blocks/16-example-multi-artifact-catalog-with-nested-catalog.json",
        "blocks/17-example-hierarchical-catalog.json",
    ] {
        out.push((name.to_string(), fs::read(spec.join(name)).unwrap()));
    }
    out.extend(parsed_upstream_documents());
    out
}

fn parsed_upstream_documents() -> Vec<(String, Vec<u8>)> {
    let dir = tests_dir().join("fixtures/upstream");
    let mut out = Vec::new();
    for f in fs::read_dir(&dir).unwrap() {
        let path = f.unwrap().path();
        if path.extension().is_some_and(|e| e == "json") {
            let bytes = fs::read(&path).unwrap();
            if parse(&bytes).is_ok() {
                out.push((
                    path.file_name().unwrap().to_string_lossy().into_owned(),
                    bytes,
                ));
            }
        }
    }
    out
}

// Law 1 over the upstream corpus: parse(to_canonical_vec(m)) == m
#[test]
fn every_parsed_upstream_fixture_round_trips() {
    let docs = parsed_upstream_documents();
    assert!(
        docs.iter().any(|(n, _)| n == "minimal.json"),
        "minimal.json must parse"
    );
    for (name, bytes) in docs {
        let m = ok_bytes(&bytes);
        let c = to_canonical_vec(&m).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_eq!(ok_bytes(&c), m, "{name}");
    }
}

// Law 2: canonicalising twice is byte-identical and deterministic, for every corpus document and every golden
#[test]
fn canonicalising_is_idempotent() {
    for (name, bytes) in corpus_documents() {
        let m = ok_bytes(&bytes);
        let c = to_canonical_vec(&m).unwrap();
        assert_eq!(to_canonical_vec(&m).unwrap(), c, "{name} deterministic");
        assert_eq!(
            to_canonical_vec(&ok_bytes(&c)).unwrap(),
            c,
            "{name} idempotent"
        );
    }
    for f in fs::read_dir(tests_dir().join("fixtures/canonical")).unwrap() {
        let path = f.unwrap().path();
        if path.extension().is_some_and(|e| e == "jcs") {
            let g = fs::read(&path).unwrap();
            assert_eq!(
                json_of(&g).to_canonical_vec().unwrap(),
                g,
                "{}",
                path.display()
            );
        }
    }
}

// Law 3: one path from the model to bytes
#[test]
fn document_bytes_are_the_canonical_bytes_of_to_json() {
    for (name, bytes) in corpus_documents() {
        let m = ok_bytes(&bytes);
        assert_eq!(
            to_canonical_vec(&m).unwrap(),
            m.to_json().to_canonical_vec().unwrap(),
            "{name}"
        );
    }
}

// #signed-payload-and-jws-construction: JCS bytes of the example payload
#[test]
fn spec_signed_payload_example_canonicalises_to_golden() {
    let payload = fs::read(
        tests_dir().join("fixtures/spec/blocks/09-signed-payload-and-jws-construction.json"),
    )
    .unwrap();
    assert_eq!(
        json_of(&payload).to_canonical_vec().unwrap(),
        golden("spec-signed-payload.jcs")
    );
}

// #consumer-requirements "preserve ... unchanged": absent and `[]` stay distinct
#[test]
fn empty_and_absent_optional_arrays_round_trip_distinctly() {
    let empty = doc_with_entry_extra(
        r#""tags":[],"trustManifests":[{"contributor":"c","attestations":[],"provenance":[],"trustSchema":{"identifier":"i","version":"1","verificationMethods":[]}}]"#,
    );
    let absent = doc_with_entry_extra(
        r#""trustManifests":[{"contributor":"c","trustSchema":{"identifier":"i","version":"1"}}]"#,
    );
    let (me, ma) = (ok(&empty), ok(&absent));
    assert_ne!(me, ma);
    let (ce, ca) = (
        to_canonical_vec(&me).unwrap(),
        to_canonical_vec(&ma).unwrap(),
    );
    let (te, ta) = (
        String::from_utf8(ce.clone()).unwrap(),
        String::from_utf8(ca.clone()).unwrap(),
    );
    for name in ["tags", "attestations", "provenance", "verificationMethods"] {
        assert!(te.contains(&format!(r#""{name}":[]"#)), "{name}: {te}");
        assert!(!ta.contains(&format!(r#""{name}""#)), "{name}: {ta}");
    }
    assert_eq!(ok_bytes(&ce), me);
    assert_eq!(ok_bytes(&ca), ma);

    // `trustManifests: []` is distinct from absent too
    let tm = ok(&doc_with_entry_extra(r#""trustManifests":[]"#));
    let c = to_canonical_vec(&tm).unwrap();
    assert!(
        String::from_utf8(c.clone())
            .unwrap()
            .contains(r#""trustManifests":[]"#)
    );
    assert_eq!(ok_bytes(&c), tm);
}

// #consumer-requirements: an unrecognised-members entry named like a defined member never shadows it
#[test]
fn defined_member_names_in_unrecognized_are_never_written() {
    let mut m = ok(&doc_with_entry_extra(r#""version":"1.0.0""#));
    let original = m.clone();
    let evil = || Json::String("injected".to_string());
    for name in ["identifier", "type", "url", "description", "tags", "data"] {
        m.entries[0].unrecognized.insert(name.to_string(), evil());
    }
    m.unrecognized.insert("specVersion".to_string(), evil());
    m.unrecognized.insert("host".to_string(), evil());
    let back = ok_bytes(&to_canonical_vec(&m).unwrap());
    assert_eq!(back, original);
}

// The CDDL `any` type admits `null`: explicit nulls are present and round-trip
#[test]
fn explicit_null_values_round_trip() {
    let doc = r#"{"specVersion":"1.0","additionalSignatures":null,"entries":[
        {"identifier":"i","type":"t","data":null},
        {"identifier":"j","type":"t","url":"u","trustManifests":[{"contributor":"c","additionalSignatures":null}]}]}"#;
    let m = ok(doc);
    let c = to_canonical_vec(&m).unwrap();
    let text = String::from_utf8(c.clone()).unwrap();
    assert!(text.contains(r#""data":null"#), "{text}");
    assert_eq!(
        text.matches(r#""additionalSignatures":null"#).count(),
        2,
        "{text}"
    );
    assert_eq!(ok_bytes(&c), m);
}
