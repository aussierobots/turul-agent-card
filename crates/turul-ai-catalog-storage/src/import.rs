use std::fmt;

use turul_ai_catalog::json::JsonPointer;
use turul_ai_catalog::model::AiCatalog;

use crate::ids::{CatalogId, Etag};
use crate::records::{EntryRecord, HostRecord, RecordError};
use crate::store::{CatalogStore, Condition, StoreError};

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImportError {
    /// The document carries a catalog signature, which the store cannot keep valid.
    Signed,
    /// A top-level member the store cannot hold, by JSON pointer.
    Unstorable {
        pointer: String,
    },
    /// A member failed the record gate.
    Record(RecordError),
    Store(StoreError),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signed => f.write_str("a signed catalog cannot be stored"),
            Self::Unstorable { pointer } => write!(f, "the store cannot hold {pointer}"),
            Self::Record(e) => write!(f, "{e}"),
            Self::Store(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ImportError {}

/// The one path from a catalog document to records: refuses a signed document or one with
/// top-level members the store cannot hold, gates the host and every entry, then writes them with
/// `Condition::Any`. Not atomic across records; re-running converges. Returns the host's version
/// (if any) followed by each entry's.
pub async fn import(
    store: &dyn CatalogStore,
    catalog: &CatalogId,
    document: &AiCatalog,
) -> Result<Vec<Etag>, ImportError> {
    if document.signature.is_some() || document.additional_signatures.is_some() {
        return Err(ImportError::Signed);
    }
    let root = JsonPointer::root();
    if document.extensions.is_some() {
        return Err(ImportError::Unstorable {
            pointer: root.child("extensions").to_string(),
        });
    }
    if let Some((name, _)) = document.unrecognized.iter().next() {
        return Err(ImportError::Unstorable {
            pointer: root.child(name).to_string(),
        });
    }
    let host = document
        .host
        .clone()
        .map(HostRecord::new)
        .transpose()
        .map_err(ImportError::Record)?;
    let entries = document
        .entries
        .iter()
        .cloned()
        .map(EntryRecord::new)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ImportError::Record)?;
    let mut etags = Vec::new();
    if let Some(host) = host {
        etags.push(
            store
                .put_host(catalog, &host, Condition::Any)
                .await
                .map_err(ImportError::Store)?,
        );
    }
    for record in &entries {
        etags.push(
            store
                .put_entry(catalog, record, Condition::Any)
                .await
                .map_err(ImportError::Store)?,
        );
    }
    Ok(etags)
}
