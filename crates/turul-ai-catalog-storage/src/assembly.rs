use std::fmt;

use turul_ai_catalog::canonical::{CanonicalError, to_canonical_vec};
use turul_ai_catalog::model::AiCatalog;
use turul_ai_catalog::validate::{Report, validate};

use crate::ids::CatalogId;
use crate::store::{CatalogStore, StoreError};

/// The served document: canonical bytes and the validation report (warnings only).
#[derive(Clone, Debug, PartialEq)]
pub struct Assembled {
    pub bytes: Vec<u8>,
    pub report: Report,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AssembleError {
    Store(StoreError),
    /// The assembled document has an error diagnostic.
    Invalid(Report),
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

/// The one builder of a served catalog: the host record and every entry record, entries ordered
/// by `EntryKey`, validated, then canonical bytes. The document never carries a signature.
pub async fn assemble(
    store: &dyn CatalogStore,
    catalog: &CatalogId,
) -> Result<Assembled, AssembleError> {
    let host = store.get_host(catalog).await?;
    let mut entries: Vec<_> = store
        .list_entries(catalog)
        .await?
        .into_iter()
        .map(|s| s.record.into_entry())
        .collect();
    entries.sort_by_key(|e| e.key());
    let mut doc = AiCatalog::new(entries);
    doc.host = host.map(|h| h.record.into_host());
    let report = validate(&doc);
    if report.has_errors() {
        return Err(AssembleError::Invalid(report));
    }
    let bytes = to_canonical_vec(&doc).map_err(AssembleError::NotIJson)?;
    Ok(Assembled { bytes, report })
}
