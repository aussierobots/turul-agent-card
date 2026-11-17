use std::fmt;

use async_trait::async_trait;
use turul_ai_catalog::identity::{EntryIdentifier, EntryKey};
use turul_ai_catalog::parse::ParseError;

use crate::ids::{CatalogId, Etag, RecordKey};
use crate::records::{EntryRecord, HostRecord};

/// The precondition of a write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Condition {
    Any,
    /// The record must not exist.
    Absent,
    /// The record must exist with this version.
    Matches(Etag),
}

/// A record and its version.
#[derive(Clone, Debug, PartialEq)]
pub struct Stored<R> {
    pub record: R,
    pub etag: Etag,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StoreError {
    /// `Absent` but present; `Matches` but absent or different.
    PreconditionFailed,
    /// A delete of a missing record with no expected version.
    NotFound,
    /// The entry's identifier already has stored entries of the other version kind: an
    /// unversioned entry beside versioned ones, or the reverse (#multi-version-entries: "When
    /// `version` is absent, `identifier` alone MUST be unique").
    VersionKindConflict { identifier: EntryIdentifier },
    /// A stored record that does not parse.
    Corrupt { key: RecordKey, error: ParseError },
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

/// Entry-level catalog records with ETag versions. Every backend guarantees: atomic conditional
/// writes; read-your-writes; reads return exactly the bytes written (`etag == Etag::of(bytes)`);
/// `list_entries` returns every committed entry record once, in no defined order; no operation
/// touches another [`CatalogId`]; at most one entry record per [`RecordKey`]; no identifier with
/// both unversioned and versioned entry records. `list_entries` and `get_host` are not one
/// snapshot.
#[async_trait]
pub trait CatalogStore: Send + Sync {
    fn backend_name(&self) -> &'static str;

    /// Fails `PreconditionFailed` when `condition` does not hold, then `VersionKindConflict` when
    /// the record is new and its identifier already has entries of the other version kind. The
    /// check and the write are one atomic step.
    async fn put_entry(
        &self,
        catalog: &CatalogId,
        record: &EntryRecord,
        condition: Condition,
    ) -> Result<Etag, StoreError>;

    async fn get_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
    ) -> Result<Option<Stored<EntryRecord>>, StoreError>;

    /// With `expected: Some(etag)`, fails `PreconditionFailed` when the record is absent or its
    /// version differs; with `None`, fails `NotFound` when absent.
    async fn delete_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
        expected: Option<&Etag>,
    ) -> Result<(), StoreError>;

    async fn list_entries(
        &self,
        catalog: &CatalogId,
    ) -> Result<Vec<Stored<EntryRecord>>, StoreError>;

    async fn put_host(
        &self,
        catalog: &CatalogId,
        record: &HostRecord,
        condition: Condition,
    ) -> Result<Etag, StoreError>;

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
