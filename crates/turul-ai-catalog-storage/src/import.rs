use std::fmt;
use std::time::UNIX_EPOCH;

use turul_ai_catalog::json::JsonPointer;
use turul_ai_catalog::model::AiCatalog;

use crate::ids::{CatalogId, Etag, WriterId};
use crate::owned::{OwnedWriteError, WriteMode, write_owned};
use crate::records::{EntryRecord, HostRecord, RecordError};
use crate::store::{CatalogStore, Condition, StoreError};

/// Why [`import`] refused or failed to store a catalog document.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ImportError {
    /// The document carries a catalog `signature` or `additionalSignatures`, which the store
    /// cannot keep valid.
    Signed,
    /// A top-level member the store cannot hold, by JSON pointer.
    Unstorable {
        /// The JSON pointer of the member.
        pointer: String,
    },
    /// The document has error diagnostics, each as `code at pointer`; nothing was written.
    Invalid {
        /// The error diagnostics, in document order.
        errors: Vec<String>,
    },
    /// A member failed the record gate.
    Record(RecordError),
    /// A write to the store failed. A key held by a lease is
    /// [`StoreError::ModeConflict`].
    Store(StoreError),
    /// An entry's key holds a curated record of another writer.
    OwnedByAnotherWriter,
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Signed => f.write_str("a signed catalog cannot be stored"),
            Self::Unstorable { pointer } => write!(f, "the store cannot hold {pointer}"),
            Self::Invalid { errors } => write!(f, "the document is invalid: {}", errors.join("; ")),
            Self::Record(e) => write!(f, "{e}"),
            Self::Store(e) => write!(f, "{e}"),
            Self::OwnedByAnotherWriter => {
                f.write_str("an entry's key holds a curated record of another writer")
            }
        }
    }
}

impl std::error::Error for ImportError {}

/// Stores the host and every entry of `document` under `catalog`, the one path from a catalog
/// document to records.
///
/// Gates the host and every entry before the first write, then writes the host with
/// [`Condition::Any`] and each entry as a curated record of `writer`. Not atomic across records;
/// re-running with the same writer converges. Returns the host's version (if any) followed by each
/// entry's.
///
/// `writer` is permanent configuration: an entry whose key holds a curated record of another
/// writer is refused, so changing it fails at the first entry the catalog already holds.
///
/// # Errors
///
/// [`ImportError::Signed`] when the document carries a `signature` or `additionalSignatures`;
/// [`ImportError::Unstorable`] when it has top-level members the store cannot hold;
/// [`ImportError::Invalid`] when `validate` reports an error, before anything is written;
/// [`ImportError::Record`] when the host or an entry fails the record gate;
/// [`ImportError::Store`] when a write fails, [`StoreError::ModeConflict`] included when an
/// entry's key holds a lease; [`ImportError::OwnedByAnotherWriter`] when it holds a curated record
/// of another writer.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
/// use turul_ai_catalog::model::{AiCatalog, ArtifactSource, CatalogEntry};
/// use turul_ai_catalog_storage::{CatalogId, InMemoryCatalogStore, WriterId, import};
///
/// # futures::executor::block_on(async {
/// let entry = CatalogEntry::new(
///     EntryIdentifier::new("urn:air:example.com:a2a:research"),
///     ArtifactType::A2A_AGENT_CARD,
///     ArtifactSource::Url("https://agents.example.com/.well-known/agent-card.json".into()),
/// );
/// let store = InMemoryCatalogStore::new();
/// let catalog = AiCatalog::new(vec![entry]);
/// let etags = import(&store, &CatalogId::new("main")?, &catalog, &WriterId::new("operator")?).await?;
/// assert_eq!(etags.len(), 1);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub async fn import(
    store: &dyn CatalogStore,
    catalog: &CatalogId,
    document: &AiCatalog,
    writer: &WriterId,
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
    // One validator: a document the spec rejects (e.g. two entries with one entry key, which
    // would otherwise collapse into a single record) is refused before anything is written.
    let report = turul_ai_catalog::validate::validate(document);
    if report.has_errors() {
        return Err(ImportError::Invalid {
            errors: report
                .diagnostics()
                .iter()
                .filter(|d| d.severity == turul_ai_catalog::validate::Severity::Error)
                .map(|d| format!("{} at {}", d.code, d.path))
                .collect(),
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
            write_owned(
                store,
                catalog,
                record,
                writer,
                WriteMode::Curated,
                None,
                UNIX_EPOCH,
            )
            .await
            .map(|w| w.etag)
            .map_err(|e| match e {
                OwnedWriteError::Store(e) => ImportError::Store(e),
                OwnedWriteError::OwnedByAnotherWriter => ImportError::OwnedByAnotherWriter,
                OwnedWriteError::Contended => ImportError::Store(StoreError::Contended),
                OwnedWriteError::Publisher(_) | OwnedWriteError::ExpiryOutOfRange => {
                    unreachable!("import writes curated records and runs no verifier")
                }
            })?,
        );
    }
    Ok(etags)
}
