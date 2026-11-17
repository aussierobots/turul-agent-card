//! Publisher authorization under the root `did:web` domain rule.

mod support;

use support::world::{self, at, entry, signed, with_manifests};
use turul_ai_catalog::identity::EntryIdentifier;
use turul_ai_catalog_trust::{
    Coverage, CoverageGap, DidWebRoot, Endorsement, publisher_authorized, verify_resolved,
};

fn authorized(identifier: &str, signer: &str) -> bool {
    let signer = DidWebRoot::parse(signer).unwrap_or_else(|| panic!("{signer}"));
    publisher_authorized(&EntryIdentifier::new(identifier), &signer)
}

const BILLING: &str = "urn:air:example.com:agent:billing";

#[test]
fn non_urn_air_identifier_is_never_authorized() {
    for id in [
        "https://example.com/billing",
        "did:web:example.com",
        "urn:isbn:example.com:agent:billing",
        "URN:AIR:example.com:agent:billing",
        "billing",
    ] {
        assert!(!authorized(id, "did:web:example.com"), "{id}");
    }
}

#[test]
fn urn_air_with_two_components_is_never_authorized() {
    assert!(!authorized(
        "urn:air:example.com:billing",
        "did:web:example.com"
    ));
    assert!(!authorized("urn:air:example.com", "did:web:example.com"));
    assert!(authorized(
        "urn:air:example.com:agent:sub:billing",
        "did:web:example.com"
    ));
}

#[test]
fn publisher_failing_the_domain_rule_is_never_authorized() {
    for publisher in ["Example.com", "2130706433", "127.1", "0x7f.0.0.1"] {
        assert_eq!(DidWebRoot::parse(&format!("did:web:{publisher}")), None);
        let id = format!("urn:air:{publisher}:agent:billing");
        for signer in ["did:web:example.com", "did:web:localhost"] {
            assert!(!authorized(&id, signer), "{id} by {signer}");
        }
    }
}

#[test]
fn publisher_signer_is_authorized() {
    assert!(authorized(BILLING, "did:web:example.com"));
}

#[test]
fn assessor_is_endorsed_but_not_publisher_authorized() {
    assert!(!authorized(BILLING, "did:web:assessor.example"));
    let e = world::data_entry(BILLING, r#"{"a":1}"#);
    let m = signed(&e, "did:web:assessor.example");
    let tree = world::tree(&world::catalog(vec![with_manifests(e, vec![m])]));
    let docs = world::docs(&["did:web:assessor.example"]);
    let r = verify_resolved(&tree, &docs, &world::policy(), at(0));
    let m = &r.catalogs[0].entries[0].manifests[0];
    assert!(!m.publisher_authorized);
    assert_eq!(m.endorsement(), Endorsement::Contributor);
}

#[test]
fn subdomain_signer_is_not_authorized() {
    assert!(!authorized(BILLING, "did:web:sub.example.com"));
    assert!(!authorized(
        "urn:air:sub.example.com:agent:billing",
        "did:web:example.com"
    ));
}

#[test]
fn suffix_signer_is_not_authorized() {
    assert!(!authorized(BILLING, "did:web:example.com.evil"));
    assert!(!authorized(BILLING, "did:web:evilexample.com"));
}

#[test]
fn publisher_authorization_needs_release_coverage() {
    let mut e = world::data_entry(BILLING, r#"{"a":1}"#);
    let m = signed(&e, "did:web:example.com");
    let docs = world::docs(&["did:web:example.com"]);

    let tree = world::tree(&world::catalog(vec![with_manifests(
        e.clone(),
        vec![m.clone()],
    )]));
    let r = verify_resolved(&tree, &docs, &world::policy(), at(0));
    assert_eq!(
        r.catalogs[0].entries[0].manifests[0].endorsement(),
        Endorsement::Publisher
    );

    e.version = Some("1.0.0".into());
    let tree = world::tree(&world::catalog(vec![with_manifests(e, vec![m])]));
    let r = verify_resolved(&tree, &docs, &world::policy(), at(0));
    let m = &r.catalogs[0].entries[0].manifests[0];
    assert!(m.publisher_authorized);
    assert!(matches!(
        m.coverage,
        Some(Coverage::Uncovered(CoverageGap::SubjectMismatch))
    ));
    assert_eq!(m.endorsement(), Endorsement::None);
    let _ = entry();
}
