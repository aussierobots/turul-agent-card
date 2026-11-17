use std::collections::BTreeMap;
use std::sync::RwLock;

use async_trait::async_trait;
use turul_ai_catalog::identity::EntryKey;

use crate::ids::{CatalogId, Etag, RecordKey};
use crate::provenance::{ModeKind, Provenance};
use crate::records::{EntryRecord, HostRecord};
use crate::store::{
    CatalogStore, Condition, StoreError, Stored, StoredEntry, corrupt, entry_write, key_error,
    kind_conflict, stored_entry,
};

/// One stored record: its bytes and version, and for an entry its provenance bytes and mode.
#[derive(Clone)]
struct Slot {
    bytes: Vec<u8>,
    etag: Etag,
    provenance: Option<(Vec<u8>, ModeKind)>,
}

/// Records, and per guard key the identifier's version kind (`true` for versioned) and its number
/// of entry records. A guard exists while its identifier has at least one entry record.
#[derive(Default)]
struct Records {
    records: BTreeMap<(CatalogId, RecordKey), Slot>,
    guards: BTreeMap<(CatalogId, RecordKey), (bool, usize)>,
}

/// The default backend: records in a process-local map, every write under one lock.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog::model::HostInfo;
/// use turul_ai_catalog_storage::{
///     CatalogId, CatalogStore, Condition, HostRecord, InMemoryCatalogStore,
/// };
///
/// # futures::executor::block_on(async {
/// let store = InMemoryCatalogStore::new();
/// let catalog = CatalogId::new("main")?;
/// let host = HostRecord::new(HostInfo::new("Example".into()))?;
/// let etag = store.put_host(&catalog, &host, Condition::Absent).await?;
/// let stored = store.get_host(&catalog).await?.ok_or("host missing")?;
/// assert_eq!(stored.etag, etag);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Default)]
pub struct InMemoryCatalogStore {
    records: RwLock<Records>,
}

impl InMemoryCatalogStore {
    /// Creates an empty store.
    pub fn new() -> Self {
        Self::default()
    }
}

fn poisoned<T>(_: T) -> StoreError {
    StoreError::Backend("record map lock poisoned".to_string())
}

impl InMemoryCatalogStore {
    fn put(
        &self,
        catalog: &CatalogId,
        key: RecordKey,
        bytes: &[u8],
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        let mut map = self.records.write().map_err(poisoned)?;
        let slot = (catalog.clone(), key);
        let etag = Etag::of(bytes);
        Self::check_condition(&map, &slot, &condition)?;
        map.records.insert(
            slot,
            Slot {
                bytes: bytes.to_vec(),
                etag: etag.clone(),
                provenance: None,
            },
        );
        Ok(etag)
    }

    fn check_condition(
        map: &Records,
        slot: &(CatalogId, RecordKey),
        condition: &Condition,
    ) -> Result<(), StoreError> {
        let current = map.records.get(slot).map(|s| &s.etag);
        let allowed = match condition {
            Condition::Any => true,
            Condition::Absent => current.is_none(),
            Condition::Matches(e) => current == Some(e),
        };
        if allowed {
            Ok(())
        } else {
            Err(StoreError::PreconditionFailed)
        }
    }

    fn get(&self, catalog: &CatalogId, key: RecordKey) -> Result<Option<Slot>, StoreError> {
        let map = self.records.read().map_err(poisoned)?;
        Ok(map.records.get(&(catalog.clone(), key)).cloned())
    }
}

#[async_trait]
impl CatalogStore for InMemoryCatalogStore {
    fn backend_name(&self) -> &'static str {
        "in-memory"
    }

    async fn put_entry(
        &self,
        catalog: &CatalogId,
        record: &EntryRecord,
        provenance: &Provenance,
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        let key = record.record_key().map_err(key_error)?;
        let (provenance_bytes, etag) = entry_write(&key, record, provenance)?;
        let slot = (catalog.clone(), key);
        let guard = (
            catalog.clone(),
            RecordKey::guard(&record.entry().identifier).map_err(key_error)?,
        );
        let versioned = record.entry().version.is_some();
        let mut map = self.records.write().map_err(poisoned)?;
        Self::check_condition(&map, &slot, &condition)?;
        let created = !map.records.contains_key(&slot);
        if let Some(stored) = map.records.get(&slot).and_then(|s| s.provenance.as_ref())
            && stored.1 != provenance.kind()
        {
            return Err(StoreError::ModeConflict { stored: stored.1 });
        }
        if created && map.guards.get(&guard).is_some_and(|(v, _)| *v != versioned) {
            return Err(kind_conflict(record));
        }
        map.records.insert(
            slot,
            Slot {
                bytes: record.bytes().to_vec(),
                etag: etag.clone(),
                provenance: Some((provenance_bytes, provenance.kind())),
            },
        );
        if created {
            map.guards.entry(guard).or_insert((versioned, 0)).1 += 1;
        }
        Ok(etag)
    }

    async fn get_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
    ) -> Result<Option<StoredEntry>, StoreError> {
        let record_key = RecordKey::entry(key).map_err(key_error)?;
        let Some(slot) = self.get(catalog, record_key.clone())? else {
            return Ok(None);
        };
        stored_entry(
            &record_key,
            slot.bytes,
            slot.provenance.as_ref().map(|p| p.0.as_slice()),
        )
        .map(Some)
    }

    async fn delete_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
        expected: Option<&Etag>,
    ) -> Result<(), StoreError> {
        let slot = (catalog.clone(), RecordKey::entry(key).map_err(key_error)?);
        let guard = (
            catalog.clone(),
            RecordKey::guard(key.identifier()).map_err(key_error)?,
        );
        let mut map = self.records.write().map_err(poisoned)?;
        match (map.records.get(&slot), expected) {
            (None, None) => Err(StoreError::NotFound),
            (None, Some(_)) => Err(StoreError::PreconditionFailed),
            (Some(slot), Some(e)) if slot.etag != *e => Err(StoreError::PreconditionFailed),
            _ => {
                map.records.remove(&slot);
                if let Some((_, count)) = map.guards.get_mut(&guard) {
                    *count -= 1;
                    if *count == 0 {
                        map.guards.remove(&guard);
                    }
                }
                Ok(())
            }
        }
    }

    async fn list_entries(&self, catalog: &CatalogId) -> Result<Vec<StoredEntry>, StoreError> {
        let map = self.records.read().map_err(poisoned)?;
        map.records
            .iter()
            .filter(|((c, k), _)| c == catalog && k.is_entry())
            .map(|((_, k), slot)| {
                stored_entry(
                    k,
                    slot.bytes.clone(),
                    slot.provenance.as_ref().map(|p| p.0.as_slice()),
                )
            })
            .collect()
    }

    async fn put_host(
        &self,
        catalog: &CatalogId,
        record: &HostRecord,
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        self.put(catalog, RecordKey::HOST, record.bytes(), condition)
    }

    async fn get_host(
        &self,
        catalog: &CatalogId,
    ) -> Result<Option<Stored<HostRecord>>, StoreError> {
        let Some(slot) = self.get(catalog, RecordKey::HOST)? else {
            return Ok(None);
        };
        let record =
            HostRecord::from_stored(slot.bytes).map_err(|e| corrupt(&RecordKey::HOST, e))?;
        Ok(Some(Stored {
            record,
            etag: slot.etag,
        }))
    }
}
