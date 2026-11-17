//! Selection among one contributor's manifests by authenticated `issuedAt`.

mod support;

use support::world::{self, SIGNER, at, entry, with_manifests};
use turul_ai_catalog::model::{Attestation, TrustManifest};
use turul_ai_catalog_trust::{Selection, select_current, verify_resolved};

fn manifest(issued: &str, expires: Option<&str>, note: &str) -> TrustManifest {
    let mut m = TrustManifest::new(SIGNER.to_string());
    m.attestations = Some(vec![Attestation::new(
        "SLSA-Provenance".into(),
        format!("https://example.com/{note}"),
    )]);
    turul_ai_catalog_trust::sign_manifest(
        &entry(),
        m,
        &world::params(SIGNER, issued, expires),
        &world::key(),
    )
    .unwrap()
}

fn selected(manifests: Vec<TrustManifest>, now: i64) -> Vec<String> {
    let tree = world::tree(&world::catalog(vec![with_manifests(entry(), manifests)]));
    let r = verify_resolved(&tree, &world::docs(&[SIGNER]), &world::policy(), at(now));
    select_current(&r.catalogs[0].entries[0])
        .into_iter()
        .map(|(c, s)| match s {
            Selection::Current(m) => {
                let uri = &m.manifest.attestations.as_ref().unwrap()[0].uri;
                format!("{c} current {uri}")
            }
            Selection::Tie(v) => format!("{c} tie {}", v.len()),
            _ => unreachable!(),
        })
        .collect()
}

#[test]
fn latest_authenticated_issued_at_is_current() {
    let early = manifest("2026-09-12T10:00:00Z", None, "early");
    let late = manifest("2026-09-12T11:00:00Z", None, "late");
    for order in [vec![early.clone(), late.clone()], vec![late, early]] {
        assert_eq!(
            selected(order, 7200),
            [format!("{SIGNER} current https://example.com/late")]
        );
    }
}

#[test]
fn rejected_later_manifest_is_never_current() {
    let early = manifest("2026-09-12T10:00:00Z", None, "early");
    let late = manifest("2026-09-12T11:00:00Z", Some("2026-09-12T11:30:00Z"), "late");
    assert_eq!(
        selected(vec![early, late], 7200),
        [format!("{SIGNER} current https://example.com/early")]
    );
}

#[test]
fn equal_issued_at_is_a_tie_in_either_array_order() {
    let a = manifest("2026-09-12T10:00:00Z", None, "a");
    let b = manifest("2026-09-12T10:00:00Z", None, "b");
    // The same instant spelled with an offset is still equal.
    let c = manifest("2026-09-12T20:00:00+10:00", None, "c");
    for order in [vec![a.clone(), b.clone()], vec![b.clone(), a.clone()]] {
        assert_eq!(selected(order, 7200), [format!("{SIGNER} tie 2")]);
    }
    assert_eq!(selected(vec![a, c], 7200), [format!("{SIGNER} tie 2")]);
}

#[test]
fn current_manifest_is_returned_unchanged() {
    let early = manifest("2026-09-12T10:00:00Z", None, "early");
    let late = manifest("2026-09-12T11:00:00Z", None, "late");
    let entry = with_manifests(entry(), vec![early, late.clone()]);
    let tree = world::tree(&world::catalog(vec![entry]));
    let r = verify_resolved(&tree, &world::docs(&[SIGNER]), &world::policy(), at(7200));
    let chosen = select_current(&r.catalogs[0].entries[0]);
    let Selection::Current(m) = &chosen[0].1 else {
        panic!("{chosen:?}")
    };
    assert_eq!(m.manifest, &late);
}
