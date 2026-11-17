use std::fmt;

use async_trait::async_trait;
use turul_ai_catalog::identity::{EntryIdentifier, EntryKey};
use turul_ai_catalog::parse::ParseError;

use crate::ids::{CatalogId, Etag, RecordKey};
use crate::provenance::{ModeKind, Provenance, ProvenanceError};
use crate::records::{EntryRecord, HostRecord};

/// The precondition of a write, checked atomically with the write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    /// The write always applies.
    Any,
    /// The record must not exist.
    Absent,
    /// The record must exist with this version.
    Matches(Etag),
    /// The record must exist with this version, as `Matches`; an entry write then replaces it
    /// whatever its mode and whether its provenance reads, and the old record is never read. A
    /// host write treats it as `Matches`.
    Overwrite(Etag),
}

/// A record and its version.
#[derive(Clone, Debug, PartialEq)]
pub struct Stored<R> {
    /// The record as read from the backend.
    pub record: R,
    /// The version of the record, `Etag::of(record bytes)`.
    pub etag: Etag,
}

/// An entry record, who wrote it, and its version.
#[derive(Clone, Debug, PartialEq)]
pub struct StoredEntry {
    /// The record as read from the backend.
    pub record: EntryRecord,
    /// The mode and writer the record was stored with.
    pub provenance: Provenance,
    /// The version of the entry, [`Etag::of_entry`] of the record and its provenance.
    pub etag: Etag,
}

/// One entry record as [`CatalogStore::scan_entries`] reports it: the key from its bytes, its
/// stored version, and its provenance or why that does not read.
#[derive(Clone, Debug, PartialEq)]
pub struct ScannedEntry {
    /// The key of the entry in the record bytes.
    pub key: EntryKey,
    /// The version as stored.
    pub etag: Etag,
    /// The provenance as read, or the reason it does not.
    pub provenance: Result<Provenance, ProvenanceError>,
}

/// How many items of a catalog id a backend holds, by kind of backend key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ItemCounts {
    /// Host Info items (key `"h"`).
    pub host: u64,
    /// Entry items (keys `"e:…"`).
    pub entries: u64,
    /// Version-kind guards (keys `"g:…"`).
    pub guards: u64,
    /// Items under any other key.
    pub other: u64,
}

impl ItemCounts {
    /// Returns the number of items of every kind.
    pub fn total(&self) -> u64 {
        self.host + self.entries + self.guards + self.other
    }

    /// Counts one item by its backend key.
    #[cfg(any(
        feature = "in-memory",
        feature = "sqlite",
        feature = "postgres",
        feature = "dynamodb"
    ))]
    pub(crate) fn add_key(&mut self, key: &str) {
        let kind = if key == RecordKey::HOST.as_str() {
            &mut self.host
        } else if key.starts_with(RecordKey::ENTRY_PREFIX) {
            &mut self.entries
        } else if key.starts_with(RecordKey::GUARD_PREFIX) {
            &mut self.guards
        } else {
            &mut self.other
        };
        *kind += 1;
    }
}

/// What [`CatalogStore::purge_catalog`] removed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PurgeReport {
    /// The items removed, by kind.
    pub removed: ItemCounts,
}

