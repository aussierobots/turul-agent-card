//! Acceptance of one signature: order of checks, time, header agreement and independence.

mod support;

use std::time::{Duration, SystemTime};

use support::world::{self, ID, ISSUED, SIGNER, at, doc, entry, signed, signed_at};
use turul_ai_catalog::model::TrustManifest;
use turul_ai_catalog::validate::Code;
use turul_ai_catalog_trust::{
    AcceptedSignature, DidDocument, HeaderError, Reason, ResolveError, SignatureOutcome,
    SignedObject, TrustPolicy, inspect, verify_resolved, verify_signature,
};
use turul_jwt_validator::detached::DetachedJwsError;

const MANIFEST_TYP: &str = "ai-catalog-trust-manifest+jws";
const CATALOG_TYP: &str = "ai-catalog+jws";
const KID: &str = "did:web:example.com#key-1";

fn check<'a>(
    m: &'a TrustManifest,
    d: &DidDocument,
    p: &TrustPolicy,
    now: SystemTime,
) -> Result<AcceptedSignature<'a>, Reason> {
    verify_signature(SignedObject::Manifest(m), d, p, now)
}

fn accepted(m: &TrustManifest, now: SystemTime) -> Result<AcceptedSignature<'_>, Reason> {
    check(m, &doc(SIGNER), &TrustPolicy::default(), now)
}

fn skewed(secs: u64) -> TrustPolicy {
    let mut p = TrustPolicy::default();
    p.clock_skew = Duration::from_secs(secs);
    p
}

fn with_jws(mut m: TrustManifest, header: &str) -> TrustManifest {
    m.signature.as_mut().unwrap().jws = support::jws_with(header);
    m
}

