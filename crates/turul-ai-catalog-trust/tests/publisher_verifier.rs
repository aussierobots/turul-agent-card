//! The publisher verdict: which signed entries a registrar may accept.
#![cfg(feature = "registration")]

mod support;

use support::world::{self, at, data_entry, signed, signed_at, with_manifests};
use turul_ai_catalog::model::CatalogEntry;
use turul_ai_catalog_storage::PublisherVerifier;
use turul_ai_catalog_trust::{
    DidWebPublisherVerifier, TrustPolicy, compute, publisher_verdict, verify_entry,
    verify_entry_resolved,
};

const BILLING: &str = "urn:air:example.com:agent:billing";

fn verdict(
    entry: &CatalogEntry,
    signers: &[&str],
    offset: i64,
) -> Result<(), turul_ai_catalog_storage::VerifierRefusal> {
    let docs = world::docs(signers);
    publisher_verdict(&verify_entry_resolved(
        entry,
        None,
        &docs,
        &world::policy(),
        at(offset),
    ))
}

fn signed_entry(identifier: &str, signer: &str) -> CatalogEntry {
    let e = data_entry(identifier, r#"{"a":1}"#);
    let m = signed(&e, signer);
    with_manifests(e, vec![m])
}

#[test]
fn publisher_signature_is_accepted() {
    let e = signed_entry(BILLING, "did:web:example.com");
    assert!(verdict(&e, &["did:web:example.com"], 0).is_ok());
}

#[test]
fn subdomain_or_suffix_signer_is_refused() {
    for (id, signer) in [
        (BILLING, "did:web:sub.example.com"),
        (BILLING, "did:web:example.com.evil"),
        (BILLING, "did:web:evilexample.com"),
        (
            "urn:air:sub.example.com:agent:billing",
            "did:web:example.com",
        ),
    ] {
        let e = signed_entry(id, signer);
        assert!(verdict(&e, &[signer], 0).is_err(), "{id} by {signer}");
    }
}

#[test]
fn assessor_signature_is_refused() {
    let e = signed_entry(BILLING, "did:web:assessor.example");
    let refusal = verdict(&e, &["did:web:assessor.example"], 0).unwrap_err();
    assert!(refusal.0.contains("manifest 0"), "{refusal}");
}

#[test]
fn ip_spelled_publisher_is_refused() {
    for publisher in ["127.0.0.1", "2130706433", "0x7f.1"] {
        let id = format!("urn:air:{publisher}:agent:billing");
        let e = signed_entry(&id, "did:web:example.com");
        assert!(verdict(&e, &["did:web:example.com"], 0).is_err(), "{id}");
    }
}

#[test]
fn digest_mismatch_is_refused() {
    let mut e = data_entry(BILLING, r#"{"a":1}"#);
    e.digest = Some(compute("sha256", b"other bytes").unwrap());
    let m = signed(&e, "did:web:example.com");
    let e = with_manifests(e, vec![m]);
    assert!(verdict(&e, &["did:web:example.com"], 0).is_err());
}

#[test]
fn entry_without_digest_is_refused() {
    // The signer refuses to sign a digest-less entry, so the digest is dropped after signing.
    let mut e = data_entry(BILLING, r#"{"a":1}"#);
    let m = signed(&e, "did:web:example.com");
    e.digest = None;
    let e = with_manifests(e, vec![m]);
    assert!(verdict(&e, &["did:web:example.com"], 0).is_err());
}

#[test]
fn expired_publisher_signature_is_refused() {
    let e = data_entry(BILLING, r#"{"a":1}"#);
    let m = signed_at(
        &e,
        "did:web:example.com",
        world::ISSUED,
        Some("2026-09-12T11:00:00Z"),
    );
    let e = with_manifests(e, vec![m]);
    assert!(verdict(&e, &["did:web:example.com"], 3599).is_ok());
    assert!(verdict(&e, &["did:web:example.com"], 3600).is_err());
}

#[test]
fn entry_without_a_manifest_is_refused_without_resolving_anything() {
    let e = data_entry(BILLING, r#"{"a":1}"#);
    let report = verify_entry(&e, None, &TrustPolicy::default(), at(0));
    assert!(report.manifests.is_empty());
    let refusal = publisher_verdict(&report).unwrap_err();
    assert!(refusal.0.contains("no trust manifest"), "{refusal}");
}

#[tokio::test]
async fn verifier_refuses_an_unsigned_entry() {
    let verifier = DidWebPublisherVerifier::new(TrustPolicy::default());
    let e = data_entry(BILLING, r#"{"a":1}"#);
    assert!(verifier.verify_publisher(&e, at(0)).await.is_err());
}
