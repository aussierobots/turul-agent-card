//! The signed payload of a manifest and of a catalog snapshot.

mod support;

use support::{catalog, fixture, manifest, spec_golden};
use turul_ai_catalog::json::Json;
use turul_ai_catalog_trust::{PayloadError, catalog_payload, manifest_payload};

const SPEC_BLOCK: &str = "../../../turul-ai-catalog/tests/fixtures/spec/blocks/09-signed-payload-and-jws-construction.json";

fn text(b: Vec<u8>) -> String {
    String::from_utf8(b).unwrap()
}

// signed-payload-and-jws-construction-1
#[test]
fn spec_example_manifest_payload_equals_golden() {
    let block = fixture(SPEC_BLOCK);
    let with_jws = std::str::from_utf8(&block)
        .unwrap()
        .replace("\"signer\"", "\"jws\": \"aGVhZGVy..c2ln\", \"signer\"");
    let m = manifest(&with_jws);
    assert_eq!(manifest_payload(&m).unwrap(), spec_golden());
}

// signed-payload-and-jws-construction-1
#[test]
fn every_signature_member_but_jws_is_retained() {
    let m = manifest(
        r#"{"contributor":"did:web:acme.com","signature":{"signer":"did:web:acme.com","profile":"did-web-v1","issuedAt":"2026-09-12T10:00:00Z","expiresAt":"2027-09-12T10:00:00Z","x-sig":1,"jws":"a..b"}}"#,
    );
    let p = text(manifest_payload(&m).unwrap());
    for kept in [
        "\"expiresAt\":\"2027-09-12T10:00:00Z\"",
        "\"x-sig\":1",
        "\"signer\":\"did:web:acme.com\"",
        "\"profile\":\"did-web-v1\"",
        "\"issuedAt\"",
    ] {
        assert!(p.contains(kept), "{kept} in {p}");
    }
    assert!(!p.contains("\"jws\""), "{p}");
}

// signed-payload-and-jws-construction-1
#[test]
fn reserved_names_below_the_top_are_retained() {
    let m = manifest(&text(fixture(
        "signed_payload/manifest-reserved-names.input.json",
    )));
    assert_eq!(
        manifest_payload(&m).unwrap(),
        fixture("signed_payload/manifest-reserved-names.jcs")
    );
}

// signed-payload-and-jws-construction-2, consumer-requirements-6
#[test]
fn unknown_members_at_every_depth_are_retained() {
    let m = manifest(
        r#"{"contributor":"did:web:acme.com","x-m":1,
        "subject":{"identifier":"urn:air:example.com:fixture:a","type":"application/json","digest":"sha256:56bbd2ff730d81a52951fc2f58809dfb086b4be7a82766dcc18cfe32ff6acbbd","x-subject":2},
        "signature":{"signer":"did:web:acme.com","profile":"did-web-v1","issuedAt":"2026-09-12T10:00:00Z","jws":"a..b","x-signature":3},
        "attestations":[{"type":"t","uri":"https://example.com/a","x-attestation":4}],
        "provenance":[{"relation":"publishedFrom","sourceId":"s","x-link":5}],
        "trustSchema":{"identifier":"urn:trust:x","version":"1","governanceUri":"https://example.com/g","verificationMethods":["did"],"x-schema":6}}"#,
    );
    let p = text(manifest_payload(&m).unwrap());
    for (name, v) in [
        ("x-m", 1),
        ("x-subject", 2),
        ("x-signature", 3),
        ("x-attestation", 4),
        ("x-link", 5),
        ("x-schema", 6),
    ] {
        assert!(p.contains(&format!("\"{name}\":{v}")), "{name} in {p}");
    }
}

// #catalog-snapshot-coverage
#[test]
fn catalog_payload_keeps_nested_manifests_with_jws() {
    let c = catalog(&text(fixture("signed_payload/catalog-snapshot.input.json")));
    let p = catalog_payload(&c).unwrap();
    assert_eq!(p, fixture("signed_payload/catalog-snapshot.jcs"));
    assert!(text(p).contains("\"jws\":\"aGVhZGVy..c2ln\""));
}

// #catalog-snapshot-coverage
#[test]
fn root_additional_signatures_do_not_change_the_catalog_payload() {
    let mut c = catalog(&text(fixture("signed_payload/catalog-snapshot.input.json")));
    let before = catalog_payload(&c).unwrap();
    c.additional_signatures = Some(Json::Array(vec![]));
    assert_eq!(catalog_payload(&c).unwrap(), before);
    c.additional_signatures = None;
    assert_eq!(catalog_payload(&c).unwrap(), before);
}

#[test]
fn non_i_json_number_is_refused_with_its_pointer() {
    let m = manifest(
        r#"{"contributor":"did:web:acme.com","extensions":{"https://example.com/x":1e999}}"#,
    );
    match manifest_payload(&m).unwrap_err() {
        PayloadError::NotIJson { pointer } => {
            assert_eq!(pointer.to_string(), "/extensions/https:~1~1example.com~1x");
        }
        other => panic!("{other:?}"),
    }
}
