//! The bytes a detached JWS covers ([#signed-payload-and-jws-construction](https://ai-catalog.io/specification/#signed-payload-and-jws-construction), [#catalog-snapshot-coverage](https://ai-catalog.io/specification/#catalog-snapshot-coverage)).

use std::fmt;

use turul_ai_catalog::canonical::CanonicalError;
use turul_ai_catalog::json::{Json, JsonPointer};
use turul_ai_catalog::model::{AiCatalog, Members, TrustManifest};

/// Reports why a payload cannot be canonicalised.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PayloadError {
    /// A number or string is not I-JSON.
    NotIJson {
        /// Location of the offending value, relative to the signed object.
        pointer: JsonPointer,
    },
}

impl fmt::Display for PayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotIJson { pointer } => write!(f, "payload is not I-JSON at {pointer}"),
        }
    }
}

impl std::error::Error for PayloadError {}

impl From<CanonicalError> for PayloadError {
    fn from(e: CanonicalError) -> Self {
        match e {
            CanonicalError::NotIJson { pointer } => Self::NotIJson { pointer },
            // `CanonicalError` is `#[non_exhaustive]`; `NotIJson` is its only refusal.
            _ => unreachable!("CanonicalError has one variant"),
        }
    }
}

/// Returns the RFC 8785 (JCS) bytes of the manifest without its immediate `signature.jws` and
/// `additionalSignatures`.
///
/// # Errors
///
/// [`PayloadError::NotIJson`] when a number or string in the manifest is not I-JSON.
pub fn manifest_payload(manifest: &TrustManifest) -> Result<Vec<u8>, PayloadError> {
    payload_of(manifest.to_json())
}

/// Returns the RFC 8785 (JCS) bytes of the whole catalog snapshot without its immediate
/// `signature.jws` and `additionalSignatures`; nested manifests and their signatures stay.
///
/// # Errors
///
/// [`PayloadError::NotIJson`] when a number or string in the catalog is not I-JSON.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
/// use turul_ai_catalog::model::{AiCatalog, ArtifactSource, CatalogEntry};
/// use turul_ai_catalog_trust::catalog_payload;
///
/// let catalog = AiCatalog::new(vec![CatalogEntry::new(
///     EntryIdentifier::new("urn:air:example.com:a2a:research"),
///     ArtifactType::A2A_AGENT_CARD,
///     ArtifactSource::Url("https://agents.example.com/card.json".into()),
/// )]);
/// let bytes = catalog_payload(&catalog)?;
/// assert!(bytes.starts_with(br#"{"entries":"#));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn catalog_payload(catalog: &AiCatalog) -> Result<Vec<u8>, PayloadError> {
    payload_of(catalog.to_json())
}

fn payload_of(json: Json) -> Result<Vec<u8>, PayloadError> {
    let Json::Object(members) = json else {
        unreachable!("a manifest or catalog serialises to an object")
    };
    let mut out = Members::default();
    for (name, value) in members.iter() {
        match (name.as_str(), value) {
            ("additionalSignatures", _) => {}
            ("signature", Json::Object(sig)) => {
                let mut kept = Members::default();
                for (n, v) in sig.iter().filter(|(n, _)| n.as_str() != "jws") {
                    kept.insert(n.clone(), v.clone());
                }
                out.insert(name.clone(), Json::Object(kept));
            }
            _ => {
                out.insert(name.clone(), value.clone());
            }
        }
    }
    Ok(Json::Object(out).to_canonical_vec()?)
}
