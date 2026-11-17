//! The per-signature, per-entry report: independence, retention, nesting and what a
//! signature authenticates.

mod support;

#[path = "../../turul-ai-catalog-client/tests/support/loopback.rs"]
mod loopback;

use loopback::{Reply, Server};
use support::world::{
    self, ID, SIGNER, at, data_entry, entry, sign_root, signed, signed_at, with_manifests,
};
use turul_ai_catalog::identity::ArtifactType;
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::{AiCatalog, CatalogEntry, Members, Publisher, TrustManifest};
use turul_ai_catalog::validate::validate;
use turul_ai_catalog_trust::{
    Coverage, CoverageGap, DidDocuments, Endorsement, ManifestReport, Reason, SignatureOutcome,
    TrustReport, verify_resolved,
};

type Mutation<T> = Box<dyn Fn(&mut T)>;

const GOOD: &str = "did:web:good.example";
const OTHER: &str = "did:web:other.example";

fn run<'a>(
    tree: &'a turul_ai_catalog_client::ResolvedCatalog,
    docs: &DidDocuments,
    now: i64,
) -> TrustReport<'a> {
    verify_resolved(tree, docs, &world::policy(), at(now))
}

fn single(report: &TrustReport<'_>) -> Endorsement {
    report.catalogs[0].entries[0].manifests[0].endorsement()
}

fn first<'r, 'a>(report: &'r TrustReport<'a>) -> &'r ManifestReport<'a> {
    &report.catalogs[0].entries[0].manifests[0]
}

#[test]
fn endorsement_applies_coverage_and_authority() {
    // Signed by the publisher: coverage and authority both hold.
    let e = entry();
    let m = signed(&e, SIGNER);
    let tree = world::tree(&world::catalog(vec![with_manifests(e.clone(), vec![m])]));
    assert_eq!(
        single(&run(&tree, &world::docs(&[SIGNER]), 0)),
        Endorsement::Publisher
    );
    // Signed by someone else: coverage holds, authority does not.
    let m = signed(&e, GOOD);
    let tree = world::tree(&world::catalog(vec![with_manifests(e, vec![m])]));
    assert_eq!(
        single(&run(&tree, &world::docs(&[GOOD]), 0)),
        Endorsement::Contributor
    );
}

#[test]
fn manifest_authentication_covers_only_signed_contents() {
    let e = entry();
    let mut m = signed(&e, SIGNER);
    let docs = world::docs(&[SIGNER]);
    let ok = |m: &TrustManifest| {
        let tree = world::tree(&world::catalog(vec![with_manifests(
            e.clone(),
            vec![m.clone()],
        )]));
        matches!(
            first(&run(&tree, &docs, 0)).signature,
            SignatureOutcome::Accepted(_)
        )
    };
    assert!(ok(&m));
    // Anything in the manifest payload is authenticated: changing it breaks the signature.
    m.attestations = Some(vec![turul_ai_catalog::model::Attestation::new(
        "SLSA-Provenance".into(),
        "https://example.com/added-later".into(),
    )]);
    assert!(!ok(&m));
}

#[test]
fn header_and_shape_defects_are_outcomes_not_errors() {
    let e = entry();
    let good = signed(&e, SIGNER);
    let baseline = world::catalog(vec![with_manifests(e.clone(), vec![good.clone()])]);
    let level = validate(&baseline).conformance();

    let defects: Vec<(&str, Mutation<TrustManifest>)> = vec![
        (
            "alg",
            Box::new(|m| {
                m.signature.as_mut().unwrap().jws = support::jws_with(
                    r#"{"alg":"ES384","typ":"ai-catalog-trust-manifest+jws","kid":"did:web:example.com#key-1"}"#,
                )
            }),
        ),
        (
            "kid",
            Box::new(|m| {
                m.signature.as_mut().unwrap().jws = support::jws_with(
                    r#"{"alg":"ES256","typ":"ai-catalog-trust-manifest+jws","kid":"did:web:other.example#key-1"}"#,
                )
            }),
        ),
        (
            "signer shape",
            Box::new(|m| {
                m.contributor = "did:web:Example.com".into();
                m.signature.as_mut().unwrap().signer = "did:web:Example.com".into();
            }),
        ),
        (
            "profile",
            Box::new(|m| m.signature.as_mut().unwrap().profile = "other".into()),
        ),
    ];
    for (name, defect) in defects {
        let mut m = good.clone();
        defect(&mut m);
        let c = world::catalog(vec![with_manifests(e.clone(), vec![m])]);
        assert_eq!(validate(&c).conformance(), level, "{name}");
        let tree = world::tree(&c);
        let report = run(&tree, &world::docs(&[SIGNER]), 0);
        assert!(
            matches!(first(&report).signature, SignatureOutcome::Rejected(_)),
            "{name}"
        );
        assert_eq!(single(&report), Endorsement::None, "{name}");
    }
}

