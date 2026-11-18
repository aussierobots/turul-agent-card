//! Endorsing entries and catalogs with the publisher's own manifest.

mod support;

#[path = "../../turul-ai-catalog-client/tests/support/loopback.rs"]
mod loopback;

use loopback::{Reply, Server};
use support::world::{
    self, ISSUED, SIGNER, at, data_entry, docs, entry, key, loopback as fetch_policy, params,
    signed_at, url_entry,
};
use turul_ai_catalog::model::{CatalogEntry, TrustManifest};
use turul_ai_catalog_trust::{
    Endorsement, SignError, endorse_catalog, endorse_entry, verify_resolved,
};

const OTHER: &str = "did:web:other.example";
const LATER: &str = "2026-10-01T10:00:00Z";

fn manifests(entry: &CatalogEntry) -> &[TrustManifest] {
    entry.trust_manifests.as_deref().unwrap_or(&[])
}

fn bytes(catalog: &turul_ai_catalog::model::AiCatalog) -> Vec<u8> {
    catalog.to_json().to_canonical_vec().unwrap()
}

#[test]
fn unchanged_release_keeps_its_manifest() {
    let first = endorse_entry(entry(), &params(SIGNER, ISSUED, None), &key()).unwrap();
    assert_eq!(manifests(&first).len(), 1);
    let again = endorse_entry(first.clone(), &params(SIGNER, LATER, None), &key()).unwrap();
    assert_eq!(again, first);
}

#[test]
fn manifest_under_another_key_id_is_resigned() {
    let first = endorse_entry(entry(), &params(SIGNER, ISSUED, None), &key()).unwrap();
    let mut rotated = params(SIGNER, LATER, None);
    rotated.key_fragment = "key-2".to_string();
    let again = endorse_entry(first, &rotated, &key()).unwrap();
    let [only] = manifests(&again) else {
        panic!("one manifest expected");
    };
    assert_eq!(only.signature.as_ref().unwrap().issued_at, LATER);
}

#[test]
fn expiring_manifest_is_resigned() {
    let expires = "2027-01-01T00:00:00Z";
    let first = endorse_entry(entry(), &params(SIGNER, ISSUED, Some(expires)), &key()).unwrap();
    let again = endorse_entry(first, &params(SIGNER, LATER, None), &key()).unwrap();
    let [only] = manifests(&again) else {
        panic!("one manifest expected");
    };
    assert_eq!(only.signature.as_ref().unwrap().issued_at, LATER);
    assert_eq!(only.signature.as_ref().unwrap().expires_at, None);
}

