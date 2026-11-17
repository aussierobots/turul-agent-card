use std::collections::BTreeMap;
use std::sync::RwLock;

use async_trait::async_trait;
use turul_ai_catalog::identity::EntryKey;

use crate::ids::{CatalogId, Etag, RecordKey};
use crate::records::{EntryRecord, HostRecord};
use crate::store::{CatalogStore, Condition, StoreError, Stored, corrupt, key_error};

type Records = BTreeMap<(CatalogId, RecordKey), (Vec<u8>, Etag)>;

/// The default backend: records in a process-local map.
#[derive(Default)]
pub struct InMemoryCatalogStore {
    records: RwLock<Records>,
}

impl InMemoryCatalogStore {
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
        let current = map.get(&slot).map(|(_, etag)| etag);
        let allowed = match &condition {
            Condition::Any => true,
            Condition::Absent => current.is_none(),
            Condition::Matches(e) => current == Some(e),
        };
        if !allowed {
            return Err(StoreError::PreconditionFailed);
        }
        let etag = Etag::of(bytes);
        map.insert(slot, (bytes.to_vec(), etag.clone()));
        Ok(etag)
    }

    fn get(
        &self,
        catalog: &CatalogId,
        key: RecordKey,
    ) -> Result<Option<(Vec<u8>, Etag)>, StoreError> {
        let map = self.records.read().map_err(poisoned)?;
        Ok(map.get(&(catalog.clone(), key)).cloned())
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
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        let key = record.record_key().map_err(key_error)?;
        self.put(catalog, key, record.bytes(), condition)
    }

    async fn get_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
    ) -> Result<Option<Stored<EntryRecord>>, StoreError> {
        let record_key = RecordKey::entry(key).map_err(key_error)?;
        let Some((bytes, etag)) = self.get(catalog, record_key.clone())? else {
            return Ok(None);
        };
        let record = EntryRecord::from_stored(bytes).map_err(|e| corrupt(&record_key, e))?;
        Ok(Some(Stored { record, etag }))
    }

    async fn delete_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
        expected: Option<&Etag>,
    ) -> Result<(), StoreError> {
        let slot = (catalog.clone(), RecordKey::entry(key).map_err(key_error)?);
        let mut map = self.records.write().map_err(poisoned)?;
        match (map.get(&slot), expected) {
            (None, None) => Err(StoreError::NotFound),
            (None, Some(_)) => Err(StoreError::PreconditionFailed),
            (Some((_, current)), Some(e)) if current != e => Err(StoreError::PreconditionFailed),
            _ => {
                map.remove(&slot);
                Ok(())
            }
        }
    }

    async fn list_entries(
        &self,
        catalog: &CatalogId,
    ) -> Result<Vec<Stored<EntryRecord>>, StoreError> {
        let map = self.records.read().map_err(poisoned)?;
        map.iter()
            .filter(|((c, k), _)| c == catalog && k.is_entry())
            .map(|((_, k), (bytes, etag))| {
                let record = EntryRecord::from_stored(bytes.clone()).map_err(|e| corrupt(k, e))?;
                Ok(Stored {
                    record,
                    etag: etag.clone(),
                })
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
        let Some((bytes, etag)) = self.get(catalog, RecordKey::HOST)? else {
            return Ok(None);
        };
        let record = HostRecord::from_stored(bytes).map_err(|e| corrupt(&RecordKey::HOST, e))?;
        Ok(Some(Stored { record, etag }))
    }
}