#[test]
fn unverified_content_is_retained() {
    let e = entry();
    let mut m = signed(&e, SIGNER);
    m.signature.as_mut().unwrap().jws =
        signed(&data_entry("urn:air:example.com:fixture:o", "2"), SIGNER)
            .signature
            .unwrap()
            .jws;
    let tree = world::tree(&world::catalog(vec![with_manifests(e, vec![m])]));
    let report = run(&tree, &world::docs(&[SIGNER]), 0);
    let r = first(&report);
    assert!(matches!(
        r.signature,
        SignatureOutcome::Rejected(Reason::Signature(_))
    ));
    assert!(std::ptr::eq(
        r.manifest,
        &tree.document.entries[0].trust_manifests.as_ref().unwrap()[0]
    ));
    assert!(std::ptr::eq(report.catalogs[0].catalog, &tree.document));
}

#[test]
fn entry_metadata_is_outside_manifest_authentication() {
    let mutations: Vec<(&str, Mutation<CatalogEntry>)> = vec![
        (
            "publisher",
            Box::new(|e| {
                e.publisher = Some(Publisher::new("did:web:evil.example".into(), "Evil".into()))
            }),
        ),
        (
            "extensions",
            Box::new(|e| {
                let mut x = Members::default();
                x.insert(
                    "com.example.policy".into(),
                    Json::String("https://example.com/p".into()),
                );
                e.extensions = Some(x);
            }),
        ),
        (
            "unknown member",
            Box::new(|e| {
                e.unrecognized
                    .insert("x-extra".into(), Json::String("v".into()));
            }),
        ),
    ];
    for (name, mutate) in mutations {
        let mut e = entry();
        let m = signed(&e, SIGNER);
        e.trust_manifests = Some(vec![m]);
        let signed_catalog = sign_root(world::catalog(vec![e]), SIGNER);

        let mut changed: AiCatalog = signed_catalog.clone();
        mutate(&mut changed.entries[0]);

        let docs = world::docs(&[SIGNER]);
        let before_tree = world::tree(&signed_catalog);
        let before = run(&before_tree, &docs, 0);
        let after_tree = world::tree(&changed);
        let after = run(&after_tree, &docs, 0);

        assert!(matches!(
            before.catalogs[0].signature,
            SignatureOutcome::Accepted(_)
        ));
        assert_eq!(single(&before), single(&after), "{name}");
        assert!(
            matches!(first(&after).signature, SignatureOutcome::Accepted(_)),
            "{name}"
        );
        assert!(
            matches!(
                after.catalogs[0].signature,
                SignatureOutcome::Rejected(Reason::Signature(_))
            ),
            "{name}: {:?}",
            after.catalogs[0].signature
        );
    }
}

#[test]
fn unsigned_did_web_contributor_is_not_endorsed() {
    let e = with_manifests(entry(), vec![TrustManifest::new(SIGNER.to_string())]);
    let tree = world::tree(&world::catalog(vec![e]));
    let report = run(&tree, &world::docs(&[SIGNER]), 0);
    assert!(matches!(
        first(&report).signature,
        SignatureOutcome::Unsigned
    ));
    assert_eq!(single(&report), Endorsement::None);
}

