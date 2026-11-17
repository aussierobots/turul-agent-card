//! Runtime registration: who may write which entries, and the lease or curated record they leave.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use turul_ai_catalog::identity::{EntryKey, UrnAir, is_root_did_web_domain};
use turul_ai_catalog::model::CatalogEntry;

use crate::ids::{CatalogId, Etag, WriterId};
use crate::owned::{ATTEMPTS, OwnedWriteError, WriteMode, lease_expiry, write_owned};
use crate::provenance::{ModeKind, Provenance};
use crate::records::{EntryRecord, RecordError};
use crate::store::{CatalogStore, StoreError};

const URN_AIR_PREFIX: &str = "urn:air:";

/// Whether `text` starts with `urn:air:` in any ASCII case.
fn looks_like_urn_air(text: &str) -> bool {
    text.as_bytes()
        .get(..URN_AIR_PREFIX.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(URN_AIR_PREFIX.as_bytes()))
}

/// Why a [`RegistrationPolicy`] or [`WriterGrant`] was refused at construction.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PolicyError {
    /// A grant with neither publishers nor prefixes.
    EmptyGrant,
    /// A publisher that is not a root `did:web` domain.
    InvalidPublisher(String),
    /// An empty identifier prefix, which would admit every identifier.
    EmptyPrefix,
    /// A prefix starting with `urn:air:` in any ASCII case.
    UrnAirPrefix(String),
    /// A lease length of zero.
    ZeroLeaseTtl,
    /// A lease length so long that a lease written at the Unix epoch would end past year 9999,
    /// which RFC 3339 cannot write.
    LeaseTtlOutOfRange,
    /// Two grants for one writer.
    DuplicateWriter(WriterId),
    /// A grant requires a publisher signature and no verifier was given.
    VerifierMissing,
}

impl fmt::Display for PolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyGrant => f.write_str("a grant needs at least one publisher or prefix"),
            Self::InvalidPublisher(p) => write!(f, "{p:?} is not a root did:web domain"),
            Self::EmptyPrefix => f.write_str("an identifier prefix must not be empty"),
            Self::UrnAirPrefix(p) => write!(f, "prefix {p:?} would admit urn:air identifiers"),
            Self::ZeroLeaseTtl => f.write_str("the lease length must not be zero"),
            Self::LeaseTtlOutOfRange => {
                f.write_str("the lease length would end a lease past year 9999")
            }
            Self::DuplicateWriter(w) => write!(f, "two grants for writer {w}"),
            Self::VerifierMissing => {
                f.write_str("a grant requires a publisher signature and no verifier was given")
            }
        }
    }
}

impl std::error::Error for PolicyError {}

/// What one writer may register: `urn:air` identifiers under its publishers, other identifiers
/// under its prefixes, and whether each registration must carry a publisher signature.
///
/// Naming a publisher is the operator's assertion that the writer is that publisher or its
/// authorized delegate. Only the entry `identifier` is ever checked against it. Writers granted
/// the same publisher or prefix share its identifiers: each may add versions of an identifier
/// another registered, though never overwrite or remove another writer's record.
///
/// A grant permits leases only; [`WriterGrant::with_curated`] also permits curated entries, which
/// never expire and are not verified again after their write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WriterGrant {
    publishers: BTreeSet<String>,
    prefixes: Vec<String>,
    require_publisher_signature: bool,
    curated: bool,
}

impl WriterGrant {
    /// Builds a grant. Publishers are `urn:air` `{publisher}` components, compared byte for byte;
    /// prefixes admit non-`urn:air` identifiers that start with them, byte for byte.
    ///
    /// # Errors
    ///
    /// [`PolicyError::EmptyGrant`] when both are empty; [`PolicyError::InvalidPublisher`] when a
    /// publisher is not a lowercase ASCII root `did:web` domain; [`PolicyError::EmptyPrefix`] for
    /// an empty prefix; [`PolicyError::UrnAirPrefix`] for a prefix starting with `urn:air:` in any
    /// ASCII case.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog_storage::{PolicyError, WriterGrant};
    ///
    /// assert!(WriterGrant::new(["example.com".to_string()], [], false).is_ok());
    /// assert_eq!(WriterGrant::new([], [], false), Err(PolicyError::EmptyGrant));
    /// ```
    pub fn new(
        publishers: impl IntoIterator<Item = String>,
        prefixes: impl IntoIterator<Item = String>,
        require_publisher_signature: bool,
    ) -> Result<Self, PolicyError> {
        let publishers: BTreeSet<String> = publishers.into_iter().collect();
        let prefixes: Vec<String> = prefixes.into_iter().collect();
        if publishers.is_empty() && prefixes.is_empty() {
            return Err(PolicyError::EmptyGrant);
        }
        if let Some(p) = publishers.iter().find(|p| !is_root_did_web_domain(p)) {
            return Err(PolicyError::InvalidPublisher(p.clone()));
        }
        for prefix in &prefixes {
            if prefix.is_empty() {
                return Err(PolicyError::EmptyPrefix);
            }
            if looks_like_urn_air(prefix) {
                return Err(PolicyError::UrnAirPrefix(prefix.clone()));
            }
        }
        Ok(Self {
            publishers,
            prefixes,
            require_publisher_signature,
            curated: false,
        })
    }