fn manifest_header(alg: &str, kid: &str) -> String {
    format!(r#"{{"alg":"{alg}","typ":"{MANIFEST_TYP}","kid":"{kid}"}}"#)
}

fn swapped_signature(m: &TrustManifest) -> TrustManifest {
    let e = world::data_entry("urn:air:example.com:fixture:other", r#"{"b":2}"#);
    let other = signed(&e, SIGNER);
    let mut out = m.clone();
    out.signature.as_mut().unwrap().jws = other.signature.unwrap().jws;
    out
}

fn base() -> TrustManifest {
    signed(&entry(), SIGNER)
}

fn rejected(r: Result<AcceptedSignature<'_>, Reason>) -> Reason {
    match r {
        Err(e) => e,
        Ok(a) => panic!("accepted {a:?}"),
    }
}

#[test]
fn es256_is_verified() {
    let m = base();
    let a = accepted(&m, at(0)).unwrap();
    assert_eq!(a.signer.as_str(), SIGNER);
    assert_eq!(a.kid.as_str(), KID);
    assert_eq!(a.expires_at, None);
    assert_eq!(a.issued_at.to_string(), ISSUED);
}

#[test]
fn accepted_at_issued_at() {
    assert!(accepted(&base(), at(0)).is_ok());
    assert!(accepted(&base(), at(1_000_000)).is_ok());
}

#[test]
fn rejected_one_second_before_issued_at() {
    assert!(matches!(
        rejected(accepted(&base(), at(-1))),
        Reason::NotYetValid
    ));
}

#[test]
fn rejected_at_expires_at() {
    let m = signed_at(&entry(), SIGNER, ISSUED, Some("2026-09-12T11:00:00Z"));
    assert!(accepted(&m, at(3599)).is_ok());
    assert!(matches!(rejected(accepted(&m, at(3600))), Reason::Expired));
    assert!(matches!(rejected(accepted(&m, at(7200))), Reason::Expired));
}

#[test]
fn offsets_compare_as_instants() {
    let m = signed_at(
        &entry(),
        SIGNER,
        "2026-09-12T20:00:00+10:00",
        Some("2026-09-12T21:00:00+10:00"),
    );
    assert!(accepted(&m, at(0)).is_ok());
    assert!(matches!(
        rejected(accepted(&m, at(-1))),
        Reason::NotYetValid
    ));
    assert!(matches!(rejected(accepted(&m, at(3600))), Reason::Expired));
}

#[test]
fn clock_skew_widens_both_bounds() {
    let m = signed_at(&entry(), SIGNER, ISSUED, Some("2026-09-12T11:00:00Z"));
    let ok = |secs, now| check(&m, &doc(SIGNER), &skewed(secs), at(now)).is_ok();
    assert!(!ok(0, -1));
    assert!(ok(2, -1));
    assert!(ok(2, -2));
    assert!(!ok(2, -3));
    assert!(ok(2, 3601));
    assert!(!ok(2, 3602));
}

#[test]
fn unrepresentable_skew_is_never_looser() {
    let m = signed_at(&entry(), SIGNER, ISSUED, Some("2026-09-12T11:00:00Z"));
    let mut p = TrustPolicy::default();
    p.clock_skew = Duration::MAX;
    let r = |now| check(&m, &doc(SIGNER), &p, at(now));
    assert!(r(0).is_ok());
    assert!(matches!(rejected(r(-1)), Reason::NotYetValid));
    assert!(matches!(rejected(r(3600)), Reason::Expired));
}

#[test]
fn matching_signer_with_failing_jws_is_not_authenticated() {
    let m = swapped_signature(&base());
    assert_eq!(m.signature.as_ref().unwrap().signer, m.contributor);
    assert!(matches!(
        rejected(accepted(&m, at(0))),
        Reason::Signature(DetachedJwsError::SignatureInvalid)
    ));
}

#[test]
fn error_diagnostic_in_signed_object_rejects() {
    let mut m = base();
    let digest = m
        .subject
        .as_ref()
        .unwrap()
        .digest
        .to_uppercase()
        .replace("SHA256", "sha256");
    m.subject.as_mut().unwrap().digest = digest;
    assert!(matches!(
        rejected(accepted(&m, at(0))),
        Reason::Invalid(Code::DigestInvalid)
    ));
}

#[test]
fn checks_run_in_the_documented_order() {
    let doc_ok = doc(SIGNER);
    let p = TrustPolicy::default();

    // 0: no signature.
    let mut unsigned = base();
    unsigned.signature = None;
    assert!(matches!(
        rejected(check(&unsigned, &doc_ok, &p, at(0))),
        Reason::Unsigned
    ));

    // 1: an error diagnostic, although the profile is also unsupported.
    let mut m = base();
    m.signature.as_mut().unwrap().profile = "other".into();
    m.contributor = "did:web:elsewhere.example".into();
    assert!(matches!(
        rejected(check(&m, &doc_ok, &p, at(0))),
        Reason::Invalid(Code::SignerContributorMismatch)
    ));

    // 2: unsupported, although the signer shape and header are also bad.
    let mut m = with_jws(base(), "{}");
    m.signature.as_mut().unwrap().profile = "other".into();
    assert!(matches!(
        rejected(check(&m, &doc_ok, &p, at(0))),
        Reason::Unsupported(_)
    ));
    let mut none = TrustPolicy::default();
    none.profiles.clear();
    assert!(matches!(
        rejected(check(&with_jws(base(), "{}"), &doc_ok, &none, at(0))),
        Reason::Disallowed
    ));

    // 3: signer shape, although the header is also bad.
    let bad_signer = "did:web:Example.com";
    let mut m = with_jws(base(), "{}");
    m.contributor = bad_signer.into();
    m.signature.as_mut().unwrap().signer = bad_signer.into();
    assert!(matches!(
        rejected(check(&m, &doc_ok, &p, at(0))),
        Reason::SignerShape
    ));

    // 4: header, although the document is also wrong.
    let m = with_jws(base(), &manifest_header("ES384", KID));
    assert!(matches!(
        rejected(check(&m, &doc("did:web:other.example"), &p, at(0))),
        Reason::Header(HeaderError::Jws(DetachedJwsError::AlgNotAllowed))
    ));

    // 6: document id, although the key would be missing too.
    let wrong = doc("did:web:other.example");
    assert!(matches!(
        rejected(check(&base(), &wrong, &p, at(0))),
        Reason::Resolve(Some(ResolveError::IdMismatch))
    ));

    // 7: key selection, although the signature is also wrong.
    let nokey = world::doc_with_fragment(SIGNER, "key-2");
    assert!(matches!(
        rejected(check(&swapped_signature(&base()), &nokey, &p, at(0))),
        Reason::Key(_)
    ));

    // 8: the signature, although the time is also wrong.
    assert!(matches!(
        rejected(check(&swapped_signature(&base()), &doc_ok, &p, at(-100))),
        Reason::Signature(_)
    ));

    // 9: time, last.
    assert!(matches!(
        rejected(check(&base(), &doc_ok, &p, at(-100))),
        Reason::NotYetValid
    ));
}

#[test]
fn other_alg_is_rejected_before_any_lookup() {
    let m = with_jws(base(), &manifest_header("ES384", KID));
    let tree = world::tree(&world::catalog(vec![world::with_manifests(
        entry(),
        vec![m],
    )]));
    let report = verify_resolved(&tree, &Default::default(), &TrustPolicy::default(), at(0));
    assert!(matches!(
        header_outcome(&report),
        Reason::Header(HeaderError::Jws(DetachedJwsError::AlgNotAllowed))
    ));
}

#[test]
fn kid_mismatch_fails_before_any_lookup() {
    let m = with_jws(
        base(),
        &manifest_header("ES256", "did:web:other.example#key-1"),
    );
    let tree = world::tree(&world::catalog(vec![world::with_manifests(
        entry(),
        vec![m],
    )]));
    let report = verify_resolved(&tree, &Default::default(), &TrustPolicy::default(), at(0));
    assert!(matches!(
        header_outcome(&report),
        Reason::Header(HeaderError::KidMismatch)
    ));
}

#[test]
fn signer_shape_is_rejected_before_any_lookup() {
    let bad = "did:web:Example.com";
    let mut m = base();
    m.contributor = bad.into();
    m.signature.as_mut().unwrap().signer = bad.into();
    assert!(matches!(rejected(accepted(&m, at(0))), Reason::SignerShape));
    let tree = world::tree(&world::catalog(vec![world::with_manifests(
        entry(),
        vec![m],
    )]));
    let report = verify_resolved(&tree, &Default::default(), &TrustPolicy::default(), at(0));
    assert!(matches!(header_outcome(&report), Reason::SignerShape));
}

fn header_outcome<'r>(report: &'r turul_ai_catalog_trust::TrustReport<'_>) -> &'r Reason {
    match &report.catalogs[0].entries[0].manifests[0].signature {
        SignatureOutcome::Rejected(r) => r,
        other => panic!("{other:?}"),
    }
}

#[test]
fn duplicate_typ_cannot_replay_across_kinds() {
    let jws_catalog_then_manifest = support::jws_with(&format!(
        r#"{{"alg":"ES256","typ":"{CATALOG_TYP}","kid":"{KID}","typ":"{MANIFEST_TYP}"}}"#
    ));
    let jws_manifest_then_catalog = support::jws_with(&format!(
        r#"{{"alg":"ES256","typ":"{MANIFEST_TYP}","kid":"{KID}","typ":"{CATALOG_TYP}"}}"#
    ));

    let mut c = world::sign_root(world::catalog(vec![]), SIGNER);
    c.signature.as_mut().unwrap().jws = jws_catalog_then_manifest;
    assert!(matches!(
        verify_signature(
            SignedObject::Catalog(&c),
            &doc(SIGNER),
            &TrustPolicy::default(),
            at(0)
        ),
        Err(Reason::Header(HeaderError::Jws(
            DetachedJwsError::InvalidHeader
        )))
    ));

    let mut m = base();
    m.signature.as_mut().unwrap().jws = jws_manifest_then_catalog;
    assert!(matches!(
        rejected(accepted(&m, at(0))),
        Reason::Header(HeaderError::Jws(DetachedJwsError::InvalidHeader))
    ));
}

#[test]
fn every_header_fixture_is_rejected_with_its_inspect_variant() {
    let kid = "did:web:acme.com#k";
    let signer = "did:web:acme.com";
    let headers = [
        format!(r#"{{"typ":"{MANIFEST_TYP}","kid":"{kid}"}}"#),
        format!(r#"{{"alg":"ES256","kid":"{kid}"}}"#),
        format!(r#"{{"alg":"ES256","typ":"{CATALOG_TYP}","kid":"{kid}"}}"#),
        format!(r#"{{"alg":"none","typ":"{MANIFEST_TYP}","kid":"{kid}"}}"#),
        format!(r#"{{"alg":"ES384","typ":"{MANIFEST_TYP}","kid":"{kid}"}}"#),
        format!(r#"{{"alg":"ES256","typ":"{MANIFEST_TYP}","kid":"{kid}","b64":false}}"#),
        format!(r#"{{"alg":"ES256","typ":"{MANIFEST_TYP}","kid":"{kid}","crit":["exp"]}}"#),
        format!(r#"{{"alg":"ES256","typ":"{MANIFEST_TYP}","kid":"{kid}","jku":"x"}}"#),
        format!(r#"{{"alg":"ES256","typ":"{MANIFEST_TYP}","kid":"{kid}","jwk":"x"}}"#),
        format!(r#"{{"alg":"ES256","typ":"{MANIFEST_TYP}","kid":"{kid}","x5u":"x"}}"#),
        format!(r#"{{"alg":"ES256","typ":"{MANIFEST_TYP}","kid":"{kid}","x5c":"x"}}"#),
        format!(r#"{{"alg":"ES256","typ":"{MANIFEST_TYP}"}}"#),
        format!(r#"{{"alg":"ES256","typ":"{MANIFEST_TYP}","kid":"did:web:other.example#k"}}"#),
        format!(r#"{{"alg":"ES256","typ":"{MANIFEST_TYP}","kid":"did:web:acme.com"}}"#),
        format!(r#"{{"alg":"ES256","typ":"{MANIFEST_TYP}","kid":"did:web:acme.com#"}}"#),
    ];
    let acme = signed(
        &world::data_entry("urn:air:acme.com:fixture:a", "1"),
        signer,
    );
    for h in headers {
        let m = with_jws(acme.clone(), &h);
        let expected =
            inspect(SignedObject::Manifest(&m), m.signature.as_ref().unwrap()).expect_err(&h);
        let got = check(
            &m,
            &world::doc_with_fragment(signer, "k"),
            &TrustPolicy::default(),
            at(0),
        );
        match got {
            Err(Reason::Header(e)) => assert_eq!(e, expected, "{h}"),
            other => panic!("{h}: {other:?}"),
        }
    }
}

#[test]
fn standalone_manifest_verifies_without_coverage() {
    // Outside any entry: only the manifest's own diagnostics apply.
    let m = base();
    let a = accepted(&m, at(0)).unwrap();
    assert!(matches!(a.object, SignedObject::Manifest(_)));
}

#[test]
fn spec_example_signatures_never_verify() {
    let m = support::manifest(
        r#"{"contributor":"did:web:acme.com","subject":{"identifier":"urn:air:acme.com:plugin:finance-suite","type":"application/ai-catalog+json","digest":"sha256:22223333444455556666777788889999aaaabbbbccccddddeeeeffff00001111"},"signature":{"signer":"did:web:acme.com","profile":"did-web-v1","issuedAt":"2026-03-20T14:00:00Z","jws":"eyJhbGciOiJFUzI1NiIsImtpZCI6ImRpZDp3ZWI6YWNtZS5jb20jcmVsZWFzZS1zaWduaW5nLWtleSIsInR5cCI6ImFpLWNhdGFsb2ctdHJ1c3QtbWFuaWZlc3QrandzIn0..detached-jws-signature"}}"#,
    );
    let d = world::doc_with_fragment("did:web:acme.com", "release-signing-key");
    let r = verify_signature(
        SignedObject::Manifest(&m),
        &d,
        &TrustPolicy::default(),
        at(0),
    );
    assert!(matches!(r, Err(Reason::Signature(_))), "{r:?}");
}

#[test]
fn updated_at_is_never_read() {
    let outcome = |updated: Option<&str>| {
        let mut e = entry();
        e.updated_at = updated.map(str::to_string);
        let m = signed_at(&e, SIGNER, ISSUED, Some("2026-09-12T11:00:00Z"));
        let tree = world::tree(&world::catalog(vec![world::with_manifests(e, vec![m])]));
        let report = verify_resolved(&tree, &world::docs(&[SIGNER]), &world::policy(), at(10));
        let r = &report.catalogs[0].entries[0].manifests[0];
        (
            matches!(r.signature, SignatureOutcome::Accepted(_)),
            r.endorsement(),
        )
    };
    let none = outcome(None);
    assert!(none.0);
    assert_eq!(outcome(Some("2020-01-01T00:00:00Z")), none);
    assert_eq!(outcome(Some("2030-01-01T00:00:00Z")), none);
}

fn four_signers() -> (turul_ai_catalog::model::AiCatalog, Vec<&'static str>) {
    let e = entry();
    let good = signed(&e, "did:web:good.example");
    let expired = signed_at(
        &e,
        "did:web:expired.example",
        ISSUED,
        Some("2026-09-12T10:00:05Z"),
    );
    let mut unsupported = signed(&e, "did:web:unsupported.example");
    unsupported.signature.as_mut().unwrap().profile = "https://example.com/profile".into();
    let mut invalid = signed(&e, "did:web:invalid.example");
    let other = world::data_entry("urn:air:example.com:fixture:other", r#"{"b":2}"#);
    invalid.signature.as_mut().unwrap().jws = signed(&other, "did:web:invalid.example")
        .signature
        .unwrap()
        .jws;
    (
        world::catalog(vec![world::with_manifests(
            e,
            vec![good, expired, unsupported, invalid],
        )]),
        vec![
            "did:web:good.example",
            "did:web:expired.example",
            "did:web:unsupported.example",
            "did:web:invalid.example",
        ],
    )
}

#[test]
fn invalid_unsupported_or_expired_never_counts() {
    let (c, signers) = four_signers();
    let tree = world::tree(&c);
    let report = verify_resolved(&tree, &world::docs(&signers), &world::policy(), at(60));
    let m = &report.catalogs[0].entries[0].manifests;
    assert!(matches!(
        m[1].signature,
        SignatureOutcome::Rejected(Reason::Expired)
    ));
    assert!(matches!(
        m[2].signature,
        SignatureOutcome::Rejected(Reason::Unsupported(_))
    ));
    assert!(matches!(
        m[3].signature,
        SignatureOutcome::Rejected(Reason::Signature(_))
    ));
    for rejected in &m[1..] {
        assert_eq!(
            rejected.endorsement(),
            turul_ai_catalog_trust::Endorsement::None
        );
    }
}

#[test]
fn other_manifests_are_evaluated_independently() {
    let (c, signers) = four_signers();
    let tree = world::tree(&c);
    let report = verify_resolved(&tree, &world::docs(&signers), &world::policy(), at(60));
    let m = &report.catalogs[0].entries[0].manifests;
    assert!(matches!(m[0].signature, SignatureOutcome::Accepted(_)));
    assert_eq!(
        m[0].endorsement(),
        turul_ai_catalog_trust::Endorsement::Contributor
    );
    let _ = ID;
}
