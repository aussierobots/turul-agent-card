use std::str::FromStr;

use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use turul_ai_catalog::identity::EntryKey;

use crate::ids::{CatalogId, Etag, RecordKey};
use crate::provenance::{ModeKind, Provenance, ProvenanceError};
use crate::records::{EntryRecord, HostRecord};
use crate::store::{
    CatalogStore, Condition, ItemCounts, PurgeReport, ScannedEntry, StoreError, Stored,
    StoredEntry, corrupt, corrupt_provenance, entry_write, key_error, kind_conflict, scanned_entry,
    stored_entry,
};

const MEMORY: &str = ":memory:";

/// Records in a SQLite database.
///
/// [`SqliteCatalogStore::connect`] creates the file and the tables when missing.
pub struct SqliteCatalogStore {
    pool: SqlitePool,
}

fn backend(e: sqlx::Error) -> StoreError {
    StoreError::Backend(e.to_string())
}

impl SqliteCatalogStore {
    /// Opens the database at `path`; `":memory:"` opens a private in-memory database.
    ///
    /// # Errors
    ///
    /// [`StoreError::Backend`] when the database cannot be opened or its tables created.
    pub async fn connect(path: &str) -> Result<Self, StoreError> {
        let (options, pool_options) = if path == MEMORY {
            // Each connection to an in-memory database is its own database, so the pool holds
            // exactly one and never lets it expire.
            (
                SqliteConnectOptions::from_str("sqlite::memory:").map_err(backend)?,
                SqlitePoolOptions::new()
                    .min_connections(1)
                    .max_connections(1)
                    .idle_timeout(None)
                    .max_lifetime(None),
            )
        } else {
            (
                SqliteConnectOptions::new()
                    .filename(path)
                    .create_if_missing(true),
                SqlitePoolOptions::new(),
            )
        };
        let pool = pool_options.connect_with(options).await.map_err(backend)?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS catalog_records (\
             catalog_id TEXT NOT NULL, record_key TEXT NOT NULL, \
             record BLOB NOT NULL, etag TEXT NOT NULL, \
             provenance BLOB, mode TEXT, \
             PRIMARY KEY (catalog_id, record_key), \
             CHECK ((record_key = 'h' AND provenance IS NULL AND mode IS NULL) \
             OR (record_key <> 'h' AND provenance IS NOT NULL AND mode IN ('lease', 'curated'))))",
        )
        .execute(&pool)
        .await
        .map_err(backend)?;
        // A table created by an earlier release lacks the columns; it is refused, not read as
        // curated.
        sqlx::query("SELECT provenance, mode FROM catalog_records LIMIT 0")
            .execute(&pool)
            .await
            .map_err(|e| {
                StoreError::Backend(format!(
                    "catalog_records was created by an earlier release and has no provenance \
                     columns: {e}"
                ))
            })?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS catalog_guards (\
             catalog_id TEXT NOT NULL, guard_key TEXT NOT NULL, \
             versioned INTEGER NOT NULL, entries INTEGER NOT NULL, \
             PRIMARY KEY (catalog_id, guard_key))",
        )
        .execute(&pool)
        .await
        .map_err(backend)?;
        Ok(Self { pool })
    }

    async fn put(
        &self,
        catalog: &CatalogId,
        key: &RecordKey,
        bytes: &[u8],
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        let etag = Etag::of(bytes);
        let sql = match &condition {
            Condition::Any | Condition::Overwrite => {
                "INSERT INTO catalog_records (catalog_id, record_key, record, etag) \
                 VALUES (?1, ?2, ?3, ?4) \
                 ON CONFLICT (catalog_id, record_key) DO UPDATE \
                 SET record = excluded.record, etag = excluded.etag"
            }
            Condition::Absent => {
                "INSERT INTO catalog_records (catalog_id, record_key, record, etag) \
                 VALUES (?1, ?2, ?3, ?4) ON CONFLICT (catalog_id, record_key) DO NOTHING"
            }
            Condition::Matches(_) => {
                "UPDATE catalog_records SET record = ?3, etag = ?4 \
                 WHERE catalog_id = ?1 AND record_key = ?2 AND etag = ?5"
            }
        };
        let mut query = sqlx::query(sql)
            .bind(catalog.as_str())
            .bind(key.as_str())
            .bind(bytes)
            .bind(etag.as_str());
        if let Condition::Matches(expected) = &condition {
            query = query.bind(expected.as_str());
        }
        let done = query.execute(&self.pool).await.map_err(backend)?;
        if done.rows_affected() == 1 {
            Ok(etag)
        } else {
            Err(StoreError::PreconditionFailed)
        }
    }

    async fn get(
        &self,
        catalog: &CatalogId,
        key: &RecordKey,
    ) -> Result<Option<(Vec<u8>, Etag)>, StoreError> {
        let row = sqlx::query(
            "SELECT record, etag FROM catalog_records WHERE catalog_id = ?1 AND record_key = ?2",
        )
        .bind(catalog.as_str())
        .bind(key.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;
        row.map(|r| read_row(&r)).transpose()
    }
}