#[test]
fn reliance_needs_signer_time_coverage_and_integrity() {
    let docs = world::docs(&[SIGNER]);
    let outcome = |e: CatalogEntry, m: TrustManifest, docs: &DidDocuments, now| {
        let tree = world::tree(&world::catalog(vec![with_manifests(e, vec![m])]));
        single(&run(&tree, docs, now))
    };
    let e = entry();
    let m = signed_at(&e, SIGNER, world::ISSUED, Some("2026-09-12T11:00:00Z"));
    assert_eq!(
        outcome(e.clone(), m.clone(), &docs, 0),
        Endorsement::Publisher
    );
    // No authenticated signer.
    assert_eq!(
        outcome(e.clone(), m.clone(), &world::docs(&[]), 0),
        Endorsement::None
    );
    // Outside the time window.
    assert_eq!(
        outcome(e.clone(), m.clone(), &docs, 7200),
        Endorsement::None
    );
    assert_eq!(outcome(e.clone(), m.clone(), &docs, -1), Endorsement::None);
    // No coverage.
    let mut moved = e.clone();
    moved.version = Some("9".into());
    assert_eq!(outcome(moved, m, &docs, 0), Endorsement::None);
    // No integrity.
    let mut wrong = e.clone();
    wrong.digest = Some(turul_ai_catalog_trust::compute("sha256", b"x").unwrap());
    let m = signed(&wrong, SIGNER);
    assert_eq!(outcome(wrong, m, &docs, 0), Endorsement::None);
}

#[test]
fn each_contributor_is_verified_on_its_own() {
    let e = entry();
    let good = signed(&e, GOOD);
    let bad = signed(&e, OTHER);
    let tree = world::tree(&world::catalog(vec![with_manifests(e, vec![good, bad])]));
    let report = run(&tree, &world::docs(&[GOOD]), 0);
    let m = &report.catalogs[0].entries[0].manifests;
    assert_eq!(m[0].endorsement(), Endorsement::Contributor);
    assert!(matches!(m[0].coverage, Some(Coverage::Covered)));
    assert!(matches!(
        m[1].signature,
        SignatureOutcome::Rejected(Reason::Resolve(None))
    ));
    assert!(m[1].coverage.is_none());
}

#[test]
fn did_web_outcomes_are_distinguished_from_unsupported_profiles() {
    let e = entry();
    let ok = signed(&e, GOOD);
    let mut private = signed(&e, OTHER);
    private.signature.as_mut().unwrap().profile = "urn:example:private".into();
    let tree = world::tree(&world::catalog(vec![with_manifests(e, vec![ok, private])]));
    let report = run(&tree, &world::docs(&[GOOD, OTHER]), 0);
    let m = &report.catalogs[0].entries[0].manifests;
    let SignatureOutcome::Accepted(a) = &m[0].signature else {
        panic!("{:?}", m[0].signature)
    };
    assert_eq!(a.object.signature().unwrap().profile, "did-web-v1");
    assert!(matches!(
        &m[1].signature,
        SignatureOutcome::Rejected(Reason::Unsupported(p)) if p == "urn:example:private"
    ));
}

#[test]
fn signer_without_document_is_resolve_none() {
    let e = entry();
    let m = signed(&e, SIGNER);
    let tree = world::tree(&world::catalog(vec![with_manifests(e, vec![m])]));
    let report = run(&tree, &world::docs(&[]), 0);
    assert!(matches!(
        first(&report).signature,
        SignatureOutcome::Rejected(Reason::Resolve(None))
    ));
    let mut failed = DidDocuments::new();
    failed.insert(
        world::root(SIGNER),
        Err(turul_ai_catalog_trust::ResolveError::IdMismatch),
    );
    let report = run(&tree, &failed, 0);
    assert!(matches!(
        first(&report).signature,
        SignatureOutcome::Rejected(Reason::Resolve(Some(_)))
    ));
}

#[test]
fn nested_catalog_entries_get_outcomes() {
    let inner_entry = entry();
    let inner = world::catalog(vec![with_manifests(
        inner_entry.clone(),
        vec![signed(&inner_entry, SIGNER)],
    )]);
    let child_body = inner.to_json().to_canonical_vec().unwrap();
    let server = Server::start(vec![Reply::ok("application/ai-catalog+json", &child_body)]);

    let mut nested = world::url_entry(
        "urn:air:example.com:fixture:child",
        &server.url("/child.json"),
        &child_body,
    );
    nested.artifact_type = ArtifactType::new("application/ai-catalog+json");
    let root = world::catalog(vec![nested]);
    let tree = world::tree(&root);
    let report = run(&tree, &world::docs(&[SIGNER]), 0);

    assert_eq!(report.catalogs.len(), 2);
    assert!(report.catalogs[0].base.is_none());
    assert!(
        report.catalogs[1]
            .base
            .is_some_and(|b| b.ends_with("/child.json"))
    );
    let m = &report.catalogs[1].entries[0].manifests[0];
    assert_eq!(m.endorsement(), Endorsement::Publisher);
}