/// Why a [`CatalogStore`] operation failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreError {
    /// `Condition::Absent` but present; `Condition::Matches` but absent or different.
    PreconditionFailed,
    /// A delete of a missing record with no expected version.
    NotFound,
    /// The entry's identifier already has stored entries of the other version kind: an
    /// unversioned entry beside versioned ones, or the reverse (#multi-version-entries: "When
    /// `version` is absent, `identifier` alone MUST be unique").
    VersionKindConflict {
        /// The identifier whose stored entries have the other version kind.
        identifier: EntryIdentifier,
    },
    /// A put would replace a record stored in the other mode.
    ModeConflict {
        /// The mode of the stored record.
        stored: ModeKind,
    },
    /// An entry record whose provenance is missing or does not parse.
    CorruptProvenance {
        /// The key of the record whose provenance failed to read.
        key: RecordKey,
        /// The failure.
        error: ProvenanceError,
    },
    /// A stored record that does not parse.
    Corrupt {
        /// The key of the record that failed to parse.
        key: RecordKey,
        /// The parse failure.
        error: ParseError,
    },
    /// Concurrent writers conflicted with the write on every attempt.
    Contended,
    /// The backend's own error, as text.
    Backend(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PreconditionFailed => f.write_str("precondition failed"),
            Self::NotFound => f.write_str("record not found"),
            Self::VersionKindConflict { identifier } => write!(
                f,
                "{} already has stored entries of the other version kind",
                identifier.as_str()
            ),
            Self::ModeConflict { stored } => {
                write!(f, "the stored record is in {stored} mode")
            }
            Self::CorruptProvenance { key, error } => {
                write!(f, "corrupt provenance of record {key}: {error}")
            }
            Self::Corrupt { key, error } => write!(f, "corrupt record {key}: {error}"),
            Self::Contended => f.write_str("concurrent writers conflicted on every attempt"),
            Self::Backend(m) => write!(f, "backend error: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Entry-level catalog records with [`Etag`] versions.
///
/// Every backend guarantees: atomic conditional
/// writes; read-your-writes; reads return exactly the bytes written (a host's `etag == Etag::of(bytes)`);
/// `list_entries` returns every committed entry record once, in no defined order; no operation
/// touches another [`CatalogId`]; at most one entry record per [`RecordKey`]; no identifier with
/// both unversioned and versioned entry records; no entry record replaced by a put of the other
/// [`ModeKind`]. An entry's version is [`Etag::of_entry`], so it covers the provenance.
/// `list_entries` and `get_host` are not one snapshot.
#[async_trait]
pub trait CatalogStore: Send + Sync {
    /// Returns the backend's short name, such as `"in-memory"`.
    fn backend_name(&self) -> &'static str;

    /// Writes an entry record with its provenance under `condition` and returns its new version.
    ///
    /// The condition check, the mode check, the version-kind check and the write are one atomic
    /// step, checked in that order. A provenance whose bytes would not read back is refused
    /// before the store is touched.
    ///
    /// # Errors
    ///
    /// [`StoreError::CorruptProvenance`] when `provenance` would not read back, or the stored
    /// record's mode is missing or unknown; [`StoreError::PreconditionFailed`] when `condition`
    /// does not hold;
    /// [`StoreError::ModeConflict`] when the put would replace a record of the other mode;
    /// [`StoreError::VersionKindConflict`] when the record is new and its identifier already has
    /// entries of the other version kind; [`StoreError::Contended`] when concurrent transactions
    /// conflicted on every attempt; [`StoreError::Backend`] on a backend failure.
    async fn put_entry(
        &self,
        catalog: &CatalogId,
        record: &EntryRecord,
        provenance: &Provenance,
        condition: Condition,
    ) -> Result<Etag, StoreError>;

    /// Returns the entry stored under `key`, or `None` when absent.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the stored bytes do not parse;
    /// [`StoreError::CorruptProvenance`] when the provenance is missing or does not parse;
    /// [`StoreError::Backend`] on a backend failure.
    async fn get_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
    ) -> Result<Option<StoredEntry>, StoreError>;

    /// Deletes the entry record stored under `key`.
    ///
    /// # Errors
    ///
    /// With `expected: Some(etag)`, [`StoreError::PreconditionFailed`] when the record is absent
    /// or its version differs; with `None`, [`StoreError::NotFound`] when absent;
    /// [`StoreError::Backend`] on a backend failure.
    async fn delete_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
        expected: Option<&Etag>,
    ) -> Result<(), StoreError>;

    /// Returns every entry of `catalog`, in no defined order.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when a stored record does not parse;
    /// [`StoreError::CorruptProvenance`] when a stored provenance is missing or does not parse;
    /// [`StoreError::Backend`] on a backend failure.
    async fn list_entries(&self, catalog: &CatalogId) -> Result<Vec<StoredEntry>, StoreError>;

    /// Writes the Host Info record of `catalog` under `condition` and returns its new version.
    ///
    /// # Errors
    ///
    /// [`StoreError::PreconditionFailed`] when `condition` does not hold;
    /// [`StoreError::Backend`] on a backend failure.
    async fn put_host(
        &self,
        catalog: &CatalogId,
        record: &HostRecord,
        condition: Condition,
    ) -> Result<Etag, StoreError>;

    /// Returns the Host Info record of `catalog` and its version, or `None` when absent.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the stored bytes do not parse; [`StoreError::Backend`] on a
    /// backend failure.
    async fn get_host(&self, catalog: &CatalogId)
    -> Result<Option<Stored<HostRecord>>, StoreError>;

    /// Returns every entry record of `catalog` with its key, stored version and provenance, a
    /// provenance that does not read being a value in the result rather than a failure. Order is
    /// unspecified.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when record bytes do not parse; [`StoreError::Backend`] on a
    /// backend failure.
    async fn scan_entries(&self, catalog: &CatalogId) -> Result<Vec<ScannedEntry>, StoreError>;

    /// Counts the items a [`CatalogStore::purge_catalog`] would remove now, by kind of backend
    /// key, reading no record bytes, provenance, mode or guard value.
    ///
    /// # Errors
    ///
    /// [`StoreError::Backend`] on a backend failure.
    async fn count_catalog(&self, catalog: &CatalogId) -> Result<ItemCounts, StoreError>;

    /// Removes every item stored under `catalog`, whatever its key or contents, reading no record
    /// bytes, provenance, mode or guard value, and returns how many it removed by kind. Changes no
    /// other catalog. DynamoDB removes item by item, so a failure part-way leaves the items not
    /// yet removed and re-running converges.
    ///
    /// # Errors
    ///
    /// [`StoreError::Contended`] when an item stayed inside other writers' transactions on every
    /// attempt; [`StoreError::Backend`] on a backend failure.
    async fn purge_catalog(&self, catalog: &CatalogId) -> Result<PurgeReport, StoreError>;
}

