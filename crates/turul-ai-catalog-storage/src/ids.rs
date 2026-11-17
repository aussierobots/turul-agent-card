use std::borrow::Cow;
use std::fmt;

use sha2::{Digest, Sha256};
use turul_ai_catalog::identity::{EntryIdentifier, EntryKey};
use turul_ai_catalog::json::Json;

use crate::provenance::{Provenance, ProvenanceError};
use crate::records::{EntryRecord, RecordError};

fn hex(bytes: &[u8]) -> String {
    use fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Whether `id` is 1 to 128 bytes of `[A-Za-z0-9._-]`, the grammar of [`CatalogId`] and
/// [`WriterId`].
fn is_id_text(id: &str) -> bool {
    (1..=128).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// A storage namespace naming one catalog document: 1 to 128 bytes of `[A-Za-z0-9._-]`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CatalogId(String);

impl CatalogId {
    /// Builds a catalog id from `id`.
    ///
    /// # Errors
    ///
    /// [`RecordError::InvalidCatalogId`] when `id` is not 1 to 128 bytes of `[A-Za-z0-9._-]`.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog_storage::CatalogId;
    ///
    /// assert_eq!(CatalogId::new("main")?.as_str(), "main");
    /// assert!(CatalogId::new("not valid").is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(id: impl Into<String>) -> Result<Self, RecordError> {
        let id = id.into();
        if is_id_text(&id) {
            Ok(Self(id))
        } else {
            Err(RecordError::InvalidCatalogId)
        }
    }

    /// Returns the id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CatalogId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The opaque, operator-assigned identity of whoever writes an entry: 1 to 128 bytes of
/// `[A-Za-z0-9._-]`, compared exactly. The library trusts nothing about it beyond what the
/// deployer authenticated, and never derives one from an address or an entry member.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct WriterId(String);

impl WriterId {
    /// Builds a writer id from `id`.
    ///
    /// # Errors
    ///
    /// [`RecordError::InvalidWriterId`] when `id` is not 1 to 128 bytes of `[A-Za-z0-9._-]`.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog_storage::WriterId;
    ///
    /// assert_eq!(WriterId::new("operator.1")?.as_str(), "operator.1");
    /// assert!(WriterId::new("10.0.0.1:443").is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(id: impl Into<String>) -> Result<Self, RecordError> {
        let id = id.into();
        if is_id_text(&id) {
            Ok(Self(id))
        } else {
            Err(RecordError::InvalidWriterId)
        }
    }

    /// Returns the id as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WriterId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The backend key of a record within a catalog: `"h"` for the host, `"e:"` and a fixed-length
/// digest of the entry key for an entry, `"g:"` and a fixed-length digest of the identifier for
/// the version-kind guard of an identifier.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordKey(Cow<'static, str>);

impl RecordKey {
    /// The key of the Host Info record.
    pub const HOST: RecordKey = RecordKey(Cow::Borrowed("h"));

    /// Prefix shared by every entry record key.
    pub const ENTRY_PREFIX: &'static str = "e:";

    /// Prefix shared by every guard key.
    pub const GUARD_PREFIX: &'static str = "g:";

    /// Returns `"e:"` and the lowercase hex SHA-256 of the canonical JSON array
    /// `[identifier, version]`, an absent version being `null`.
    ///
    /// # Errors
    ///
    /// [`RecordError::NotIJson`] when the key cannot be written as canonical JSON.
    pub fn entry(key: &EntryKey) -> Result<RecordKey, RecordError> {
        let version = match key.version() {
            Some(v) => Json::String(v.to_string()),
            None => Json::Null,
        };
        Self::digest_key(
            Self::ENTRY_PREFIX,
            vec![Json::String(key.identifier().as_str().to_string()), version],
        )
    }

    /// Returns `"g:"` and the lowercase hex SHA-256 of the canonical JSON array `[identifier]`: the
    /// key of the record that holds an identifier's version kind and entry count, so that one identifier
    /// never has both unversioned and versioned entries
    /// ([#multi-version-entries](https://ai-catalog.io/specification/#multi-version-entries)).
    ///
    /// # Errors
    ///
    /// [`RecordError::NotIJson`] when the key cannot be written as canonical JSON.
    pub fn guard(identifier: &EntryIdentifier) -> Result<RecordKey, RecordError> {
        Self::digest_key(
            Self::GUARD_PREFIX,
            vec![Json::String(identifier.as_str().to_string())],
        )
    }

    fn digest_key(prefix: &str, name: Vec<Json>) -> Result<RecordKey, RecordError> {
        let bytes = Json::Array(name)
            .to_canonical_vec()
            .map_err(RecordError::NotIJson)?;
        Ok(RecordKey(Cow::Owned(format!(
            "{prefix}{}",
            hex(&Sha256::digest(bytes))
        ))))
    }

    /// A key read back from a backend.
    #[cfg(any(feature = "sqlite", feature = "postgres", feature = "dynamodb"))]
    pub(crate) fn from_stored_name(name: String) -> RecordKey {
        RecordKey(Cow::Owned(name))
    }

    /// Returns the key as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns whether the key names an entry record rather than the host or a guard.
    pub fn is_entry(&self) -> bool {
        self.0.starts_with(Self::ENTRY_PREFIX)
    }
}

impl fmt::Display for RecordKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The version of one stored record: for the host, the lowercase hex SHA-256 of its record bytes;
/// for an entry, [`Etag::of_entry`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Etag(String);

impl Etag {
    /// Returns the version of `record_bytes`.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog_storage::Etag;
    ///
    /// assert_eq!(Etag::of(b"a"), Etag::of(b"a"));
    /// assert_ne!(Etag::of(b"a"), Etag::of(b"b"));
    /// assert_eq!(Etag::of(b"").as_str().len(), 64);
    /// ```
    pub fn of(record_bytes: &[u8]) -> Etag {
        Etag(hex(&Sha256::digest(record_bytes)))
    }

    /// Returns the version of an entry: the lowercase hex SHA-256 of the canonical JSON of the
    /// two-element array `[entry JSON, provenance JSON]`, so a change of writer or expiry changes
    /// the version.
    ///
    /// # Errors
    ///
    /// The [`ProvenanceError`] of [`Provenance::to_bytes`] when the provenance would not read
    /// back, so no version exists for an entry no store can hold.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
    /// use turul_ai_catalog::model::{ArtifactSource, CatalogEntry};
    /// use turul_ai_catalog_storage::{EntryRecord, Etag, Provenance, WriterId};
    ///
    /// let entry = CatalogEntry::new(
    ///     EntryIdentifier::new("urn:air:example.com:mcp:weather"),
    ///     ArtifactType::MCP_SERVER_CARD,
    ///     ArtifactSource::Url("https://example.com/card.json".into()),
    /// );
    /// let record = EntryRecord::new(entry)?;
    /// let a = Provenance::Curated { writer: WriterId::new("a")? };
    /// let b = Provenance::Curated { writer: WriterId::new("b")? };
    /// assert_ne!(Etag::of_entry(&record, &a)?, Etag::of_entry(&record, &b)?);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn of_entry(
        record: &EntryRecord,
        provenance: &Provenance,
    ) -> Result<Etag, ProvenanceError> {
        provenance.to_bytes()?;
        let preimage = Json::Array(vec![record.entry().to_json(), provenance.to_json()])
            .to_canonical_vec()
            .map_err(|_| ProvenanceError::NotCanonical)?;
        Ok(Etag::of(&preimage))
    }

    /// A version read back from a backend as written.
    #[cfg(any(feature = "sqlite", feature = "postgres", feature = "dynamodb"))]
    pub(crate) fn from_stored(etag: String) -> Etag {
        Etag(etag)
    }

    /// Returns the version as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Etag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