#[test]
fn inline_nested_catalog_entries_get_no_outcome() {
    let inner_entry = data_entry("urn:air:example.com:fixture:inner", r#"{"i":1}"#);
    let inner = world::catalog(vec![with_manifests(
        inner_entry.clone(),
        vec![signed(&inner_entry, SIGNER)],
    )]);
    let data = String::from_utf8(inner.to_json().to_canonical_vec().unwrap()).unwrap();
    let mut container = data_entry(ID, &data);
    container.artifact_type = ArtifactType::new("application/ai-catalog+json");
    let m = signed(&container, SIGNER);
    let tree = world::tree(&world::catalog(vec![with_manifests(container, vec![m])]));
    let report = run(&tree, &world::docs(&[SIGNER]), 0);

    assert_eq!(report.catalogs.len(), 1);
    assert_eq!(report.catalogs[0].entries.len(), 1);
    assert_eq!(single(&report), Endorsement::Publisher);
    assert!(matches!(
        report.catalogs[0].entries[0].manifests[0].coverage,
        Some(Coverage::Covered)
    ));
}

#[test]
fn failing_manifest_leaves_other_endorsement_intact() {
    let e = entry();
    let good = signed(&e, GOOD);
    let mut bad = signed(&e, OTHER);
    bad.signature.as_mut().unwrap().profile = "x".into();
    for order in [vec![good.clone(), bad.clone()], vec![bad, good]] {
        let tree = world::tree(&world::catalog(vec![with_manifests(e.clone(), order)]));
        let report = run(&tree, &world::docs(&[GOOD, OTHER]), 0);
        let endorsed = report.catalogs[0].entries[0]
            .manifests
            .iter()
            .filter(|m| m.endorsement() != Endorsement::None)
            .count();
        assert_eq!(endorsed, 1);
    }
}

#[test]
fn failing_entry_leaves_other_entries_intact() {
    let a = entry();
    let b = data_entry("urn:air:example.com:fixture:b", "2");
    let ma = signed(&a, SIGNER);
    let mut mb = signed(&b, SIGNER);
    mb.signature.as_mut().unwrap().jws = ma.signature.as_ref().unwrap().jws.clone();
    let tree = world::tree(&world::catalog(vec![
        with_manifests(a, vec![ma]),
        with_manifests(b, vec![mb]),
    ]));
    let report = run(&tree, &world::docs(&[SIGNER]), 0);
    let e = &report.catalogs[0].entries;
    assert_eq!(e[0].manifests[0].endorsement(), Endorsement::Publisher);
    assert_eq!(e[1].manifests[0].endorsement(), Endorsement::None);
    assert!(e[1].manifests[0].coverage.is_none());
    let _ = CoverageGap::NotAManifest;
}

#[test]
fn verify_is_resolve_then_verify_resolved() {
    let src = include_str!("../src/report.rs");
    let start = src.find("pub fn verify<").unwrap();
    let body = &src[start..];
    assert!(body.contains("collect_signers(tree, policy, &mut signers)"));
    assert!(body.contains("resolve(&s, &policy.fetch)"));
    assert!(body.contains("verify_resolved(tree, &docs, policy, now)"));
    assert!(!body.contains("verify_artifact") && !body.contains("verify_signature"));
}

#[test]
fn report_has_no_conformance_level() {
    let src = include_str!("../src/report.rs");
    for word in ["ConformanceLevel", "Conformance", "Trusted", "level"] {
        assert!(!src.contains(word), "{word}");
    }
    let tree = world::tree(&world::catalog(vec![]));
    let report = run(&tree, &world::docs(&[]), 0);
    let TrustReport { catalogs, .. } = report;
    assert_eq!(catalogs.len(), 1);
}
