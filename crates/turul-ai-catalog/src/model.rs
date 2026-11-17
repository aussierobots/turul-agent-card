//! The AI Catalog document model. Field names are the spec names in snake case.

use std::collections::BTreeMap;

use crate::identity::{ArtifactType, EntryIdentifier, EntryKey, HostIdentifier, SPEC_VERSION};
use crate::json::Json;

/// Members a struct does not define, kept unchanged (#unrecognized-members).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Members(BTreeMap<String, Json>);

impl Members {
    pub fn get(&self, name: &str) -> Option<&Json> {
        self.0.get(name)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Json)> {
        self.0.iter()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn insert(&mut self, name: String, value: Json) -> Option<Json> {
        self.0.insert(name, value)
    }
}

/// Exactly one of `url` and `data` (#catalog-entry).
#[derive(Clone, Debug, PartialEq)]
pub enum ArtifactSource {
    Url(String),
    Data(Json),
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct AiCatalog {
    pub spec_version: String,
    pub host: Option<HostInfo>,
    pub entries: Vec<CatalogEntry>,
    pub signature: Option<Signature>,
    pub additional_signatures: Option<Json>,
    pub extensions: Option<Members>,
    pub unrecognized: Members,
}

impl AiCatalog {
    pub fn new(entries: Vec<CatalogEntry>) -> Self {
        Self {
            spec_version: SPEC_VERSION.to_string(),
            host: None,
            entries,
            signature: None,
            additional_signatures: None,
            extensions: None,
            unrecognized: Members::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HostInfo {
    pub display_name: String,
    pub identifier: Option<HostIdentifier>,
    pub documentation_url: Option<String>,
    pub logo_url: Option<String>,
    pub unrecognized: Members,
}

impl HostInfo {
    pub fn new(display_name: String) -> Self {
        Self {
            display_name,
            identifier: None,
            documentation_url: None,
            logo_url: None,
            unrecognized: Members::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CatalogEntry {
    pub identifier: EntryIdentifier,
    pub display_name: Option<String>,
    pub artifact_type: ArtifactType,
    pub source: ArtifactSource,
    pub version: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub publisher: Option<Publisher>,
    pub digest: Option<String>,
    pub trust_manifests: Option<Vec<TrustManifest>>,
    pub updated_at: Option<String>,
    pub extensions: Option<Members>,
    pub unrecognized: Members,
}

impl CatalogEntry {
    pub fn new(
        identifier: EntryIdentifier,
        artifact_type: ArtifactType,
        source: ArtifactSource,
    ) -> Self {
        Self {
            identifier,
            display_name: None,
            artifact_type,
            source,
            version: None,
            description: None,
            tags: None,
            publisher: None,
            digest: None,
            trust_manifests: None,
            updated_at: None,
            extensions: None,
            unrecognized: Members::default(),
        }
    }

    /// The one way to build an [`EntryKey`].
    pub fn key(&self) -> EntryKey {
        EntryKey::of(self)
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Publisher {
    pub identifier: String,
    pub display_name: String,
    pub identity_type: Option<String>,
    pub unrecognized: Members,
}

impl Publisher {
    pub fn new(identifier: String, display_name: String) -> Self {
        Self {
            identifier,
            display_name,
            identity_type: None,
            unrecognized: Members::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TrustManifest {
    pub contributor: String,
    pub subject: Option<Subject>,
    pub signature: Option<Signature>,
    pub additional_signatures: Option<Json>,
    pub trust_schema: Option<TrustSchema>,
    pub attestations: Option<Vec<Attestation>>,
    pub provenance: Option<Vec<ProvenanceLink>>,
    pub extensions: Option<Members>,
    pub unrecognized: Members,
}

impl TrustManifest {
    pub fn new(contributor: String) -> Self {
        Self {
            contributor,
            subject: None,
            signature: None,
            additional_signatures: None,
            trust_schema: None,
            attestations: None,
            provenance: None,
            extensions: None,
            unrecognized: Members::default(),
        }
    }

    /// `signature.signer` equals `contributor`, byte for byte (#contributor-identity); `false`
    /// without a signature.
    pub fn signer_matches_contributor(&self) -> bool {
        self.signature
            .as_ref()
            .is_some_and(|s| s.signer == self.contributor)
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Subject {
    pub identifier: EntryIdentifier,
    pub artifact_type: ArtifactType,
    pub digest: String,
    pub version: Option<String>,
    pub unrecognized: Members,
}

impl Subject {
    pub fn new(identifier: EntryIdentifier, artifact_type: ArtifactType, digest: String) -> Self {
        Self {
            identifier,
            artifact_type,
            digest,
            version: None,
            unrecognized: Members::default(),
        }
    }

    /// The subject that binds `entry`: `None` without a `digest`, otherwise the entry's
    /// `identifier`, `type`, `digest` and `version` (present exactly when the entry's is).
    pub fn for_entry(entry: &CatalogEntry) -> Option<Self> {
        let mut s = Self::new(
            entry.identifier.clone(),
            entry.artifact_type.clone(),
            entry.digest.clone()?,
        );
        s.version = entry.version.clone();
        Some(s)
    }

    /// The one structural binding (#subject-object, #entry-release-coverage): every member
    /// exactly equals the entry's.
    pub fn binds(&self, entry: &CatalogEntry) -> bool {
        self.mismatches(entry).is_empty()
    }

    /// The names of the members that differ from `entry`, in subject order.
    pub(crate) fn mismatches(&self, entry: &CatalogEntry) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.identifier != entry.identifier {
            out.push("identifier");
        }
        if self.artifact_type != entry.artifact_type {
            out.push("type");
        }
        if Some(&self.digest) != entry.digest.as_ref() {
            out.push("digest");
        }
        if self.version != entry.version {
            out.push("version");
        }
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Signature {
    pub signer: String,
    pub profile: String,
    pub issued_at: String,
    pub expires_at: Option<String>,
    pub jws: String,
    pub unrecognized: Members,
}

impl Signature {
    pub fn new(signer: String, profile: String, issued_at: String, jws: String) -> Self {
        Self {
            signer,
            profile,
            issued_at,
            expires_at: None,
            jws,
            unrecognized: Members::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TrustSchema {
    pub identifier: String,
    pub version: String,
    pub governance_uri: Option<String>,
    pub verification_methods: Option<Vec<String>>,
    pub unrecognized: Members,
}

impl TrustSchema {
    pub fn new(identifier: String, version: String) -> Self {
        Self {
            identifier,
            version,
            governance_uri: None,
            verification_methods: None,
            unrecognized: Members::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Attestation {
    pub attestation_type: String,
    pub uri: String,
    pub digest: Option<String>,
    pub size: Option<u64>,
    pub description: Option<String>,
    pub unrecognized: Members,
}

impl Attestation {
    pub fn new(attestation_type: String, uri: String) -> Self {
        Self {
            attestation_type,
            uri,
            digest: None,
            size: None,
            description: None,
            unrecognized: Members::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ProvenanceLink {
    pub relation: String,
    pub source_id: String,
    pub source_digest: Option<String>,
    pub registry_uri: Option<String>,
    pub statement_uri: Option<String>,
    pub signature_ref: Option<String>,
    pub unrecognized: Members,
}

impl ProvenanceLink {
    pub fn new(relation: String, source_id: String) -> Self {
        Self {
            relation,
            source_id,
            source_digest: None,
            registry_uri: None,
            statement_uri: None,
            signature_ref: None,
            unrecognized: Members::default(),
        }
    }
}
