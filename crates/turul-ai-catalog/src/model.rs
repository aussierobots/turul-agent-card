//! The AI Catalog document model. Field names are the spec names in snake case.
//!
//! Every struct except [`Members`] is `#[non_exhaustive]`: build one with its `new` constructor
//! and set the optional fields afterwards.

use std::collections::BTreeMap;

use crate::identity::{ArtifactType, EntryIdentifier, EntryKey, HostIdentifier, SPEC_VERSION};
use crate::json::Json;

/// Members a struct does not define, kept unchanged ([#unrecognized-members](https://ai-catalog.io/specification/#unrecognized-members)).
///
/// Also holds `extensions` objects, whose keys are namespaced by their authors.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Members(BTreeMap<String, Json>);

impl Members {
    /// Returns the value of member `name`.
    pub fn get(&self, name: &str) -> Option<&Json> {
        self.0.get(name)
    }

    /// Iterates the members in name order.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &Json)> {
        self.0.iter()
    }

    /// Returns the number of members.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether there are no members.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Sets member `name` to `value`, returning the value it replaced.
    pub fn insert(&mut self, name: String, value: Json) -> Option<Json> {
        self.0.insert(name, value)
    }
}

/// Exactly one of `url` and `data` ([#catalog-entry](https://ai-catalog.io/specification/#catalog-entry)).
#[derive(Clone, Debug, PartialEq)]
pub enum ArtifactSource {
    /// A link to the artifact (`url`).
    Url(String),
    /// The artifact embedded as JSON (`data`).
    Data(Json),
}

/// An AI Catalog document ([#top-level-structure](https://ai-catalog.io/specification/#top-level-structure)).
///
/// # Examples
///
/// ```
/// use turul_ai_catalog::model::AiCatalog;
///
/// let catalog = AiCatalog::new(Vec::new());
/// assert_eq!(catalog.spec_version, "1.0");
/// assert!(catalog.host.is_none());
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct AiCatalog {
    /// `specVersion`, as written.
    pub spec_version: String,
    /// `host`: who operates the catalog.
    pub host: Option<HostInfo>,
    /// `entries`: the catalogued artifacts.
    pub entries: Vec<CatalogEntry>,
    /// `signature` over the catalog.
    pub signature: Option<Signature>,
    /// `additionalSignatures`, kept as opaque JSON.
    pub additional_signatures: Option<Json>,
    /// `extensions`: namespaced members defined outside the specification.
    pub extensions: Option<Members>,
    /// Members the specification does not define for this object.
    pub unrecognized: Members,
}

impl AiCatalog {
    /// Builds a catalog of `entries` at [`SPEC_VERSION`], with every optional member absent.
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

/// Host Info: who operates the catalog ([#host-info](https://ai-catalog.io/specification/#host-info)).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HostInfo {
    /// `displayName`.
    pub display_name: String,
    /// `identifier`, never treated as verified.
    pub identifier: Option<HostIdentifier>,
    /// `documentationUrl`.
    pub documentation_url: Option<String>,
    /// `logoUrl`.
    pub logo_url: Option<String>,
    /// Members the specification does not define for this object.
    pub unrecognized: Members,
}

impl HostInfo {
    /// Builds Host Info with `display_name` and every optional member absent.
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

/// A catalog entry: one artifact the catalog lists ([#catalog-entry](https://ai-catalog.io/specification/#catalog-entry)).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct CatalogEntry {
    /// `identifier`.
    pub identifier: EntryIdentifier,
    /// `displayName`.
    pub display_name: Option<String>,
    /// `type`: the media type of the artifact.
    pub artifact_type: ArtifactType,
    /// `url` or `data`: where the artifact is.
    pub source: ArtifactSource,
    /// `version`, as written.
    pub version: Option<String>,
    /// `description`.
    pub description: Option<String>,
    /// `tags`.
    pub tags: Option<Vec<String>>,
    /// `publisher`.
    pub publisher: Option<Publisher>,
    /// `digest` of the artifact, as `algorithm:hex-value` text; see [`DigestValue`](crate::digest::DigestValue).
    pub digest: Option<String>,
    /// `trustManifests`.
    pub trust_manifests: Option<Vec<TrustManifest>>,
    /// `updatedAt`, as RFC 3339 text; see [`Timestamp`](crate::timestamp::Timestamp).
    pub updated_at: Option<String>,
    /// `extensions`: namespaced members defined outside the specification.
    pub extensions: Option<Members>,
    /// Members the specification does not define for this object.
    pub unrecognized: Members,
}