    /// Also permits [`ModeRequest::Curated`]. Without it, a curated request is refused with
    /// [`RegistrationError::CuratedNotGranted`] before the store is read.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog_storage::WriterGrant;
    ///
    /// let grant = WriterGrant::new(["example.com".to_string()], [], false)?.with_curated();
    /// # Ok::<(), turul_ai_catalog_storage::PolicyError>(())
    /// ```
    #[must_use]
    pub fn with_curated(mut self) -> Self {
        self.curated = true;
        self
    }

    /// Checks `identifier` against the publishers and prefixes.
    fn admits(&self, identifier: &str) -> Result<(), RegistrationError> {
        if let Some(urn) = UrnAir::parse(identifier) {
            return if self.publishers.contains(urn.publisher()) {
                Ok(())
            } else {
                Err(RegistrationError::PublisherNotAllowed {
                    publisher: urn.publisher().to_string(),
                })
            };
        }
        if looks_like_urn_air(identifier) {
            return Err(RegistrationError::MalformedUrnAir);
        }
        if self.prefixes.iter().any(|p| identifier.starts_with(p)) {
            Ok(())
        } else {
            Err(RegistrationError::IdentifierNotAllowed)
        }
    }
}

/// The grants of every writer and the length of a lease.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrationPolicy {
    grants: BTreeMap<WriterId, WriterGrant>,
    lease_ttl: Duration,
}

impl RegistrationPolicy {
    /// Builds a policy. The operator sets `lease_ttl`; a writer cannot ask for another.
    ///
    /// # Errors
    ///
    /// [`PolicyError::ZeroLeaseTtl`] for a zero `lease_ttl`; [`PolicyError::LeaseTtlOutOfRange`]
    /// when a lease of that length from the Unix epoch would end past year 9999;
    /// [`PolicyError::DuplicateWriter`] when a writer appears twice.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use turul_ai_catalog_storage::{RegistrationPolicy, WriterGrant, WriterId};
    ///
    /// let grant = WriterGrant::new(["example.com".to_string()], [], false)?;
    /// let policy = RegistrationPolicy::new([(WriterId::new("billing")?, grant)], Duration::from_secs(300))?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(
        grants: impl IntoIterator<Item = (WriterId, WriterGrant)>,
        lease_ttl: Duration,
    ) -> Result<Self, PolicyError> {
        if lease_ttl.is_zero() {
            return Err(PolicyError::ZeroLeaseTtl);
        }
        if lease_expiry(UNIX_EPOCH, lease_ttl).is_none() {
            return Err(PolicyError::LeaseTtlOutOfRange);
        }
        let mut map = BTreeMap::new();
        for (writer, grant) in grants {
            if map.contains_key(&writer) {
                return Err(PolicyError::DuplicateWriter(writer));
            }
            map.insert(writer, grant);
        }
        Ok(Self {
            grants: map,
            lease_ttl,
        })
    }
}

/// The verifier's reason for refusing an entry, as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VerifierRefusal(pub String);

impl fmt::Display for VerifierRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for VerifierRefusal {}

/// Decides whether an entry carries proof that its `urn:air` publisher signed it. Storage
/// defines the seam and never depends on the crypto that implements it.
#[async_trait]
pub trait PublisherVerifier: Send + Sync {
    /// Accepts `entry`, or says why not. `now` is the registrar's clock reading.
    async fn verify_publisher(
        &self,
        entry: &CatalogEntry,
        now: SystemTime,
    ) -> Result<(), VerifierRefusal>;
}

/// The mode a registration asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeRequest {
    /// An entry that expires unless its writer refreshes it.
    Lease,
    /// An entry that stays until removed. Needs a grant made with [`WriterGrant::with_curated`].
    Curated,
}

/// What a registration left in the store.
#[derive(Clone, Debug, PartialEq)]
pub struct Registered {
    /// The version of the stored entry.
    pub etag: Etag,
    /// The provenance now stored: the writer and, for a lease, its expiry.
    pub provenance: Provenance,
    /// Whether a write committed; `false` when the stored entry already said what was asked.
    pub written: bool,
}

