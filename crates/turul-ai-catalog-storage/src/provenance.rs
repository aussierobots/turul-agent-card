use std::fmt;

use turul_ai_catalog::json::{Json, read};
use turul_ai_catalog::model::Members;
use turul_ai_catalog::timestamp::Timestamp;

use crate::ids::WriterId;

/// Which of the two entry modes a record was written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModeKind {
    /// Written by a runtime registration; it expires.
    Lease,
    /// Written by an operator; it does not expire.
    Curated,
}

impl ModeKind {
    /// The stored name of the mode.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Lease => "lease",
            Self::Curated => "curated",
        }
    }

    /// The mode named by a stored `mode` value.
    #[cfg(any(feature = "sqlite", feature = "postgres", feature = "dynamodb"))]
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        match name {
            "lease" => Some(Self::Lease),
            "curated" => Some(Self::Curated),
            _ => None,
        }
    }
}

impl fmt::Display for ModeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Who wrote an entry record and under which mode, stored beside the record bytes and covered by
/// the entry's [`Etag`](crate::Etag).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Provenance {
    /// A lease: the record is served until `expires_at`, and belongs to `writer`.
    Lease {
        /// The writer that owns the record.
        writer: WriterId,
        /// The instant the lease ends.
        expires_at: Timestamp,
    },
    /// A curated record: it does not expire, and belongs to `writer`.
    Curated {
        /// The writer that owns the record.
        writer: WriterId,
    },
}

/// Why stored provenance bytes were refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProvenanceError {
    /// The item holds no provenance.
    Missing,
    /// The bytes are not a JSON object.
    NotAnObject,
    /// The members are not exactly those of one mode.
    Members,
    /// `mode` is neither `"lease"` nor `"curated"`.
    Mode,
    /// `writer` is not a valid writer id.
    Writer,
    /// `expiresAt` is not an RFC 3339 date-time.
    ExpiresAt,
    /// The bytes are valid but not the canonical form.
    NotCanonical,
}

impl fmt::Display for ProvenanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Missing => "no provenance stored",
            Self::NotAnObject => "provenance is not a JSON object",
            Self::Members => "provenance members are not those of one mode",
            Self::Mode => "provenance mode is neither lease nor curated",
            Self::Writer => "provenance writer is not a valid writer id",
            Self::ExpiresAt => "provenance expiresAt is not an RFC 3339 date-time",
            Self::NotCanonical => "provenance is not in canonical form",
        })
    }
}

impl std::error::Error for ProvenanceError {}

impl Provenance {
    /// Returns the mode of the record.
    pub fn kind(&self) -> ModeKind {
        match self {
            Self::Lease { .. } => ModeKind::Lease,
            Self::Curated { .. } => ModeKind::Curated,
        }
    }

    /// Returns the writer that owns the record.
    pub fn writer(&self) -> &WriterId {
        match self {
            Self::Lease { writer, .. } | Self::Curated { writer } => writer,
        }
    }

    /// Returns the JSON object: `{"mode":"curated","writer":W}` or
    /// `{"expiresAt":T,"mode":"lease","writer":W}`, `T` being the RFC 3339 form of the instant.
    pub fn to_json(&self) -> Json {
        let mut members = Members::default();
        let mut set = |name: &str, value: String| {
            members.insert(name.to_string(), Json::String(value));
        };
        set("mode", self.kind().as_str().to_string());
        set("writer", self.writer().to_string());
        if let Self::Lease { expires_at, .. } = self {
            set("expiresAt", expires_at.to_string());
        }
        Json::Object(members)
    }

    /// Returns the stored bytes: the RFC 8785 form of [`Provenance::to_json`].
    ///
    /// # Errors
    ///
    /// The [`ProvenanceError`] that [`Provenance::parse`] would return for the bytes, when they
    /// would not read back: [`ProvenanceError::ExpiresAt`] for an instant outside the years 0000
    /// to 9999, which RFC 3339 cannot write. A store refuses such a provenance rather than keep a
    /// record its own reads would refuse.
    pub fn to_bytes(&self) -> Result<Vec<u8>, ProvenanceError> {
        let bytes = self.canonical()?;
        Provenance::parse(&bytes)?;
        Ok(bytes)
    }

    /// The RFC 8785 bytes of [`Provenance::to_json`]. A writer id and a timestamp are ASCII, so
    /// the canonical writer never refuses them.
    fn canonical(&self) -> Result<Vec<u8>, ProvenanceError> {
        self.to_json()
            .to_canonical_vec()
            .map_err(|_| ProvenanceError::NotCanonical)
    }

    /// Reads stored provenance bytes.
    ///
    /// # Errors
    ///
    /// [`ProvenanceError`] when the bytes are not exactly one of the two canonical shapes.
    pub fn parse(bytes: &[u8]) -> Result<Provenance, ProvenanceError> {
        let Ok(Json::Object(members)) = read(bytes) else {
            return Err(ProvenanceError::NotAnObject);
        };
        let text = |name: &str| match members.get(name) {
            Some(Json::String(s)) => Some(s.as_str()),
            _ => None,
        };
        let writer = |s: &str| WriterId::new(s).map_err(|_| ProvenanceError::Writer);
        let provenance = match text("mode") {
            Some("curated") if members.len() == 2 => Provenance::Curated {
                writer: writer(text("writer").ok_or(ProvenanceError::Members)?)?,
            },
            Some("lease") if members.len() == 3 => Provenance::Lease {
                writer: writer(text("writer").ok_or(ProvenanceError::Members)?)?,
                expires_at: Timestamp::parse(text("expiresAt").ok_or(ProvenanceError::Members)?)
                    .map_err(|_| ProvenanceError::ExpiresAt)?,
            },
            Some("lease" | "curated") => return Err(ProvenanceError::Members),
            _ => return Err(ProvenanceError::Mode),
        };
        if provenance.canonical()? == bytes {
            Ok(provenance)
        } else {
            Err(ProvenanceError::NotCanonical)
        }
    }
}