/// Locks the guard row of one identifier until the transaction ends, inserting it with no entries
/// when missing, and returns its version kind and entry count. Every entry write and delete takes
/// this lock first, so a version-kind check and the write it allows cannot interleave with another
/// writer of the same identifier.
async fn lock_guard(
    tx: &mut Transaction<'static, Sqlite>,
    catalog: &CatalogId,
    guard: &RecordKey,
    versioned: bool,
) -> Result<(bool, i64), StoreError> {
    let row = sqlx::query(
        "INSERT INTO catalog_guards (catalog_id, guard_key, versioned, entries) \
         VALUES (?1, ?2, ?3, 0) ON CONFLICT (catalog_id, guard_key) \
         DO UPDATE SET entries = catalog_guards.entries RETURNING versioned, entries",
    )
    .bind(catalog.as_str())
    .bind(guard.as_str())
    .bind(versioned)
    .fetch_one(&mut **tx)
    .await
    .map_err(backend)?;
    Ok((
        row.try_get("versioned").map_err(backend)?,
        row.try_get("entries").map_err(backend)?,
    ))
}

/// Adds `delta` to the guard's entry count and drops the guard when no entry is left. A guard row
/// with no entries exists only inside the transaction that inserted it, so its kind is the kind of
/// that transaction's record.
async fn count_entries(
    tx: &mut Transaction<'static, Sqlite>,
    catalog: &CatalogId,
    guard: &RecordKey,
    delta: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE catalog_guards SET entries = entries + ?3 \
         WHERE catalog_id = ?1 AND guard_key = ?2",
    )
    .bind(catalog.as_str())
    .bind(guard.as_str())
    .bind(delta)
    .execute(&mut **tx)
    .await
    .map_err(backend)?;
    sqlx::query(
        "DELETE FROM catalog_guards \
         WHERE catalog_id = ?1 AND guard_key = ?2 AND entries <= 0",
    )
    .bind(catalog.as_str())
    .bind(guard.as_str())
    .execute(&mut **tx)
    .await
    .map_err(backend)?;
    Ok(())
}

fn read_row(row: &sqlx::sqlite::SqliteRow) -> Result<(Vec<u8>, Etag), StoreError> {
    let bytes: Vec<u8> = row.try_get("record").map_err(backend)?;
    // The version is the digest of the bytes read, so a read never reports a stale digest column.
    let etag = Etag::of(&bytes);
    Ok((bytes, etag))
}

fn read_entry(key: &RecordKey, row: &sqlx::sqlite::SqliteRow) -> Result<StoredEntry, StoreError> {
    let bytes: Vec<u8> = row.try_get("record").map_err(backend)?;
    let provenance: Vec<u8> = row.try_get("provenance").map_err(backend)?;
    stored_entry(key, bytes, Some(&provenance))
}

