//! Catalog endorsement needs a configured operator.

mod support;

use support::world::{self, SIGNER, at, doc, entry, root, sign_root};
use turul_ai_catalog::identity::HostIdentifier;
use turul_ai_catalog::model::HostInfo;
use turul_ai_catalog_trust::{
    SignatureOutcome, SignedObject, TrustPolicy, verify_resolved, verify_signature,
};

fn report_endorsed(policy: &TrustPolicy, host_identifier: Option<&str>) -> (bool, bool) {
    let mut c = world::catalog(vec![entry()]);
    if let Some(id) = host_identifier {
        let mut h = HostInfo::new("Host".into());
        h.identifier = Some(HostIdentifier::new(id));
        c.host = Some(h);
    }
    let c = sign_root(c, SIGNER);
    let tree = world::tree(&c);
    let r = verify_resolved(&tree, &world::docs(&[SIGNER]), policy, at(0));
    (
        matches!(r.catalogs[0].signature, SignatureOutcome::Accepted(_)),
        r.catalogs[0].endorsed,
    )
}

#[test]
fn verified_signature_with_empty_operators_is_not_endorsed() {
    let policy = world::policy();
    assert!(policy.operators.is_empty());
    assert_eq!(report_endorsed(&policy, None), (true, false));
    let c = sign_root(world::catalog(vec![]), SIGNER);
    assert!(verify_signature(SignedObject::Catalog(&c), &doc(SIGNER), &policy, at(0)).is_ok());
}

#[test]
fn operator_signature_is_endorsed() {
    let mut policy = world::policy();
    policy.operators.push(root(SIGNER));
    assert_eq!(report_endorsed(&policy, None), (true, true));
    let mut other = world::policy();
    other.operators.push(root("did:web:other.example"));
    assert_eq!(report_endorsed(&other, None), (true, false));
}

#[test]
fn host_identifier_is_never_consulted() {
    let policy = world::policy();
    assert_eq!(report_endorsed(&policy, Some(SIGNER)), (true, false));
    let mut other = world::policy();
    other.operators.push(root("did:web:other.example"));
    assert_eq!(
        report_endorsed(&other, Some("did:web:other.example")),
        (true, false)
    );
}
