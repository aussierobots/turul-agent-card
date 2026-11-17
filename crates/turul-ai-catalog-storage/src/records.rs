use std::fmt;

use turul_ai_catalog::canonical::CanonicalError;
use turul_ai_catalog::identity::EntryKey;
use turul_ai_catalog::model::{AiCatalog, CatalogEntry, HostInfo};
use turul_ai_catalog::parse::{ParseError, parse_entry, parse_host};
use turul_ai_catalog::validate::{Report, validate};

use crate::ids::RecordKey;

/// Why a record or catalog id was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RecordError {
    /// The member has an error diagnostic.
    Invalid(Report),
    /// The member cannot be written as RFC 8785 canonical JSON.
    NotIJson(CanonicalError),
    /// A catalog id is not 1 to 128 bytes of `[A-Za-z0-9._-]`.
    InvalidCatalogId,
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(_) => f.write_str("the record has error diagnostics"),
            Self::NotIJson(e) => write!(f, "{e}"),
            Self::InvalidCatalogId => {
                f.write_str("catalog id must be 1 to 128 bytes of [A-Za-z0-9._-]")
            }
        }
    }
}

impl std::error::Error for RecordError {}

fn gate(catalog: &AiCatalog) -> Result<(), RecordError> {
    let report = validate(catalog);
    if report.has_errors() {
        Err(RecordError::Invalid(report))
    } else {
        Ok(())
    }
}

/// An entry that passed [`validate`] and canonical serialisation. The only way to build one is
/// [`EntryRecord::new`].
#[derive(Clone, Debug, PartialEq)]
pub struct EntryRecord {
    entry: CatalogEntry,
    bytes: Vec<u8>,
}

impl EntryRecord {
    /// Builds a record from `entry`, keeping its canonical JSON bytes.
    ///
    /// # Errors
    ///
    /// [`RecordError::Invalid`] when the entry has an error diagnostic;
    /// [`RecordError::NotIJson`] when it cannot be written as canonical JSON.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
    /// use turul_ai_catalog::model::{ArtifactSource, CatalogEntry};
    /// use turul_ai_catalog_storage::EntryRecord;
    ///
    /// let entry = CatalogEntry::new(
    ///     EntryIdentifier::new("urn:air:example.com:mcp:weather"),
    ///     ArtifactType::MCP_SERVER_CARD,
    ///     ArtifactSource::Url("https://example.com/.well-known/mcp/server-card.json".into()),
    /// );
    /// let record = EntryRecord::new(entry)?;
    /// assert!(!record.bytes().is_empty());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(entry: CatalogEntry) -> Result<Self, RecordError> {
        gate(&AiCatalog::new(vec![entry.clone()]))?;
        let bytes = entry
            .to_json()
            .to_canonical_vec()
            .map_err(RecordError::NotIJson)?;
        Ok(Self { entry, bytes })
    }

    /// A record read back from a backend: parsed with the format crate's entry reader, bytes kept
    /// as stored.
    pub(crate) fn from_stored(bytes: Vec<u8>) -> Result<Self, ParseError> {
        let entry = parse_entry(&bytes)?;
        Ok(Self { entry, bytes })
    }

    /// Returns the entry.
    pub fn entry(&self) -> &CatalogEntry {
        &self.entry
    }

    /// Consumes the record and returns the entry.
    pub fn into_entry(self) -> CatalogEntry {
        self.entry
    }

    /// Returns the entry's [`EntryKey`].
    pub fn key(&self) -> EntryKey {
        self.entry.key()
    }

    /// Returns the canonical record bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the backend [`RecordKey`] of the entry.
    ///
    /// # Errors
    ///
    /// [`RecordError::NotIJson`] when the key cannot be written as canonical JSON.
    pub fn record_key(&self) -> Result<RecordKey, RecordError> {
        RecordKey::entry(&self.key())
    }
}

/// Host Info that passed [`validate`] and canonical serialisation.
#[derive(Clone, Debug, PartialEq)]
pub struct HostRecord {
    host: HostInfo,
    bytes: Vec<u8>,
}

impl HostRecord {
    /// Builds a record from `host`, keeping its canonical JSON bytes.
    ///
    /// # Errors
    ///
    /// [`RecordError::Invalid`] when the host has an error diagnostic;
    /// [`RecordError::NotIJson`] when it cannot be written as canonical JSON.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog::model::HostInfo;
    /// use turul_ai_catalog_storage::HostRecord;
    ///
    /// let record = HostRecord::new(HostInfo::new("Example".into()))?;
    /// assert_eq!(record.host().display_name, "Example");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(host: HostInfo) -> Result<Self, RecordError> {
        let mut doc = AiCatalog::new(vec![]);
        doc.host = Some(host.clone());
        gate(&doc)?;
        let bytes = host
            .to_json()
            .to_canonical_vec()
            .map_err(RecordError::NotIJson)?;
        Ok(Self { host, bytes })
    }

    pub(crate) fn from_stored(bytes: Vec<u8>) -> Result<Self, ParseError> {
        let host = parse_host(&bytes)?;
        Ok(Self { host, bytes })
    }

    /// Returns the Host Info.
    pub fn host(&self) -> &HostInfo {
        &self.host
    }

    /// Consumes the record and returns the Host Info.
    pub fn into_host(self) -> HostInfo {
        self.host
    }

    /// Returns the canonical record bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}