/// Why a registration or removal was refused.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum RegistrationError {
    /// The writer has no grant.
    UnknownWriter,
    /// A curated registration by a writer whose grant permits leases only.
    CuratedNotGranted,
    /// The entry failed the record gate.
    Record(RecordError),
    /// A `urn:air` identifier under a publisher the writer was not granted.
    PublisherNotAllowed {
        /// The identifier's publisher component.
        publisher: String,
    },
    /// An identifier that starts with `urn:air:` and is not a `urn:air` identifier.
    MalformedUrnAir,
    /// A non-`urn:air` identifier outside every prefix the writer was granted.
    IdentifierNotAllowed,
    /// The grant requires a publisher signature and the identifier is not `urn:air`.
    SignatureRequiresUrnAir,
    /// The key holds a record in the other mode.
    ModeConflict {
        /// The mode of the stored record.
        stored: ModeKind,
    },
    /// The key holds a record of another writer.
    OwnedByAnotherWriter,
    /// The verifier refused the entry.
    PublisherSignature(VerifierRefusal),
    /// The store refused the read or the write.
    Store(StoreError),
    /// Other writers changed the record on every attempt.
    Contended,
    /// The lease would end past year 9999, which RFC 3339 cannot write.
    ExpiryOutOfRange,
    /// Removal of a key that holds no record.
    NotFound,
}

impl fmt::Display for RegistrationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownWriter => f.write_str("the writer has no grant"),
            Self::CuratedNotGranted => f.write_str("the writer's grant permits leases only"),
            Self::Record(e) => write!(f, "{e}"),
            Self::PublisherNotAllowed { publisher } => {
                write!(f, "the writer may not register under publisher {publisher}")
            }
            Self::MalformedUrnAir => f.write_str("the identifier is a malformed urn:air"),
            Self::IdentifierNotAllowed => {
                f.write_str("the identifier is outside every prefix the writer was granted")
            }
            Self::SignatureRequiresUrnAir => {
                f.write_str("a publisher signature requires a urn:air identifier")
            }
            Self::ModeConflict { stored } => write!(f, "the stored entry is in {stored} mode"),
            Self::OwnedByAnotherWriter => f.write_str("the entry belongs to another writer"),
            Self::PublisherSignature(r) => write!(f, "publisher signature refused: {r}"),
            Self::Store(e) => write!(f, "{e}"),
            Self::Contended => f.write_str("other writers changed the entry on every attempt"),
            Self::ExpiryOutOfRange => f.write_str("the lease would end past year 9999"),
            Self::NotFound => f.write_str("no such entry"),
        }
    }
}

impl std::error::Error for RegistrationError {}

impl From<StoreError> for RegistrationError {
    fn from(e: StoreError) -> Self {
        match e {
            StoreError::ModeConflict { stored } => Self::ModeConflict { stored },
            StoreError::Contended => Self::Contended,
            e => Self::Store(e),
        }
    }
}

impl From<OwnedWriteError> for RegistrationError {
    fn from(e: OwnedWriteError) -> Self {
        match e {
            OwnedWriteError::Store(e) => e.into(),
            OwnedWriteError::OwnedByAnotherWriter => Self::OwnedByAnotherWriter,
            OwnedWriteError::Publisher(r) => Self::PublisherSignature(r),
            OwnedWriteError::Contended => Self::Contended,
            OwnedWriteError::ExpiryOutOfRange => Self::ExpiryOutOfRange,
        }
    }
}

/// Registers and removes entries of one catalog on behalf of writers the deployer has
/// authenticated.
///
/// It owns its store and verifier through [`Arc`]s, so it can live in a service's static state
/// and be shared across tasks.
///
/// The registrar constrains a writer only when that writer cannot reach the store without it: a
/// process holding store write credentials can bypass every check here.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
/// use std::time::{Duration, UNIX_EPOCH};
/// use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
/// use turul_ai_catalog::model::{ArtifactSource, CatalogEntry};
/// use turul_ai_catalog_storage::{
///     CatalogId, CatalogStore, InMemoryCatalogStore, ModeRequest, Registrar, RegistrationPolicy,
///     WriterGrant, WriterId,
/// };
///
/// # futures::executor::block_on(async {
/// let store: Arc<dyn CatalogStore> = Arc::new(InMemoryCatalogStore::new());
/// let writer = WriterId::new("weather")?;
/// let grant = WriterGrant::new(["example.com".to_string()], [], false)?;
/// let policy = RegistrationPolicy::new([(writer.clone(), grant)], Duration::from_secs(300))?;
/// let registrar = Registrar::new(store, CatalogId::new("main")?, policy, None)?;
/// let entry = CatalogEntry::new(
///     EntryIdentifier::new("urn:air:example.com:mcp:weather"),
///     ArtifactType::MCP_SERVER_CARD,
///     ArtifactSource::Url("https://mcp.example.com/mcp/server-card".into()),
/// );
/// let now = UNIX_EPOCH + Duration::from_secs(1_800_000_000);
/// let registered = registrar.register(&writer, entry, ModeRequest::Lease, now).await?;
/// assert!(registered.written);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct Registrar {
    store: Arc<dyn CatalogStore>,
    catalog: CatalogId,
    policy: RegistrationPolicy,
    verifier: Option<Arc<dyn PublisherVerifier>>,
}

