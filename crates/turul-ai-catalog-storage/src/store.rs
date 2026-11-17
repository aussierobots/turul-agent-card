use std::fmt;

use async_trait::async_trait;
use turul_ai_catalog::identity::{EntryIdentifier, EntryKey};
use turul_ai_catalog::parse::ParseError;

use crate::ids::{CatalogId, Etag, RecordKey};
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
}

/// A record and its version.
#[derive(Clone, Debug, PartialEq)]
pub struct Stored<R> {
    /// The record as read from the backend.
    pub record: R,
    /// The version of the record, `Etag::of(record bytes)`.
    pub etag: Etag,
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
    /// A stored record that does not parse.
    Corrupt {
        /// The key of the record that failed to parse.
        key: RecordKey,
        /// The parse failure.
        error: ParseError,
    },
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
            Self::Corrupt { key, error } => write!(f, "corrupt record {key}: {error}"),
            Self::Backend(m) => write!(f, "backend error: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Entry-level catalog records with [`Etag`] versions.
///
/// Every backend guarantees: atomic conditional
/// writes; read-your-writes; reads return exactly the bytes written (`etag == Etag::of(bytes)`);
/// `list_entries` returns every committed entry record once, in no defined order; no operation
/// touches another [`CatalogId`]; at most one entry record per [`RecordKey`]; no identifier with
/// both unversioned and versioned entry records. `list_entries` and `get_host` are not one
/// snapshot.
#[async_trait]
pub trait CatalogStore: Send + Sync {
    /// Returns the backend's short name, such as `"in-memory"`.
    fn backend_name(&self) -> &'static str;

    /// Writes an entry record under `condition` and returns its new version.
    ///
    /// The condition check, the version-kind check and the write are one atomic step.
    ///
    /// # Errors
    ///
    /// [`StoreError::PreconditionFailed`] when `condition` does not hold;
    /// [`StoreError::VersionKindConflict`] when the record is new and its identifier already has
    /// entries of the other version kind; [`StoreError::Backend`] on a backend failure.
    async fn put_entry(
        &self,
        catalog: &CatalogId,
        record: &EntryRecord,
        condition: Condition,
    ) -> Result<Etag, StoreError>;

    /// Returns the entry record stored under `key` and its version, or `None` when absent.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when the stored bytes do not parse; [`StoreError::Backend`] on a
    /// backend failure.
    async fn get_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
    ) -> Result<Option<Stored<EntryRecord>>, StoreError>;

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

    /// Returns every entry record of `catalog`, in no defined order.
    ///
    /// # Errors
    ///
    /// [`StoreError::Corrupt`] when a stored record does not parse; [`StoreError::Backend`] on a
    /// backend failure.
    async fn list_entries(
        &self,
        catalog: &CatalogId,
    ) -> Result<Vec<Stored<EntryRecord>>, StoreError>;

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
}

pub(crate) fn key_error(e: crate::RecordError) -> StoreError {
    StoreError::Backend(e.to_string())
}

pub(crate) fn kind_conflict(record: &EntryRecord) -> StoreError {
    StoreError::VersionKindConflict {
        identifier: record.entry().identifier.clone(),
    }
}

pub(crate) fn corrupt(key: &RecordKey, error: turul_ai_catalog::parse::ParseError) -> StoreError {
    StoreError::Corrupt {
        key: key.clone(),
        error,
    }
}
