use std::borrow::Cow;
use std::fmt;

use sha2::{Digest, Sha256};
use turul_ai_catalog::identity::EntryKey;
use turul_ai_catalog::json::Json;

use crate::records::RecordError;

fn hex(bytes: &[u8]) -> String {
    use fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// A storage namespace naming one catalog document: 1 to 128 bytes of `[A-Za-z0-9._-]`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CatalogId(String);

impl CatalogId {
    pub fn new(id: impl Into<String>) -> Result<Self, RecordError> {
        let id = id.into();
        let ok = (1..=128).contains(&id.len())
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
        if ok {
            Ok(Self(id))
        } else {
            Err(RecordError::InvalidCatalogId)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CatalogId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The backend key of a record within a catalog: `"h"` for the host, `"e:"` and a fixed-length
/// digest of the entry key for an entry.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RecordKey(Cow<'static, str>);

impl RecordKey {
    pub const HOST: RecordKey = RecordKey(Cow::Borrowed("h"));

    /// Prefix shared by every entry record key.
    pub const ENTRY_PREFIX: &'static str = "e:";

    /// `"e:"` and the lowercase hex SHA-256 of the canonical JSON array `[identifier, version]`,
    /// an absent version being `null`.
    pub fn entry(key: &EntryKey) -> Result<RecordKey, RecordError> {
        let version = match key.version() {
            Some(v) => Json::String(v.to_string()),
            None => Json::Null,
        };
        let name = Json::Array(vec![
            Json::String(key.identifier().as_str().to_string()),
            version,
        ]);
        let bytes = name.to_canonical_vec().map_err(RecordError::NotIJson)?;
        Ok(RecordKey(Cow::Owned(format!(
            "{}{}",
            Self::ENTRY_PREFIX,
            hex(&Sha256::digest(bytes))
        ))))
    }

    /// A key read back from a backend.
    #[cfg(any(feature = "sqlite", feature = "postgres", feature = "dynamodb"))]
    pub(crate) fn from_stored_name(name: String) -> RecordKey {
        RecordKey(Cow::Owned(name))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_entry(&self) -> bool {
        self.0.starts_with(Self::ENTRY_PREFIX)
    }
}

impl fmt::Display for RecordKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The version of one stored record: the lowercase hex SHA-256 of its record bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Etag(String);

impl Etag {
    pub fn of(record_bytes: &[u8]) -> Etag {
        Etag(hex(&Sha256::digest(record_bytes)))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Etag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