#[test]
fn changed_digest_replaces_the_signers_manifest() {
    let first = endorse_entry(entry(), &params(SIGNER, ISSUED, None), &key()).unwrap();
    let mut changed = data_entry(world::ID, r#"{"a":2}"#);
    changed.trust_manifests = first.trust_manifests.clone();
    let digest = changed.digest.clone().unwrap();

    let again = endorse_entry(changed, &params(SIGNER, LATER, None), &key()).unwrap();
    let [only] = manifests(&again) else {
        panic!("one manifest expected");
    };
    assert_eq!(only.subject.as_ref().unwrap().digest, digest);
    assert_eq!(only.signature.as_ref().unwrap().issued_at, LATER);
}

#[test]
fn other_contributors_are_untouched() {
    let base = entry();
    let theirs = signed_at(&base, OTHER, ISSUED, None);
    let stale = signed_at(&base, SIGNER, ISSUED, None);
    let mut changed = data_entry(world::ID, r#"{"a":2}"#);
    changed.trust_manifests = Some(vec![theirs.clone(), stale]);

    let again = endorse_entry(changed, &params(SIGNER, LATER, None), &key()).unwrap();
    let list = manifests(&again);
    assert_eq!(list.len(), 2);
    assert_eq!(list[0], theirs);
    assert_eq!(list[1].contributor, SIGNER);
    assert_eq!(list[1].signature.as_ref().unwrap().issued_at, LATER);

    // An unchanged release keeps the other contributor's manifest as well.
    let kept = endorse_entry(again.clone(), &params(SIGNER, LATER, None), &key()).unwrap();
    assert_eq!(kept, again);
}

#[test]
fn entry_without_digest_is_refused() {
    let mut e = entry();
    e.digest = None;
    assert_eq!(
        endorse_entry(e, &params(SIGNER, ISSUED, None), &key()).unwrap_err(),
        SignError::DigestMissing
    );
}

#[test]
fn endorse_catalog_reaches_publisher_endorsement() {
    let body = br#"{"name":"served"}"#;
    let server = Server::start(vec![
        Reply::ok("application/json", body),
        Reply::ok("application/json", body),
    ]);
    let mut served = url_entry(
        "urn:air:example.com:fixture:u",
        &server.url("/card.json"),
        b"",
    );
    served.digest = None;
    let mut inline = entry();
    inline.digest = None;
    let catalog = world::catalog(vec![inline, served]);

    let endorsed = endorse_catalog(
        catalog,
        None,
        &params(SIGNER, ISSUED, None),
        &key(),
        &fetch_policy(),
    )
    .unwrap();

    let tree = world::tree(&endorsed);
    let report = verify_resolved(&tree, &docs(&[SIGNER]), &world::policy(), at(0));
    assert_eq!(report.catalogs[0].entries.len(), 2);
    for entry in &report.catalogs[0].entries {
        assert!(matches!(entry.integrity, Some(Ok(()))), "{entry:?}");
        assert_eq!(entry.manifests.len(), 1);
        assert_eq!(entry.manifests[0].endorsement(), Endorsement::Publisher);
    }
}

#[test]
fn endorse_catalog_skips_entries_of_other_publishers() {
    let mut mine = entry();
    mine.digest = None;
    let theirs = data_entry("urn:air:other.example:fixture:b", r#"{"b":1}"#);
    let mut undigested = data_entry("urn:air:other.example:fixture:c", r#"{"c":1}"#);
    undigested.digest = None;
    let catalog = world::catalog(vec![mine, theirs.clone(), undigested.clone()]);

    let endorsed = endorse_catalog(
        catalog,
        None,
        &params(SIGNER, ISSUED, None),
        &key(),
        &fetch_policy(),
    )
    .unwrap();
    assert_eq!(manifests(&endorsed.entries[0]).len(), 1);
    assert!(endorsed.entries[0].digest.is_some());
    assert_eq!(endorsed.entries[1], theirs);
    assert_eq!(endorsed.entries[2], undigested);
}

#[test]
fn endorse_catalog_is_idempotent() {
    let mut a = entry();
    a.digest = None;
    let mut b = data_entry("urn:air:example.com:fixture:b", r#"{"b":[1,2]}"#);
    b.digest = None;
    let catalog = world::catalog(vec![a, b]);

    let once = endorse_catalog(
        catalog,
        None,
        &params(SIGNER, ISSUED, None),
        &key(),
        &fetch_policy(),
    )
    .unwrap();
    let twice = endorse_catalog(
        once.clone(),
        None,
        &params(SIGNER, LATER, None),
        &key(),
        &fetch_policy(),
    )
    .unwrap();
    assert_eq!(bytes(&once), bytes(&twice));

    let same = endorse_catalog(
        once.clone(),
        None,
        &params(SIGNER, ISSUED, None),
        &key(),
        &fetch_policy(),
    )
    .unwrap();
    assert_eq!(bytes(&once), bytes(&same));
}

#[test]
fn first_failure_names_its_entry_and_returns_nothing() {
    let server = Server::start(vec![Reply::new(500)]);
    let mut ok = entry();
    ok.digest = None;
    let mut bad = url_entry("urn:air:example.com:fixture:u", &server.url("/x"), b"");
    bad.digest = None;
    let catalog = world::catalog(vec![ok, bad]);
    let err = endorse_catalog(
        catalog,
        None,
        &params(SIGNER, ISSUED, None),
        &key(),
        &fetch_policy(),
    )
    .unwrap_err();
    assert_eq!(err.entry, 1);
    assert!(matches!(
        err.kind,
        turul_ai_catalog_trust::EndorseErrorKind::Digest(_)
    ));
}

#[test]
fn manifest_signed_by_another_key_under_the_same_kid_is_resigned() {
    let first = endorse_entry(entry(), &params(SIGNER, ISSUED, None), &key()).unwrap();
    let replacement = turul_ai_catalog_trust::SigningKey::generate();
    let again = endorse_entry(first, &params(SIGNER, LATER, None), &replacement).unwrap();
    let [only] = manifests(&again) else {
        panic!("one manifest expected");
    };
    assert_eq!(only.signature.as_ref().unwrap().issued_at, LATER);
}

#[test]
fn stale_signer_manifests_are_dropped_when_one_is_kept() {
    let base = entry();
    let current = endorse_entry(base.clone(), &params(SIGNER, ISSUED, None), &key()).unwrap();
    let mut stale = signed_at(&base, SIGNER, ISSUED, None);
    stale.signature.as_mut().unwrap().jws = "e30..e30".to_string();
    let mut both = current.clone();
    both.trust_manifests.as_mut().unwrap().push(stale);
    let again = endorse_entry(both, &params(SIGNER, LATER, None), &key()).unwrap();
    assert_eq!(again, current);
}

#[test]
fn requested_expiry_always_resigns() {
    let first = endorse_entry(entry(), &params(SIGNER, ISSUED, None), &key()).unwrap();
    let expires = "2027-01-01T00:00:00Z";
    let again = endorse_entry(first, &params(SIGNER, LATER, Some(expires)), &key()).unwrap();
    let [only] = manifests(&again) else {
        panic!("one manifest expected");
    };
    assert_eq!(
        only.signature.as_ref().unwrap().expires_at.as_deref(),
        Some(expires)
    );
}

#[test]
fn a_matching_digest_is_kept_as_written() {
    let mut e = data_entry(world::ID, r#"{"a":1}"#);
    let jcs = br#"{"a":1}"#;
    e.digest = Some(format!(
        "SHA512:{}",
        &turul_ai_catalog_trust::compute("sha512", jcs).unwrap()["sha512:".len()..]
    ));
    let theirs = signed_at(&e, OTHER, ISSUED, None);
    e.trust_manifests = Some(vec![theirs.clone()]);
    let digest = e.digest.clone();
    let endorsed = endorse_catalog(
        world::catalog(vec![e]),
        None,
        &params(SIGNER, ISSUED, None),
        &key(),
        &fetch_policy(),
    )
    .unwrap();
    assert_eq!(endorsed.entries[0].digest, digest);
    assert!(theirs.subject.as_ref().unwrap().binds(&endorsed.entries[0]));
}

#[test]
fn a_changed_entry_drops_the_catalog_signature() {
    let mut a = entry();
    a.digest = None;
    let signed = world::sign_root(world::catalog(vec![a]), SIGNER);
    assert!(signed.signature.is_some());
    let endorsed = endorse_catalog(
        signed,
        None,
        &params(SIGNER, ISSUED, None),
        &key(),
        &fetch_policy(),
    )
    .unwrap();
    assert!(endorsed.signature.is_none());

    // Nothing changed: the catalog signature stays.
    let resigned = world::sign_root(endorsed, SIGNER);
    let again = endorse_catalog(
        resigned.clone(),
        None,
        &params(SIGNER, LATER, None),
        &key(),
        &fetch_policy(),
    )
    .unwrap();
    assert_eq!(bytes(&again), bytes(&resigned));
}
