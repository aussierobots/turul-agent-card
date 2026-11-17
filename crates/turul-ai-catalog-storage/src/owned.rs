use std::time::{Duration, SystemTime};

use turul_ai_catalog::timestamp::Timestamp;

use crate::ids::{CatalogId, Etag, WriterId};
use crate::provenance::{ModeKind, Provenance};
use crate::records::EntryRecord;
use crate::registration::{PublisherVerifier, VerifierRefusal};
use crate::store::{CatalogStore, Condition, StoreError};

/// Attempts at one write before giving up on a record other writers keep changing.
pub(crate) const ATTEMPTS: usize = 8;

/// Why an owned write was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OwnedWriteError {
    /// The store refused the read or the write, a record of the other mode included.
    Store(StoreError),
    /// The key holds a record of another writer.
    OwnedByAnotherWriter,
    /// The verifier refused the entry.
    Publisher(VerifierRefusal),
    /// Other writers changed the record on every attempt.
    Contended,
    /// A lease from the write's `now` would end past what RFC 3339 can write.
    ExpiryOutOfRange,
}

impl From<StoreError> for OwnedWriteError {
    fn from(e: StoreError) -> Self {
        Self::Store(e)
    }
}

/// The mode an owned write records.
#[derive(Clone, Copy, Debug)]
pub(crate) enum WriteMode {
    Curated,
    /// A lease of this length from the write's `now`.
    Lease(Duration),
}

impl WriteMode {
    fn kind(self) -> ModeKind {
        match self {
            Self::Curated => ModeKind::Curated,
            Self::Lease(_) => ModeKind::Lease,
        }
    }
}

/// The end of a lease of length `ttl` from `now`, or `None` when it would fall outside the years
/// RFC 3339 can write (0000 to 9999), so no stored expiry is one its reads would refuse.
pub(crate) fn lease_expiry(now: SystemTime, ttl: Duration) -> Option<Timestamp> {
    let expiry = Timestamp::from_system_time(now.checked_add(ttl)?);
    (Timestamp::parse(&expiry.to_string()) == Ok(expiry)).then_some(expiry)
}

/// What an owned write left in the store.
#[derive(Clone, Debug)]
pub(crate) struct Written {
    pub(crate) etag: Etag,
    pub(crate) provenance: Provenance,
    /// Whether a put committed; `false` when the stored record already said what was asked.
    pub(crate) written: bool,
}

/// The one path to an entry record: writes `record` for `writer` in `mode`.
///
/// An absent key is created. A stored record in the other mode is refused with
/// [`StoreError::ModeConflict`], one of another writer with `OwnedByAnotherWriter`. Of the same
/// writer, a curated record with equal bytes is left alone; a lease with equal bytes (a refresh)
/// is rewritten only once its expiry is less than three quarters of the lease length away, and
/// never to an earlier expiry; a lease with new bytes (an update) takes the later of its stored
/// expiry and `now` plus the lease length. Every put is `Condition::Matches` of the version read,
/// or `Absent`, so nothing another writer changed since is overwritten; a lost race re-reads, up
/// to [`ATTEMPTS`] times, then `Contended`.
///
/// `verifier`, when given, runs once, after the first read that leads to a put and before that
/// put; a retry does not verify again. A lease that would end past year 9999 is refused with
/// `ExpiryOutOfRange` before the store is read.
pub(crate) async fn write_owned(
    store: &dyn CatalogStore,
    catalog: &CatalogId,
    record: &EntryRecord,
    writer: &WriterId,
    mode: WriteMode,
    verifier: Option<&dyn PublisherVerifier>,
    now: SystemTime,
) -> Result<Written, OwnedWriteError> {
    // The expiry a write grants and the stored expiry at or after which a refresh is not yet due.
    let lease = match mode {
        WriteMode::Curated => None,
        WriteMode::Lease(ttl) => Some((
            lease_expiry(now, ttl).ok_or(OwnedWriteError::ExpiryOutOfRange)?,
            lease_expiry(now, ttl - ttl / 4).ok_or(OwnedWriteError::ExpiryOutOfRange)?,
        )),
    };
    let mut verified = verifier.is_none();
    for _ in 0..ATTEMPTS {
        let stored = store.get_entry(catalog, &record.key()).await?;
        let (provenance, condition) = match (stored, mode) {
            (None, _) => (fresh(writer, lease), Condition::Absent),
            (Some(stored), _) => {
                if stored.provenance.kind() != mode.kind() {
                    return Err(StoreError::ModeConflict {
                        stored: stored.provenance.kind(),
                    }
                    .into());
                }
                if stored.provenance.writer() != writer {
                    return Err(OwnedWriteError::OwnedByAnotherWriter);
                }
                let same_bytes = stored.record.bytes() == record.bytes();
                let provenance = match (&stored.provenance, lease) {
                    (Provenance::Lease { expires_at, .. }, Some((granted, due))) => {
                        if same_bytes && *expires_at >= due {
                            return Ok(Written {
                                etag: stored.etag,
                                provenance: stored.provenance,
                                written: false,
                            });
                        }
                        Provenance::Lease {
                            writer: writer.clone(),
                            expires_at: (*expires_at).max(granted),
                        }
                    }
                    _ => {
                        if same_bytes {
                            return Ok(Written {
                                etag: stored.etag,
                                provenance: stored.provenance,
                                written: false,
                            });
                        }
                        stored.provenance
                    }
                };
                (provenance, Condition::Matches(stored.etag))
            }
        };
        if !verified {
            if let Some(verifier) = verifier {
                verifier
                    .verify_publisher(record.entry(), now)
                    .await
                    .map_err(OwnedWriteError::Publisher)?;
            }
            verified = true;
        }
        match store
            .put_entry(catalog, record, &provenance, condition)
            .await
        {
            Ok(etag) => {
                return Ok(Written {
                    etag,
                    provenance,
                    written: true,
                });
            }
            Err(StoreError::PreconditionFailed) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Err(OwnedWriteError::Contended)
}

/// The provenance of a new record: a lease ending at the granted expiry, or curated.
fn fresh(writer: &WriterId, lease: Option<(Timestamp, Timestamp)>) -> Provenance {
    match lease {
        None => Provenance::Curated {
            writer: writer.clone(),
        },
        Some((granted, _)) => Provenance::Lease {
            writer: writer.clone(),
            expires_at: granted,
        },
    }
}