#[async_trait]
impl CatalogStore for SqliteCatalogStore {
    fn backend_name(&self) -> &'static str {
        "sqlite"
    }

    async fn put_entry(
        &self,
        catalog: &CatalogId,
        record: &EntryRecord,
        provenance: &Provenance,
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        let key = record.record_key().map_err(key_error)?;
        let guard = RecordKey::guard(&record.entry().identifier).map_err(key_error)?;
        let versioned = record.entry().version.is_some();
        let (provenance_bytes, etag) = entry_write(&key, record, provenance)?;
        let mode = provenance.kind();
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(backend)?;
        let (guard_versioned, entries) = lock_guard(&mut tx, catalog, &guard, versioned).await?;
        // An overwrite checks that a record exists and reads none of it.
        let stored = sqlx::query(if condition == Condition::Overwrite {
            "SELECT 1 AS present FROM catalog_records WHERE catalog_id = ?1 AND record_key = ?2"
        } else {
            "SELECT etag, mode FROM catalog_records WHERE catalog_id = ?1 AND record_key = ?2"
        })
        .bind(catalog.as_str())
        .bind(key.as_str())
        .fetch_optional(&mut *tx)
        .await
        .map_err(backend)?;
        let created = match (&condition, stored) {
            (Condition::Absent, Some(_)) | (Condition::Matches(_), None) => {
                return Err(StoreError::PreconditionFailed);
            }
            (_, Some(row)) => {
                if let Condition::Matches(e) = &condition {
                    let stored_etag: String = row.try_get("etag").map_err(backend)?;
                    if stored_etag != e.as_str() {
                        return Err(StoreError::PreconditionFailed);
                    }
                }
                if condition != Condition::Overwrite {
                    let stored_mode: Option<String> = row.try_get("mode").map_err(backend)?;
                    match stored_mode.as_deref().map(ModeKind::from_name) {
                        Some(Some(stored)) if stored == mode => {}
                        Some(Some(stored)) => return Err(StoreError::ModeConflict { stored }),
                        Some(None) => return Err(corrupt_provenance(&key, ProvenanceError::Mode)),
                        None => return Err(corrupt_provenance(&key, ProvenanceError::Missing)),
                    }
                }
                sqlx::query(
                    "UPDATE catalog_records SET record = ?3, etag = ?4, provenance = ?5, \
                     mode = ?6 WHERE catalog_id = ?1 AND record_key = ?2",
                )
                .bind(catalog.as_str())
                .bind(key.as_str())
                .bind(record.bytes())
                .bind(etag.as_str())
                .bind(provenance_bytes.as_slice())
                .bind(mode.as_str())
                .execute(&mut *tx)
                .await
                .map_err(backend)?;
                0
            }
            (_, None) => {
                if entries > 0 && guard_versioned != versioned {
                    return Err(kind_conflict(record));
                }
                sqlx::query(
                    "INSERT INTO catalog_records \
                     (catalog_id, record_key, record, etag, provenance, mode) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                )
                .bind(catalog.as_str())
                .bind(key.as_str())
                .bind(record.bytes())
                .bind(etag.as_str())
                .bind(provenance_bytes.as_slice())
                .bind(mode.as_str())
                .execute(&mut *tx)
                .await
                .map_err(backend)?;
                1
            }
        };
        // A replace adds no entry; it drops the empty guard inserted for a record stored without one.
        count_entries(&mut tx, catalog, &guard, created).await?;
        tx.commit().await.map_err(backend)?;
        Ok(etag)
    }

    async fn get_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
    ) -> Result<Option<StoredEntry>, StoreError> {
        let record_key = RecordKey::entry(key).map_err(key_error)?;
        let row = sqlx::query(
            "SELECT record, provenance FROM catalog_records \
             WHERE catalog_id = ?1 AND record_key = ?2",
        )
        .bind(catalog.as_str())
        .bind(record_key.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(backend)?;
        row.map(|row| read_entry(&record_key, &row)).transpose()
    }

    async fn delete_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
        expected: Option<&Etag>,
    ) -> Result<(), StoreError> {
        let record_key = RecordKey::entry(key).map_err(key_error)?;
        let guard = RecordKey::guard(key.identifier()).map_err(key_error)?;
        let versioned = key.version().is_some();
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(backend)?;
        let (_, entries) = lock_guard(&mut tx, catalog, &guard, versioned).await?;
        let done = sqlx::query(
            "DELETE FROM catalog_records WHERE catalog_id = ?1 AND record_key = ?2 \
             AND (?3 IS NULL OR etag = ?3)",
        )
        .bind(catalog.as_str())
        .bind(record_key.as_str())
        .bind(expected.map(Etag::as_str))
        .execute(&mut *tx)
        .await
        .map_err(backend)?;
        match (done.rows_affected(), expected) {
            (1, _) => {}
            (_, None) => return Err(StoreError::NotFound),
            (_, Some(_)) => return Err(StoreError::PreconditionFailed),
        }
        // A record stored without a guard was never counted.
        let delta = if entries > 0 { -1 } else { 0 };
        count_entries(&mut tx, catalog, &guard, delta).await?;
        tx.commit().await.map_err(backend)?;
        Ok(())
    }

    async fn list_entries(&self, catalog: &CatalogId) -> Result<Vec<StoredEntry>, StoreError> {
        let rows = sqlx::query(
            "SELECT record_key, record, provenance FROM catalog_records \
             WHERE catalog_id = ?1 AND substr(record_key, 1, 2) = ?2",
        )
        .bind(catalog.as_str())
        .bind(RecordKey::ENTRY_PREFIX)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;
        rows.iter()
            .map(|row| {
                let name: String = row.try_get("record_key").map_err(backend)?;
                read_entry(&RecordKey::from_stored_name(name), row)
            })
            .collect()
    }

    async fn put_host(
        &self,
        catalog: &CatalogId,
        record: &HostRecord,
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        self.put(catalog, &RecordKey::HOST, record.bytes(), condition)
            .await
    }

    async fn get_host(
        &self,
        catalog: &CatalogId,
    ) -> Result<Option<Stored<HostRecord>>, StoreError> {
        let Some((bytes, etag)) = self.get(catalog, &RecordKey::HOST).await? else {
            return Ok(None);
        };
        let record = HostRecord::from_stored(bytes).map_err(|e| corrupt(&RecordKey::HOST, e))?;
        Ok(Some(Stored { record, etag }))
    }

    async fn scan_entries(&self, catalog: &CatalogId) -> Result<Vec<ScannedEntry>, StoreError> {
        let rows = sqlx::query(
            "SELECT record_key, record, etag, provenance FROM catalog_records \
             WHERE catalog_id = ?1 AND substr(record_key, 1, 2) = ?2",
        )
        .bind(catalog.as_str())
        .bind(RecordKey::ENTRY_PREFIX)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;
        rows.iter()
            .map(|row| {
                let name: String = row.try_get("record_key").map_err(backend)?;
                let etag: String = row.try_get("etag").map_err(backend)?;
                let bytes: Vec<u8> = row.try_get("record").map_err(backend)?;
                let provenance: Option<Vec<u8>> = row.try_get("provenance").map_err(backend)?;
                scanned_entry(
                    &RecordKey::from_stored_name(name),
                    bytes,
                    Etag::from_stored(etag),
                    provenance.as_deref(),
                )
            })
            .collect()
    }

    async fn count_catalog(&self, catalog: &CatalogId) -> Result<ItemCounts, StoreError> {
        let mut counts = ItemCounts::default();
        for row in sqlx::query("SELECT record_key FROM catalog_records WHERE catalog_id = ?1")
            .bind(catalog.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(backend)?
        {
            let name: String = row.try_get("record_key").map_err(backend)?;
            counts.add_key(&name);
        }
        let guards = sqlx::query("SELECT guard_key FROM catalog_guards WHERE catalog_id = ?1")
            .bind(catalog.as_str())
            .fetch_all(&self.pool)
            .await
            .map_err(backend)?;
        counts.guards += guards.len() as u64;
        Ok(counts)
    }

    async fn purge_catalog(&self, catalog: &CatalogId) -> Result<PurgeReport, StoreError> {
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(backend)?;
        let mut removed = ItemCounts::default();
        for row in
            sqlx::query("DELETE FROM catalog_records WHERE catalog_id = ?1 RETURNING record_key")
                .bind(catalog.as_str())
                .fetch_all(&mut *tx)
                .await
                .map_err(backend)?
        {
            let name: String = row.try_get("record_key").map_err(backend)?;
            removed.add_key(&name);
        }
        let guards =
            sqlx::query("DELETE FROM catalog_guards WHERE catalog_id = ?1 RETURNING guard_key")
                .bind(catalog.as_str())
                .fetch_all(&mut *tx)
                .await
                .map_err(backend)?;
        removed.guards += guards.len() as u64;
        tx.commit().await.map_err(backend)?;
        Ok(PurgeReport { removed })
    }
}