impl CatalogEntry {
    /// Builds an entry with its required members and every optional member absent.
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

    /// Returns the [`EntryKey`] of this entry; the one way to build one.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
    /// use turul_ai_catalog::model::{ArtifactSource, CatalogEntry};
    ///
    /// let mut entry = CatalogEntry::new(
    ///     EntryIdentifier::new("urn:air:example.com:a2a:research"),
    ///     ArtifactType::A2A_AGENT_CARD,
    ///     ArtifactSource::Url("https://agents.example.com/.well-known/agent-card.json".into()),
    /// );
    /// entry.version = Some("1.2.0".into());
    /// assert_eq!(entry.key().version(), Some("1.2.0"));
    /// ```
    pub fn key(&self) -> EntryKey {
        EntryKey::of(self)
    }
}

/// The publisher of an entry ([#publisher-object](https://ai-catalog.io/specification/#publisher-object)).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Publisher {
    /// `identifier`, stored verbatim.
    pub identifier: String,
    /// `displayName`.
    pub display_name: String,
    /// `identityType`, stored verbatim.
    pub identity_type: Option<String>,
    /// Members the specification does not define for this object.
    pub unrecognized: Members,
}

impl Publisher {
    /// Builds a publisher with its required members and no `identityType`.
    pub fn new(identifier: String, display_name: String) -> Self {
        Self {
            identifier,
            display_name,
            identity_type: None,
            unrecognized: Members::default(),
        }
    }
}

/// A Trust Manifest: one contributor's trust metadata about an entry's artifact ([#trust-manifest](https://ai-catalog.io/specification/#trust-manifest)).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TrustManifest {
    /// `contributor`: who makes the claims, an absolute URI.
    pub contributor: String,
    /// `subject`: the entry release the manifest covers.
    pub subject: Option<Subject>,
    /// `signature` over the manifest.
    pub signature: Option<Signature>,
    /// `additionalSignatures`, kept as opaque JSON.
    pub additional_signatures: Option<Json>,
    /// `trustSchema`.
    pub trust_schema: Option<TrustSchema>,
    /// `attestations`.
    pub attestations: Option<Vec<Attestation>>,
    /// `provenance`.
    pub provenance: Option<Vec<ProvenanceLink>>,
    /// `extensions`: namespaced members defined outside the specification.
    pub extensions: Option<Members>,
    /// Members the specification does not define for this object.
    pub unrecognized: Members,
}

impl TrustManifest {
    /// Builds a manifest by `contributor` with every optional member absent.
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

    /// Returns whether `signature.signer` equals `contributor` byte for byte
    /// ([#contributor-identity](https://ai-catalog.io/specification/#contributor-identity)); `false` without a signature.
    pub fn signer_matches_contributor(&self) -> bool {
        self.signature
            .as_ref()
            .is_some_and(|s| s.signer == self.contributor)
    }
}

/// The entry release a Trust Manifest covers ([#subject-object](https://ai-catalog.io/specification/#subject-object)).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Subject {
    /// `identifier` of the covered entry.
    pub identifier: EntryIdentifier,
    /// `type` of the covered entry.
    pub artifact_type: ArtifactType,
    /// `digest` of the covered entry, as `algorithm:hex-value` text.
    pub digest: String,
    /// `version` of the covered entry, when it has one.
    pub version: Option<String>,
    /// Members the specification does not define for this object.
    pub unrecognized: Members,
}

impl Subject {
    /// Builds a subject with its required members and no `version`.
    pub fn new(identifier: EntryIdentifier, artifact_type: ArtifactType, digest: String) -> Self {
        Self {
            identifier,
            artifact_type,
            digest,
            version: None,
            unrecognized: Members::default(),
        }
    }

