//! Profile selection: `did-web-v1` by exact name, supported and permitted, never inferred.

mod support;

use support::world::{self, SIGNER, at, doc, entry, signed};
use turul_ai_catalog::model::TrustManifest;
use turul_ai_catalog_trust::{
    Endorsement, Reason, SignatureOutcome, SignedObject, TrustPolicy, verify_resolved,
    verify_signature,
};

fn with_profile(profile: &str) -> TrustManifest {
    let mut m = signed(&entry(), SIGNER);
    m.signature.as_mut().unwrap().profile = profile.to_string();
    m
}

fn verify(m: &TrustManifest, policy: &TrustPolicy) -> Result<(), Reason> {
    verify_signature(SignedObject::Manifest(m), &doc(SIGNER), policy, at(0)).map(|_| ())
}

#[test]
fn absolute_uri_profile_is_unsupported_and_never_authenticates() {
    let m = with_profile("https://example.com/profiles/private");
    assert!(matches!(
        verify(&m, &TrustPolicy::default()),
        Err(Reason::Unsupported(p)) if p == "https://example.com/profiles/private"
    ));
}

#[test]
fn did_web_v1_is_selected_by_its_exact_name() {
    assert!(verify(&with_profile("did-web-v1"), &TrustPolicy::default()).is_ok());
}

#[test]
fn policy_without_did_web_v1_is_disallowed() {
    let mut p = TrustPolicy::default();
    p.profiles = vec!["https://example.com/profiles/private".to_string()];
    assert!(matches!(
        verify(&with_profile("did-web-v1"), &p),
        Err(Reason::Disallowed)
    ));
    p.profiles.clear();
    assert!(matches!(
        verify(&with_profile("did-web-v1"), &p),
        Err(Reason::Disallowed)
    ));
}

#[test]
fn case_variant_profile_is_unsupported_without_fallback() {
    for name in ["DID-WEB-V1", "Did-Web-V1", "did-web-v1 ", "did-web-v2"] {
        assert!(
            matches!(
                verify(&with_profile(name), &TrustPolicy::default()),
                Err(Reason::Unsupported(_))
            ),
            "{name:?}"
        );
    }
}

#[test]
fn unsupported_or_disallowed_signature_never_counts() {
    let e = entry();
    let ok = signed(&e, "did:web:good.example");
    let mut bad = signed(&e, "did:web:other.example");
    bad.signature.as_mut().unwrap().profile = "DID-WEB-V1".into();
    let tree = world::tree(&world::catalog(vec![world::with_manifests(
        e,
        vec![bad, ok],
    )]));
    let docs = world::docs(&["did:web:good.example", "did:web:other.example"]);
    let report = verify_resolved(&tree, &docs, &world::policy(), at(0));
    let m = &report.catalogs[0].entries[0].manifests;
    assert!(matches!(
        m[0].signature,
        SignatureOutcome::Rejected(Reason::Unsupported(_))
    ));
    assert_eq!(m[0].endorsement(), Endorsement::None);
    assert_eq!(m[1].endorsement(), Endorsement::Contributor);

    let mut none = world::policy();
    none.profiles.clear();
    let report = verify_resolved(&tree, &docs, &none, at(0));
    assert!(report.catalogs[0].entries[0].manifests.iter().all(|m| {
        matches!(m.signature, SignatureOutcome::Rejected(_)) && m.endorsement() == Endorsement::None
    }));
}

#[test]
fn stricter_local_policy_keeps_the_profile_name() {
    let mut p = TrustPolicy::default();
    p.clock_skew = std::time::Duration::ZERO;
    assert!(p.operators.is_empty());
    assert_eq!(p.profiles, ["did-web-v1"]);
    let m = signed(&entry(), SIGNER);
    let a = verify_signature(SignedObject::Manifest(&m), &doc(SIGNER), &p, at(0)).unwrap();
    assert_eq!(a.object.signature().unwrap().profile, "did-web-v1");
}

#[test]
fn did_web_v1_authenticates_the_exact_signer() {
    let m = signed(&entry(), SIGNER);
    let other = doc("did:web:other.example");
    assert!(matches!(
        verify_signature(
            SignedObject::Manifest(&m),
            &other,
            &TrustPolicy::default(),
            at(0)
        ),
        Err(Reason::Resolve(Some(
            turul_ai_catalog_trust::ResolveError::IdMismatch
        )))
    ));
}

#[test]
fn only_did_web_v1_is_supported() {
    for name in [
        "did-web",
        "did-web-v1.1",
        "x509",
        "jws",
        "https://example.com/p",
    ] {
        let mut p = TrustPolicy::default();
        p.profiles.push(name.to_string());
        assert!(
            matches!(verify(&with_profile(name), &p), Err(Reason::Unsupported(_))),
            "{name}"
        );
    }
}

#[test]
fn did_web_rules_apply_only_under_did_web_v1() {
    // A broken header and a non-DID signer are never judged under another profile's name.
    let mut m = signed(&entry(), SIGNER);
    {
        let s = m.signature.as_mut().unwrap();
        s.profile = "https://example.com/p".into();
        s.jws = support::jws_with("{}");
    }
    assert!(matches!(
        verify(&m, &TrustPolicy::default()),
        Err(Reason::Unsupported(_))
    ));
    let mut m = signed(&entry(), SIGNER);
    {
        let s = m.signature.as_mut().unwrap();
        s.profile = "https://example.com/p".into();
        s.signer = "https://example.com/key".into();
    }
    m.contributor = "https://example.com/key".into();
    assert!(matches!(
        verify(&m, &TrustPolicy::default()),
        Err(Reason::Unsupported(_))
    ));
}