impl Registrar {
    /// Builds a registrar for `catalog`.
    ///
    /// # Errors
    ///
    /// [`PolicyError::VerifierMissing`] when a grant requires a publisher signature and
    /// `verifier` is `None`.
    pub fn new(
        store: Arc<dyn CatalogStore>,
        catalog: CatalogId,
        policy: RegistrationPolicy,
        verifier: Option<Arc<dyn PublisherVerifier>>,
    ) -> Result<Registrar, PolicyError> {
        if verifier.is_none()
            && policy
                .grants
                .values()
                .any(|g| g.require_publisher_signature)
        {
            return Err(PolicyError::VerifierMissing);
        }
        Ok(Self {
            store,
            catalog,
            policy,
            verifier,
        })
    }

    /// Creates, updates or refreshes `entry` for `writer`.
    ///
    /// Checks run in this order and the first failure is returned: the writer has a grant that
    /// permits the requested mode; the entry passes the record gate; the identifier is within the grant; a required signature
    /// has a `urn:air` identifier to be about; the stored record, if any, is in the requested
    /// mode and the writer's; the verifier, when the grant requires a signature, accepts the
    /// entry; the conditional write. A request that local policy refuses does no I/O, and one
    /// the stored state refuses causes no verification.
    ///
    /// The verifier runs only when a write is about to commit: a refresh that is not yet due
    /// changes nothing and is returned with `written: false`. A lease is rewritten once less
    /// than three quarters of the lease length remain, and never to an earlier expiry.
    ///
    /// # Errors
    ///
    /// A [`RegistrationError`] naming the first check that failed.
    pub async fn register(
        &self,
        writer: &WriterId,
        entry: CatalogEntry,
        mode: ModeRequest,
        now: SystemTime,
    ) -> Result<Registered, RegistrationError> {
        let grant = self.grant(writer)?;
        if mode == ModeRequest::Curated && !grant.curated {
            return Err(RegistrationError::CuratedNotGranted);
        }
        let record = EntryRecord::new(entry).map_err(RegistrationError::Record)?;
        let identifier = record.entry().identifier.as_str();
        grant.admits(identifier)?;
        if grant.require_publisher_signature && UrnAir::parse(identifier).is_none() {
            return Err(RegistrationError::SignatureRequiresUrnAir);
        }
        let verifier = self
            .verifier
            .as_deref()
            .filter(|_| grant.require_publisher_signature);
        let mode = match mode {
            ModeRequest::Lease => WriteMode::Lease(self.policy.lease_ttl),
            ModeRequest::Curated => WriteMode::Curated,
        };
        let written = write_owned(
            &*self.store,
            &self.catalog,
            &record,
            writer,
            mode,
            verifier,
            now,
        )
        .await?;
        Ok(Registered {
            etag: written.etag,
            provenance: written.provenance,
            written: written.written,
        })
    }

    /// Removes the entry under `key` if `writer` owns it, in either mode. No signature is
    /// required: removal makes no claim about the artifact.
    ///
    /// Authority is the writer's current grant: one that no longer covers the identifier cannot
    /// remove it.
    ///
    /// # Errors
    ///
    /// [`RegistrationError::UnknownWriter`], the identifier refusals of [`Registrar::register`],
    /// [`RegistrationError::NotFound`] when the key holds no record,
    /// [`RegistrationError::OwnedByAnotherWriter`], [`RegistrationError::Contended`] when the
    /// entry changes under every attempt, and [`RegistrationError::Store`].
    pub async fn unregister(
        &self,
        writer: &WriterId,
        key: &EntryKey,
    ) -> Result<(), RegistrationError> {
        let grant = self.grant(writer)?;
        grant.admits(key.identifier().as_str())?;
        for _ in 0..ATTEMPTS {
            let stored = self
                .store
                .get_entry(&self.catalog, key)
                .await?
                .ok_or(RegistrationError::NotFound)?;
            if stored.provenance.writer() != writer {
                return Err(RegistrationError::OwnedByAnotherWriter);
            }
            match self
                .store
                .delete_entry(&self.catalog, key, Some(&stored.etag))
                .await
            {
                Ok(()) => return Ok(()),
                Err(StoreError::PreconditionFailed) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Err(RegistrationError::Contended)
    }

    fn grant(&self, writer: &WriterId) -> Result<&WriterGrant, RegistrationError> {
        self.policy
            .grants
            .get(writer)
            .ok_or(RegistrationError::UnknownWriter)
    }
}
