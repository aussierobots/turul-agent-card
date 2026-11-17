use std::fmt;
use std::time::SystemTime;

use turul_ai_catalog::canonical::{CanonicalError, to_canonical_vec};
use turul_ai_catalog::identity::EntryKey;
use turul_ai_catalog::model::AiCatalog;
use turul_ai_catalog::timestamp::Timestamp;
use turul_ai_catalog::validate::{Report, validate};

use crate::ids::CatalogId;
use crate::provenance::Provenance;
use crate::records::EntryRecord;
use crate::store::{CatalogStore, StoreError};

/// The served document, what it left out and when it next changes by itself.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Assembled {
    /// The canonical JSON bytes of the catalog document.
    pub bytes: Vec<u8>,
    /// The validation report of the document; it has no error diagnostics.
    pub report: Report,
    /// Every listed entry record that is not in `bytes`, ordered by key, with the reason.
    pub omitted: Vec<Omission>,
    /// The earliest `expires_at` among the leases in `bytes`; `None` when it holds none.
    pub next_expiry: Option<Timestamp>,
}

/// An entry record that assembly left out of the served document.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Omission {
    /// The omitted entry's key.
    pub key: EntryKey,
    /// The provenance stored with the record.
    pub provenance: Provenance,
    /// Why it was left out.
    pub reason: OmissionReason,
}

/// Why an [`Omission`] was left out.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum OmissionReason {
    /// The record is a lease that ended at or before the assembly time.
    LeaseExpired {
        /// The instant the lease ended.
        expires_at: Timestamp,
    },
    /// The record fails validation as it stands now.
    Invalid(Report),
}

impl fmt::Display for Omission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "omitted {}", self.key.identifier().as_str())?;
        if let Some(version) = self.key.version() {
            write!(f, "@{version}")?;
        }
        write!(f, " (writer {}): ", self.provenance.writer())?;
        match &self.reason {
            OmissionReason::LeaseExpired { expires_at } => {
                write!(f, "lease expired at {expires_at}")
            }
            OmissionReason::Invalid(report) => {
                f.write_str("invalid:")?;
                for d in report
                    .diagnostics()
                    .iter()
                    .filter(|d| d.severity == turul_ai_catalog::validate::Severity::Error)
                {
                    write!(f, " {} at '{}';", d.code, d.path)?;
                }
                Ok(())
            }
        }
    }
}

/// Why [`assemble`] failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AssembleError {
    /// Reading the records failed.
    Store(StoreError),
    /// The assembled document has an error diagnostic.
    Invalid(Report),
    /// The assembled document cannot be written as RFC 8785 canonical JSON.
    NotIJson(CanonicalError),
}

impl fmt::Display for AssembleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(e) => write!(f, "{e}"),
            Self::Invalid(_) => f.write_str("the assembled catalog has error diagnostics"),
            Self::NotIJson(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AssembleError {}

impl From<StoreError> for AssembleError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// Builds the served catalog document of `catalog` at `now`, the one builder of it.
///
/// Takes the host record and every entry record. A lease is live while `now` is before its
/// `expires_at`; expired leases are left out. The rest are ordered by
/// [`EntryKey`], validated as one document, and written as canonical bytes. When that document has
/// error diagnostics, each entry that fails validation on its own is left out as
/// [`OmissionReason::Invalid`] and the document is validated again. Nothing is dropped silently:
/// every listed record is in `bytes` or in `omitted`. The document never carries a signature.
///
/// The result is a pure function of the committed records and `now`; this crate never reads a
/// clock.
///
/// # Errors
///
/// [`AssembleError::Store`] when reading the records fails; [`AssembleError::Invalid`] when the
/// document still has an error diagnostic after the per-entry pass (a conflict between entries,
/// where the specification does not say which to drop); [`AssembleError::NotIJson`] when it
/// cannot be written as canonical JSON.
///
/// # Examples
///
/// ```
/// use std::time::SystemTime;
/// use turul_ai_catalog_storage::{CatalogId, InMemoryCatalogStore, assemble};
///
/// # futures::executor::block_on(async {
/// let served = assemble(&InMemoryCatalogStore::new(), &CatalogId::new("empty")?, SystemTime::UNIX_EPOCH).await?;
/// assert!(!served.report.has_errors() && served.omitted.is_empty());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub async fn assemble(
    store: &dyn CatalogStore,
    catalog: &CatalogId,
    now: SystemTime,
) -> Result<Assembled, AssembleError> {
    let now = Timestamp::from_system_time(now);
    let host = store.get_host(catalog).await?;
    let mut omitted = Vec::new();
    let mut live: Vec<(EntryRecord, Provenance)> = Vec::new();
    for stored in store.list_entries(catalog).await? {
        match stored.provenance {
            Provenance::Lease { expires_at, .. } if now >= expires_at => omitted.push(Omission {
                key: stored.record.key(),
                provenance: stored.provenance,
                reason: OmissionReason::LeaseExpired { expires_at },
            }),
            provenance => live.push((stored.record, provenance)),
        }
    }
    live.sort_by_key(|(r, _)| r.key());
    let build = |live: &[(EntryRecord, Provenance)]| {
        let mut doc = AiCatalog::new(live.iter().map(|(r, _)| r.entry().clone()).collect());
        doc.host = host.as_ref().map(|h| h.record.host().clone());
        doc
    };
    let mut doc = build(&live);
    let mut report = validate(&doc);
    if report.has_errors() {
        let mut kept = Vec::with_capacity(live.len());
        for (record, provenance) in live {
            let own = validate(&AiCatalog::new(vec![record.entry().clone()]));
            if own.has_errors() {
                omitted.push(Omission {
                    key: record.key(),
                    provenance,
                    reason: OmissionReason::Invalid(own),
                });
            } else {
                kept.push((record, provenance));
            }
        }
        live = kept;
        doc = build(&live);
        report = validate(&doc);
        if report.has_errors() {
            return Err(AssembleError::Invalid(report));
        }
    }
    let bytes = to_canonical_vec(&doc).map_err(AssembleError::NotIJson)?;
    omitted.sort_by(|a, b| a.key.cmp(&b.key));
    let next_expiry = live
        .iter()
        .filter_map(|(_, p)| match p {
            Provenance::Lease { expires_at, .. } => Some(*expires_at),
            Provenance::Curated { .. } => None,
        })
        .min();
    Ok(Assembled {
        bytes,
        report,
        omitted,
        next_expiry,
    })
}
