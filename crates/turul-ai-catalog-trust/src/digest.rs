//! Artifact digests and integrity (#digest-format, #verifying-artifact-integrity).

use std::fmt;

use crate::payload::PayloadError;
use sha2::{Digest, Sha256, Sha384, Sha512};
use turul_ai_catalog::digest::DigestValue;
use turul_ai_catalog::json::JsonPointer;
use turul_ai_catalog::model::{ArtifactSource, CatalogEntry};
use turul_ai_catalog_client::{FetchError, FetchPolicy, fetch, resolve_reference};

/// What this crate cannot do.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Unsupported {
    /// A hash algorithm other than sha256, sha384 and sha512.
    Algorithm(String),
    /// An artifact `url` with the `data` scheme; inline content belongs in `data`.
    DataUri,
}

/// Why an artifact's integrity could not be established.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum IntegrityError {
    DigestMissing,
    DigestInvalid,
    Unsupported(Unsupported),
    /// A relative `url` without a base.
    Unresolvable,
    Fetch(FetchError),
    /// The artifact response status was not 200.
    Status(u16),
    /// Inline `data` that JCS cannot canonicalise.
    NotIJson {
        pointer: JsonPointer,
    },
    Mismatch {
        expected: String,
        actual: String,
    },
}

impl fmt::Display for IntegrityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DigestMissing => f.write_str("entry has no digest"),
            Self::DigestInvalid => f.write_str("entry digest is not a valid digest"),
            Self::Unsupported(Unsupported::Algorithm(a)) => {
                write!(f, "digest algorithm {a} is not supported")
            }
            Self::Unsupported(Unsupported::DataUri) => f.write_str("data: URLs are not supported"),
            Self::Unresolvable => f.write_str("relative url without a base"),
            Self::Fetch(e) => write!(f, "artifact fetch failed: {e}"),
            Self::Status(s) => write!(f, "artifact response status {s}"),
            Self::NotIJson { pointer } => write!(f, "inline data is not I-JSON at {pointer}"),
            Self::Mismatch { expected, actual } => {
                write!(f, "digest mismatch: expected {expected}, got {actual}")
            }
        }
    }
}

impl std::error::Error for IntegrityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Fetch(e) => Some(e),
            _ => None,
        }
    }
}

/// `<lowercase algorithm>:<lowercase hex>` of `bytes`; `algorithm` is matched ASCII
/// case-insensitively against sha256, sha384 and sha512.
pub fn compute(algorithm: &str, bytes: &[u8]) -> Result<String, IntegrityError> {
    let name = algorithm.to_ascii_lowercase();
    let raw: Vec<u8> = match name.as_str() {
        "sha256" => Sha256::digest(bytes).to_vec(),
        "sha384" => Sha384::digest(bytes).to_vec(),
        "sha512" => Sha512::digest(bytes).to_vec(),
        _ => return Err(IntegrityError::Unsupported(Unsupported::Algorithm(name))),
    };
    let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!("{name}:{hex}"))
}

/// Compares `entry.digest` with the digest of the entry's artifact bytes: its JCS `data`, or
/// the body retrieved from `url` (resolved against `base`, the containing catalog's fetched
/// URL). Only `entry.digest` is read.
///
/// Blocking: a `url` entry performs an HTTP request. Call through
/// `tokio::task::spawn_blocking` from async code.
pub fn verify_artifact(
    entry: &CatalogEntry,
    base: Option<&str>,
    policy: &FetchPolicy,
) -> Result<(), IntegrityError> {
    let text = entry
        .digest
        .as_deref()
        .ok_or(IntegrityError::DigestMissing)?;
    let digest = DigestValue::parse(text).map_err(|_| IntegrityError::DigestInvalid)?;
    let bytes = match &entry.source {
        ArtifactSource::Data(json) => {
            json.to_canonical_vec()
                .map_err(|e| match PayloadError::from(e) {
                    PayloadError::NotIJson { pointer } => IntegrityError::NotIJson { pointer },
                })?
        }
        ArtifactSource::Url(reference) => {
            let url =
                resolve_reference(base, reference).map_err(|_| IntegrityError::Unresolvable)?;
            if url.starts_with("data:") {
                return Err(IntegrityError::Unsupported(Unsupported::DataUri));
            }
            let fetched = fetch(&url, policy).map_err(IntegrityError::Fetch)?;
            if fetched.status != 200 {
                return Err(IntegrityError::Status(fetched.status));
            }
            fetched.body
        }
    };
    let name = text.split_once(':').map_or(text, |(n, _)| n);
    let actual = compute(name, &bytes)?;
    let expected = format!("{}:{}", name.to_ascii_lowercase(), digest.hex());
    if actual == expected {
        Ok(())
    } else {
        Err(IntegrityError::Mismatch { expected, actual })
    }
}