    /// Returns the subject that binds `entry`: `None` without a `digest`, otherwise the entry's
    /// `identifier`, `type`, `digest` and `version` (present exactly when the entry's is).
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
    /// use turul_ai_catalog::model::{ArtifactSource, CatalogEntry, Subject};
    ///
    /// let mut entry = CatalogEntry::new(
    ///     EntryIdentifier::new("urn:air:example.com:mcp:weather"),
    ///     ArtifactType::MCP_SERVER_CARD,
    ///     ArtifactSource::Url("https://mcp.example.com/.well-known/mcp/server-card.json".into()),
    /// );
    /// assert!(Subject::for_entry(&entry).is_none());
    ///
    /// entry.digest = Some(format!("sha256:{}", "ab".repeat(32)));
    /// let subject = Subject::for_entry(&entry).ok_or("entry has a digest")?;
    /// assert!(subject.binds(&entry));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn for_entry(entry: &CatalogEntry) -> Option<Self> {
        let mut s = Self::new(
            entry.identifier.clone(),
            entry.artifact_type.clone(),
            entry.digest.clone()?,
        );
        s.version = entry.version.clone();
        Some(s)
    }

    /// Returns whether every member exactly equals the entry's: the one structural binding
    /// ([#subject-object](https://ai-catalog.io/specification/#subject-object), [#entry-release-coverage](https://ai-catalog.io/specification/#entry-release-coverage)).
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

/// A Signature object ([#signature-object](https://ai-catalog.io/specification/#signature-object)).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Signature {
    /// `signer`: an absolute URI.
    pub signer: String,
    /// `profile`: the signing profile identifier.
    pub profile: String,
    /// `issuedAt`, as RFC 3339 text.
    pub issued_at: String,
    /// `expiresAt`, as RFC 3339 text.
    pub expires_at: Option<String>,
    /// `jws`: the detached compact JWS, stored as written.
    pub jws: String,
    /// Members the specification does not define for this object.
    pub unrecognized: Members,
}

impl Signature {
    /// Builds a signature with its required members and no `expiresAt`.
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

/// The trust schema a manifest follows ([#trust-schema-object](https://ai-catalog.io/specification/#trust-schema-object)).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TrustSchema {
    /// `identifier`.
    pub identifier: String,
    /// `version`.
    pub version: String,
    /// `governanceUri`, stored verbatim.
    pub governance_uri: Option<String>,
    /// `verificationMethods`.
    pub verification_methods: Option<Vec<String>>,
    /// Members the specification does not define for this object.
    pub unrecognized: Members,
}

impl TrustSchema {
    /// Builds a trust schema with its required members and every optional member absent.
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

/// An attestation a manifest links to ([#attestation-object](https://ai-catalog.io/specification/#attestation-object)).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Attestation {
    /// `type`: the attestation type, such as `SOC2-Type2`.
    pub attestation_type: String,
    /// `uri` of the attestation.
    pub uri: String,
    /// `digest` of the attestation, as `algorithm:hex-value` text.
    pub digest: Option<String>,
    /// `size` of the attestation in bytes.
    pub size: Option<u64>,
    /// `description`.
    pub description: Option<String>,
    /// Members the specification does not define for this object.
    pub unrecognized: Members,
}

impl Attestation {
    /// Builds an attestation with its required members and every optional member absent.
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

/// A provenance link from an artifact to its source ([#provenance-link-object](https://ai-catalog.io/specification/#provenance-link-object)).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ProvenanceLink {
    /// `relation` between the artifact and its source.
    pub relation: String,
    /// `sourceId`.
    pub source_id: String,
    /// `sourceDigest`, as `algorithm:hex-value` text.
    pub source_digest: Option<String>,
    /// `registryUri`, stored verbatim.
    pub registry_uri: Option<String>,
    /// `statementUri`.
    pub statement_uri: Option<String>,
    /// `signatureRef`.
    pub signature_ref: Option<String>,
    /// Members the specification does not define for this object.
    pub unrecognized: Members,
}

impl ProvenanceLink {
    /// Builds a provenance link with its required members and every optional member absent.
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