/// What every backend shares: error builders, the read-back of an entry and the write-side
/// check of a provenance.
#[cfg(any(
    feature = "in-memory",
    feature = "sqlite",
    feature = "postgres",
    feature = "dynamodb"
))]
mod backend {
    use super::*;

    pub(crate) fn key_error(e: crate::RecordError) -> StoreError {
        StoreError::Backend(e.to_string())
    }

    pub(crate) fn kind_conflict(record: &EntryRecord) -> StoreError {
        StoreError::VersionKindConflict {
            identifier: record.entry().identifier.clone(),
        }
    }

    pub(crate) fn corrupt_provenance(key: &RecordKey, error: ProvenanceError) -> StoreError {
        StoreError::CorruptProvenance {
            key: key.clone(),
            error,
        }
    }

    /// Builds the entry read back from a backend: the record parsed, then the provenance, with the
    /// version computed from what was read.
    pub(crate) fn stored_entry(
        key: &RecordKey,
        record_bytes: Vec<u8>,
        provenance_bytes: Option<&[u8]>,
    ) -> Result<StoredEntry, StoreError> {
        let record = EntryRecord::from_stored(record_bytes).map_err(|e| corrupt(key, e))?;
        let provenance = Provenance::parse(
            provenance_bytes
                .ok_or(ProvenanceError::Missing)
                .map_err(|e| corrupt_provenance(key, e))?,
        )
        .map_err(|e| corrupt_provenance(key, e))?;
        let etag = Etag::of_entry(&record, &provenance).map_err(|e| corrupt_provenance(key, e))?;
        Ok(StoredEntry {
            record,
            provenance,
            etag,
        })
    }

    /// Builds the scan row of an entry read back from a backend: the key from the record bytes, the
    /// version as stored, and the provenance as it reads.
    pub(crate) fn scanned_entry(
        key: &RecordKey,
        record_bytes: Vec<u8>,
        etag: Etag,
        provenance_bytes: Option<&[u8]>,
    ) -> Result<ScannedEntry, StoreError> {
        let record = EntryRecord::from_stored(record_bytes).map_err(|e| corrupt(key, e))?;
        Ok(ScannedEntry {
            key: record.key(),
            etag,
            provenance: provenance_bytes
                .ok_or(ProvenanceError::Missing)
                .and_then(Provenance::parse),
        })
    }

    pub(crate) fn corrupt(
        key: &RecordKey,
        error: turul_ai_catalog::parse::ParseError,
    ) -> StoreError {
        StoreError::Corrupt {
            key: key.clone(),
            error,
        }
    }

    /// The provenance bytes and the entry version a put stores under `key`. A provenance whose bytes
    /// would not read back is [`StoreError::CorruptProvenance`], so no backend stores a record its own
    /// reads would refuse.
    pub(crate) fn entry_write(
        key: &RecordKey,
        record: &EntryRecord,
        provenance: &Provenance,
    ) -> Result<(Vec<u8>, Etag), StoreError> {
        let bytes = provenance
            .to_bytes()
            .map_err(|e| corrupt_provenance(key, e))?;
        let etag = Etag::of_entry(record, provenance).map_err(|e| corrupt_provenance(key, e))?;
        Ok((bytes, etag))
    }
}

#[cfg(any(
    feature = "in-memory",
    feature = "sqlite",
    feature = "postgres",
    feature = "dynamodb"
))]
pub(crate) use backend::*;
