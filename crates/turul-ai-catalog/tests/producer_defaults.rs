use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier, SPEC_VERSION};
use turul_ai_catalog::model::*;

fn s(x: &str) -> String {
    x.to_string()
}

// #producer-behavior: "MUST set `specVersion` to the version of this specification they implement",
// "SHOULD NOT set ... higher"
#[test]
fn new_catalog_sets_spec_version_1_0() {
    assert_eq!(SPEC_VERSION, "1.0");
    let c = AiCatalog::new(vec![]);
    assert_eq!(c.spec_version, SPEC_VERSION);
    assert_eq!(c.spec_version, "1.0");
    assert!(c.entries.is_empty());
}

// Constructors set the required members only
#[test]
fn constructors_set_only_required_members() {
    let c = AiCatalog::new(vec![]);
    assert!(c.host.is_none() && c.extensions.is_none() && c.signature.is_none());
    assert!(c.additional_signatures.is_none() && c.unrecognized.is_empty());

    let h = HostInfo::new(s("H"));
    assert_eq!(h.display_name, "H");
    assert!(h.identifier.is_none() && h.documentation_url.is_none() && h.logo_url.is_none());
    assert!(h.unrecognized.is_empty());

    let e = CatalogEntry::new(
        EntryIdentifier::new("i"),
        ArtifactType::new("t/x"),
        ArtifactSource::Url(s("https://example.com/a")),
    );
    assert_eq!(e.identifier.as_str(), "i");
    assert_eq!(e.artifact_type.as_str(), "t/x");
    assert_eq!(e.source, ArtifactSource::Url(s("https://example.com/a")));
    assert!(e.display_name.is_none() && e.version.is_none() && e.description.is_none());
    assert!(e.tags.is_none() && e.publisher.is_none() && e.digest.is_none());
    assert!(e.trust_manifests.is_none() && e.updated_at.is_none() && e.extensions.is_none());
    assert!(e.unrecognized.is_empty());

    let p = Publisher::new(s("i"), s("P"));
    assert!(p.identity_type.is_none() && p.unrecognized.is_empty());

    let m = TrustManifest::new(s("c"));
    assert_eq!(m.contributor, "c");
    assert!(m.subject.is_none() && m.signature.is_none() && m.additional_signatures.is_none());
    assert!(m.trust_schema.is_none() && m.attestations.is_none() && m.provenance.is_none());
    assert!(m.extensions.is_none() && m.unrecognized.is_empty());

    let sub = Subject::new(
        EntryIdentifier::new("i"),
        ArtifactType::new("t/x"),
        s("sha256:00"),
    );
    assert!(sub.version.is_none() && sub.unrecognized.is_empty());

    let sig = Signature::new(s("s"), s("p"), s("2026-09-12T10:00:00Z"), s("a..b"));
    assert!(sig.expires_at.is_none() && sig.unrecognized.is_empty());

    let ts = TrustSchema::new(s("i"), s("1"));
    assert!(
        ts.governance_uri.is_none()
            && ts.verification_methods.is_none()
            && ts.unrecognized.is_empty()
    );

    let a = Attestation::new(s("t"), s("u"));
    assert!(
        a.digest.is_none()
            && a.size.is_none()
            && a.description.is_none()
            && a.unrecognized.is_empty()
    );

    let pl = ProvenanceLink::new(s("r"), s("s"));
    assert!(pl.source_digest.is_none() && pl.registry_uri.is_none());
    assert!(pl.statement_uri.is_none() && pl.signature_ref.is_none() && pl.unrecognized.is_empty());
}

// #top-level-structure, #optional-members: "Producers SHOULD omit" additionalSignatures
#[test]
fn constructors_never_set_additional_signatures() {
    assert!(AiCatalog::new(vec![]).additional_signatures.is_none());
    assert!(TrustManifest::new(s("c")).additional_signatures.is_none());
}
