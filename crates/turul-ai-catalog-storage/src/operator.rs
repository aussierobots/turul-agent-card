use std::fmt;
use std::time::SystemTime;

use turul_ai_catalog::identity::EntryKey;
use turul_ai_catalog::timestamp::Timestamp;

use crate::ids::CatalogId;
use crate::provenance::Provenance;
use crate::store::{CatalogStore, StoreError};

/// Whether a stored entry is served at a given instant, by the rule of [`assemble`](crate::assemble).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiry {
    /// A curated entry; it never expires.
    Never,
    /// A lease that ends at the instant.
    Live(Timestamp),
    /// A lease that ended at or before the listing instant.
    Expired(Timestamp),
}

/// One stored entry as an operator sees it: its key, who wrote it, and whether it is served.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Registration {
    /// The entry's key.
    pub key: EntryKey,
    /// The mode and writer the entry was stored with.
    pub provenance: Provenance,
    /// The expiry state at the listing instant.
    pub expiry: Expiry,
}

impl fmt::Display for Registration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}  {}  {}  {}  ",
            self.key.identifier().as_str(),
            self.key.version().unwrap_or("-"),
            self.provenance.kind(),
            self.provenance.writer()
        )?;
        match self.expiry {
            Expiry::Never => f.write_str("no expiry"),
            Expiry::Live(t) => write!(f, "live until {t}"),
            Expiry::Expired(t) => write!(f, "expired at {t}"),
        }
    }
}

/// Why [`remove_registration`] failed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RemoveError {
    /// No entry is stored under the key.
    NotFound,
    /// Reading or deleting failed; a delete refused because the entry changed since it was read
    /// is [`StoreError::PreconditionFailed`].
    Store(StoreError),
}

impl fmt::Display for RemoveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("no entry is stored under that key"),
            Self::Store(StoreError::PreconditionFailed) => {
                f.write_str("the entry changed since it was read; nothing was removed")
            }
            Self::Store(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RemoveError {}

/// Lists every entry of `catalog` with its provenance and its expiry state at `now`, ordered by
/// [`EntryKey`]. A lease is live while `now` is before its `expires_at`.
///
/// # Errors
///
/// The [`StoreError`] of [`CatalogStore::list_entries`].
pub async fn list_registrations(
    store: &dyn CatalogStore,
    catalog: &CatalogId,
    now: SystemTime,
) -> Result<Vec<Registration>, StoreError> {
    let now = Timestamp::from_system_time(now);
    let mut rows: Vec<Registration> = store
        .list_entries(catalog)
        .await?
        .into_iter()
        .map(|stored| {
            let expiry = match &stored.provenance {
                Provenance::Curated { .. } => Expiry::Never,
                Provenance::Lease { expires_at, .. } if now >= *expires_at => {
                    Expiry::Expired(*expires_at)
                }
                Provenance::Lease { expires_at, .. } => Expiry::Live(*expires_at),
            };
            Registration {
                key: stored.record.key(),
                provenance: stored.provenance,
                expiry,
            }
        })
        .collect();
    rows.sort_by(|a, b| a.key.cmp(&b.key));
    Ok(rows)
}

/// Removes the entry stored under `key`, of either mode, and returns the provenance it had.
///
/// The delete is conditioned on the version read, so a refresh in between fails it with
/// [`StoreError::PreconditionFailed`] instead of removing a live lease.
///
/// # Errors
///
/// [`RemoveError::NotFound`] when no entry is stored under `key`; [`RemoveError::Store`] when
/// reading or deleting fails.
pub async fn remove_registration(
    store: &dyn CatalogStore,
    catalog: &CatalogId,
    key: &EntryKey,
) -> Result<Provenance, RemoveError> {
    let stored = store
        .get_entry(catalog, key)
        .await
        .map_err(RemoveError::Store)?
        .ok_or(RemoveError::NotFound)?;
    store
        .delete_entry(catalog, key, Some(&stored.etag))
        .await
        .map_err(RemoveError::Store)?;
    Ok(stored.provenance)
}
