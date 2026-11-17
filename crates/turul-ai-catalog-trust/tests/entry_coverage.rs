//! Entry Release Coverage: signer, subject binding and artifact bytes.

mod support;

#[path = "../../turul-ai-catalog-client/tests/support/loopback.rs"]
mod loopback;

use loopback::{Reply, Server};
use support::world::{self, ID, SIGNER, at, entry, signed, url_entry, with_manifests};
use turul_ai_catalog::model::{CatalogEntry, TrustManifest};
use turul_ai_catalog::validate::Code;
use turul_ai_catalog_trust::{
    Coverage, CoverageGap, Endorsement, IntegrityError, Reason, SignatureOutcome, SignedObject,
    TrustPolicy, entry_release_coverage, verify_resolved, verify_signature,
};

fn report_of(
    e: CatalogEntry,
    manifests: Vec<TrustManifest>,
    signers: &[&str],
) -> (Endorsement, Option<Coverage>, bool, Option<String>) {
    let tree = world::tree(&world::catalog(vec![with_manifests(e, manifests)]));
    let r = verify_resolved(&tree, &world::docs(signers), &world::policy(), at(0));
    let entry = &r.catalogs[0].entries[0];
    let m = &entry.manifests[0];
    (
        m.endorsement(),
        m.coverage.clone(),
        matches!(m.signature, SignatureOutcome::Accepted(_)),
        entry.integrity.as_ref().map(|i| format!("{i:?}")),
    )
}

#[test]
fn covered_when_signed_bound_and_intact() {
    let e = entry();
    let m = signed(&e, SIGNER);
    let (end, cov, accepted, integrity) = report_of(e, vec![m], &[SIGNER]);
    assert!(accepted);
    assert_eq!(end, Endorsement::Publisher);
    assert!(matches!(cov, Some(Coverage::Covered)));
    assert_eq!(integrity.as_deref(), Some("Ok(())"));
}

#[test]
fn digest_mismatch_leaves_a_verified_manifest_uncovered() {
    // The entry digest is that of other bytes, so it still binds but the artifact differs.
    let mut e = entry();
    e.digest = Some(turul_ai_catalog_trust::compute("sha256", b"other").unwrap());
    let m = signed(&e, SIGNER);
    let (end, cov, accepted, _) = report_of(e, vec![m], &[SIGNER]);
    assert!(accepted);
    assert_eq!(end, Endorsement::None);
    assert!(matches!(
        cov,
        Some(Coverage::Uncovered(CoverageGap::Integrity(
            IntegrityError::Mismatch { .. }
        )))
    ));
}

#[test]
fn signer_not_contributor_is_rejected_and_uncovered() {
    let e = entry();
    let mut m = signed(&e, SIGNER);
    m.contributor = "did:web:other.example".into();
    let tree = world::tree(&world::catalog(vec![with_manifests(e, vec![m])]));
    let r = verify_resolved(&tree, &world::docs(&[SIGNER]), &world::policy(), at(0));
    let m = &r.catalogs[0].entries[0].manifests[0];
    assert!(matches!(
        m.signature,
        SignatureOutcome::Rejected(Reason::Invalid(Code::SignerContributorMismatch))
    ));
    assert!(m.coverage.is_none());
    assert_eq!(m.endorsement(), Endorsement::None);
}

#[test]
fn version_added_after_signing_is_uncovered() {
    let mut e = entry();
    let m = signed(&e, SIGNER);
    e.version = Some("1.0.0".into());
    let (end, cov, accepted, integrity) = report_of(e, vec![m], &[SIGNER]);
    assert!(accepted);
    assert_eq!(end, Endorsement::None);
    assert!(matches!(
        cov,
        Some(Coverage::Uncovered(CoverageGap::SubjectMismatch))
    ));
    assert!(integrity.is_none(), "nothing relied on, nothing fetched");
}

#[test]
fn catalog_signature_is_never_entry_coverage() {
    let c = world::sign_root(world::catalog(vec![entry()]), SIGNER);
    let a = verify_signature(
        SignedObject::Catalog(&c),
        &world::doc(SIGNER),
        &TrustPolicy::default(),
        at(0),
    )
    .unwrap();
    assert!(matches!(
        entry_release_coverage(&a, &c.entries[0], &Ok(())),
        Coverage::Uncovered(CoverageGap::NotAManifest)
    ));
}

#[test]
fn integrity_is_checked_only_after_the_subject() {
    let e = entry();
    let m = signed(&e, SIGNER);
    let mut moved = e.clone();
    moved.version = Some("2".into());
    let a = verify_signature(
        SignedObject::Manifest(&m),
        &world::doc(SIGNER),
        &TrustPolicy::default(),
        at(0),
    )
    .unwrap();
    assert!(matches!(
        entry_release_coverage(&a, &moved, &Err(IntegrityError::DigestMissing)),
        Coverage::Uncovered(CoverageGap::SubjectMismatch)
    ));
    assert!(matches!(
        entry_release_coverage(&a, &e, &Err(IntegrityError::DigestMissing)),
        Coverage::Uncovered(CoverageGap::Integrity(IntegrityError::DigestMissing))
    ));
    assert!(matches!(
        entry_release_coverage(&a, &e, &Ok(())),
        Coverage::Covered
    ));
}

#[test]
fn integrity_is_fetched_once_and_only_when_relied_on() {
    let body = br#"{"served":true}"#;
    let server = Server::start(vec![
        Reply::ok("application/json", body),
        Reply::ok("application/json", body),
    ]);
    let relied = url_entry(ID, &server.url("/a.json"), body);
    let two = vec![
        signed(&relied, "did:web:one.example"),
        signed(&relied, "did:web:two.example"),
    ];
    let unrelied = url_entry(
        "urn:air:example.com:fixture:b",
        &server.url("/b.json"),
        body,
    );
    let mut rejected = signed(&unrelied, "did:web:one.example");
    rejected.signature.as_mut().unwrap().jws = two[0].signature.as_ref().unwrap().jws.clone();
    let catalog = world::catalog(vec![
        with_manifests(relied, two),
        with_manifests(unrelied, vec![rejected]),
    ]);
    let tree = world::tree(&catalog);
    let docs = world::docs(&["did:web:one.example", "did:web:two.example"]);
    let r = verify_resolved(&tree, &docs, &world::policy(), at(0));

    let requests = server.log();
    assert_eq!(requests.len(), 1, "{requests:?}");
    assert_eq!(requests[0].path(), "/a.json");
    let a = &r.catalogs[0].entries;
    assert!(
        a[0].manifests
            .iter()
            .all(|m| m.endorsement() == Endorsement::Contributor)
    );
    assert!(a[1].integrity.is_none());
}
