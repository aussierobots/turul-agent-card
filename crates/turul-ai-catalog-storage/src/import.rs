use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use turul_ai_catalog::identity::EntryKey;
use turul_ai_catalog::json::JsonPointer;
use turul_ai_catalog::model::AiCatalog;
use turul_ai_catalog::timestamp::Timestamp;

use crate::ids::{CatalogId, Etag, WriterId};
use crate::owned::{OwnedWriteError, WriteMode, write_owned};
use crate::provenance::Provenance;
use crate::records::{EntryRecord, HostRecord, RecordError};
use crate::store::{CatalogStore, Condition, StoreError};

/// How [`import`] treats what the catalog id already holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImportMode {
    /// Write the document's records as curated records of the writer, refusing a key held by a
    /// lease or by another writer's curated record.
    Merge,
    /// Make the catalog id hold exactly the document: overwrite records of any mode or writer,
    /// delete entries the document lacks, refuse while a lease is live at `now`.
    Replace {
        /// The instant a lease is judged live against.
        now: SystemTime,
    },
}

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
    /// [`ImportMode::Replace`] found leases that are live, ordered by key; nothing was written.
    LiveLeases {
        /// Each live lease's key and provenance.
        leases: Vec<(EntryKey, Provenance)>,
    },
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
            Self::LiveLeases { leases } => write!(
                f,
                "the catalog holds {} live lease(s), which a replace does not overwrite",
                leases.len()
            ),
        }
    }
}

impl std::error::Error for ImportError {}

/// Stores the host and every entry of `document` under `catalog`, the one path from a catalog
/// document to records.
///
/// Gates the host and every entry before the first write. In [`ImportMode::Merge`] it then writes
/// the host with [`Condition::Any`] and each entry as a curated record of `writer`. Not atomic
/// across records; re-running with the same writer converges. Returns the host's version (if any)
/// followed by each entry's.
///
/// `writer` is permanent configuration: in `Merge` an entry whose key holds a curated record of
/// another writer is refused, so changing it fails at the first entry the catalog already holds.
///
/// [`ImportMode::Replace`] makes the catalog id hold exactly the document, in this order. It
/// scans the stored entries, reading no more than their keys, versions and provenance, and
/// refuses with [`ImportError::LiveLeases`] before any write when a lease is live. It overwrites
/// the host (a document without one leaves the stored host), overwrites each document entry the
/// scan held, conditioned on its scanned version and without reading the old record, creates each
/// one the scan lacked, deletes each scanned entry the document lacks, conditioned on its scanned
/// version, and last creates the entries whose identifier changed version kind. A concurrent
/// assembly therefore never loses a document entry the id already held, and a record registered
/// or refreshed after the scan is never overwritten: its write fails with
/// [`StoreError::PreconditionFailed`] and the import stops. Not atomic; re-running converges.
///
/// # Errors
///
/// [`ImportError::Signed`] when the document carries a `signature` or `additionalSignatures`;
/// [`ImportError::Unstorable`] when it has top-level members the store cannot hold;
/// [`ImportError::Invalid`] when `validate` reports an error, before anything is written;
/// [`ImportError::Record`] when the host or an entry fails the record gate;
/// [`ImportError::Store`] when a write fails, [`StoreError::ModeConflict`] included when an
/// entry's key holds a lease; [`ImportError::OwnedByAnotherWriter`] when it holds a curated record
/// of another writer; [`ImportError::LiveLeases`] in `Replace` while a lease is live. In `Replace`,
/// [`StoreError::Corrupt`] when a stored record does not parse, and
/// [`StoreError::PreconditionFailed`] when a record to write or delete changed after the scan.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
/// use turul_ai_catalog::model::{AiCatalog, ArtifactSource, CatalogEntry};
/// use turul_ai_catalog_storage::{CatalogId, ImportMode, InMemoryCatalogStore, WriterId, import};
///
/// # futures::executor::block_on(async {
/// let entry = CatalogEntry::new(
///     EntryIdentifier::new("urn:air:example.com:a2a:research"),
///     ArtifactType::A2A_AGENT_CARD,
///     ArtifactSource::Url("https://agents.example.com/.well-known/agent-card.json".into()),
/// );
/// let store = InMemoryCatalogStore::new();
/// let catalog = AiCatalog::new(vec![entry]);
/// let writer = WriterId::new("operator")?;
/// let etags = import(&store, &CatalogId::new("main")?, &catalog, &writer, ImportMode::Merge).await?;
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
    mode: ImportMode,
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
    match mode {
        ImportMode::Merge => merge(store, catalog, host, &entries, writer).await,
        ImportMode::Replace { now } => replace(store, catalog, host, &entries, writer, now).await,
    }
}

async fn merge(
    store: &dyn CatalogStore,
    catalog: &CatalogId,
    host: Option<HostRecord>,
    entries: &[EntryRecord],
    writer: &WriterId,
) -> Result<Vec<Etag>, ImportError> {
    let mut etags = Vec::new();
    if let Some(host) = host {
        etags.push(
            store
                .put_host(catalog, &host, Condition::Any)
                .await
                .map_err(ImportError::Store)?,
        );
    }
    for record in entries {
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

async fn replace(
    store: &dyn CatalogStore,
    catalog: &CatalogId,
    host: Option<HostRecord>,
    entries: &[EntryRecord],
    writer: &WriterId,
    now: SystemTime,
) -> Result<Vec<Etag>, ImportError> {
    let scanned = store
        .scan_entries(catalog)
        .await
        .map_err(ImportError::Store)?;
    let now = Timestamp::from_system_time(now);
    let mut leases: Vec<(EntryKey, Provenance)> = scanned
        .iter()
        .filter_map(|s| match &s.provenance {
            Ok(p @ Provenance::Lease { expires_at, .. }) if now < *expires_at => {
                Some((s.key.clone(), p.clone()))
            }
            _ => None,
        })
        .collect();
    if !leases.is_empty() {
        leases.sort_by(|a, b| a.0.cmp(&b.0));
        return Err(ImportError::LiveLeases { leases });
    }

    let mut etags = Vec::new();
    if let Some(host) = host {
        etags.push(
            store
                .put_host(catalog, &host, Condition::Any)
                .await
                .map_err(ImportError::Store)?,
        );
    }
    let provenance = Provenance::Curated {
        writer: writer.clone(),
    };
    let mut order: Vec<usize> = (0..entries.len()).collect();
    order.sort_by_key(|&i| entries[i].key());
    let mut written: Vec<Option<Etag>> = vec![None; entries.len()];
    let mut deferred = Vec::new();
    // Each write is conditioned on what the scan saw: a key the scan held must still hold that
    // version, and a key it lacked must still be empty, so a record registered or refreshed after
    // the scan stops the import instead of being overwritten.
    let seen: BTreeMap<&EntryKey, &Etag> = scanned.iter().map(|s| (&s.key, &s.etag)).collect();
    for &i in &order {
        let condition = match seen.get(&entries[i].key()) {
            Some(&etag) => Condition::Overwrite(etag.clone()),
            None => Condition::Absent,
        };
        match store
            .put_entry(catalog, &entries[i], &provenance, condition)
            .await
        {
            Ok(etag) => written[i] = Some(etag),
            Err(StoreError::VersionKindConflict { .. }) => deferred.push(i),
            Err(e) => return Err(ImportError::Store(e)),
        }
    }
    let kept: BTreeSet<EntryKey> = entries.iter().map(EntryRecord::key).collect();
    for stale in scanned.iter().filter(|s| !kept.contains(&s.key)) {
        match store
            .delete_entry(catalog, &stale.key, Some(&stale.etag))
            .await
        {
            Ok(()) => {}
            // A record deleted since the scan fails the version condition as a changed one does;
            // only a record still present stops the import.
            Err(StoreError::PreconditionFailed) => {
                if !matches!(store.get_entry(catalog, &stale.key).await, Ok(None)) {
                    return Err(ImportError::Store(StoreError::PreconditionFailed));
                }
            }
            Err(e) => return Err(ImportError::Store(e)),
        }
    }
    for i in deferred {
        written[i] = Some(
            store
                .put_entry(catalog, &entries[i], &provenance, Condition::Absent)
                .await
                .map_err(ImportError::Store)?,
        );
    }
    etags.extend(written.into_iter().flatten());
    Ok(etags)
}
