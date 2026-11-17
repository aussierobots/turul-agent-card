use async_trait::async_trait;
use sqlx::postgres::{PgPool, PgPoolOptions, PgRow};
use sqlx::{Postgres, Row, Transaction};
use turul_ai_catalog::identity::EntryKey;

use crate::ids::{CatalogId, Etag, RecordKey};
use crate::records::{EntryRecord, HostRecord};
use crate::store::{
    CatalogStore, Condition, StoreError, Stored, corrupt, key_error, kind_conflict,
};

/// Records in a PostgreSQL table. `connect` creates the table when missing.
pub struct PostgresCatalogStore {
    pool: PgPool,
}

fn backend(e: sqlx::Error) -> StoreError {
    StoreError::Backend(e.to_string())
}

impl PostgresCatalogStore {
    /// Connects to the database at `url` and creates the table when missing.
    pub async fn connect(url: &str) -> Result<Self, StoreError> {
        let pool = PgPoolOptions::new().connect(url).await.map_err(backend)?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS catalog_records (\
             catalog_id TEXT NOT NULL, record_key TEXT NOT NULL, \
             record BYTEA NOT NULL, etag TEXT NOT NULL, \
             PRIMARY KEY (catalog_id, record_key))",
        )
        .execute(&pool)
        .await
        .map_err(backend)?;
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS catalog_guards (\
             catalog_id TEXT NOT NULL, guard_key TEXT NOT NULL, \
             versioned BOOLEAN NOT NULL, entries BIGINT NOT NULL, \
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
            Condition::Any => {
                "INSERT INTO catalog_records (catalog_id, record_key, record, etag) \
                 VALUES ($1, $2, $3, $4) \
                 ON CONFLICT (catalog_id, record_key) DO UPDATE \
                 SET record = excluded.record, etag = excluded.etag"
            }
            Condition::Absent => {
                "INSERT INTO catalog_records (catalog_id, record_key, record, etag) \
                 VALUES ($1, $2, $3, $4) ON CONFLICT (catalog_id, record_key) DO NOTHING"
            }
            Condition::Matches(_) => {
                "UPDATE catalog_records SET record = $3, etag = $4 \
                 WHERE catalog_id = $1 AND record_key = $2 AND etag = $5"
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
            "SELECT record, etag FROM catalog_records WHERE catalog_id = $1 AND record_key = $2",
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
    tx: &mut Transaction<'static, Postgres>,
    catalog: &CatalogId,
    guard: &RecordKey,
    versioned: bool,
) -> Result<(bool, i64), StoreError> {
    let row = sqlx::query(
        "INSERT INTO catalog_guards (catalog_id, guard_key, versioned, entries) \
         VALUES ($1, $2, $3, 0) ON CONFLICT (catalog_id, guard_key) \
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
    tx: &mut Transaction<'static, Postgres>,
    catalog: &CatalogId,
    guard: &RecordKey,
    delta: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE catalog_guards SET entries = entries + $3 \
         WHERE catalog_id = $1 AND guard_key = $2",
    )
    .bind(catalog.as_str())
    .bind(guard.as_str())
    .bind(delta)
    .execute(&mut **tx)
    .await
    .map_err(backend)?;
    sqlx::query(
        "DELETE FROM catalog_guards \
         WHERE catalog_id = $1 AND guard_key = $2 AND entries <= 0",
    )
    .bind(catalog.as_str())
    .bind(guard.as_str())
    .execute(&mut **tx)
    .await
    .map_err(backend)?;
    Ok(())
}

fn read_row(row: &PgRow) -> Result<(Vec<u8>, Etag), StoreError> {
    let bytes: Vec<u8> = row.try_get("record").map_err(backend)?;
    // The version is the digest of the bytes read, so a read never reports a stale digest column.
    let etag = Etag::of(&bytes);
    Ok((bytes, etag))
}

#[async_trait]
impl CatalogStore for PostgresCatalogStore {
    fn backend_name(&self) -> &'static str {
        "postgres"
    }

    async fn put_entry(
        &self,
        catalog: &CatalogId,
        record: &EntryRecord,
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        let key = record.record_key().map_err(key_error)?;
        let guard = RecordKey::guard(&record.entry().identifier).map_err(key_error)?;
        let versioned = record.entry().version.is_some();
        let etag = Etag::of(record.bytes());
        let mut tx = self.pool.begin().await.map_err(backend)?;
        let (guard_versioned, entries) = lock_guard(&mut tx, catalog, &guard, versioned).await?;
        let exists =
            sqlx::query("SELECT 1 FROM catalog_records WHERE catalog_id = $1 AND record_key = $2")
                .bind(catalog.as_str())
                .bind(key.as_str())
                .fetch_optional(&mut *tx)
                .await
                .map_err(backend)?
                .is_some();
        let created = match (&condition, exists) {
            (Condition::Absent, true) | (Condition::Matches(_), false) => {
                return Err(StoreError::PreconditionFailed);
            }
            (_, true) => {
                let expected = match &condition {
                    Condition::Matches(e) => Some(e.as_str()),
                    _ => None,
                };
                let done = sqlx::query(
                    "UPDATE catalog_records SET record = $3, etag = $4 \
                     WHERE catalog_id = $1 AND record_key = $2 \
                     AND ($5::text IS NULL OR etag = $5)",
                )
                .bind(catalog.as_str())
                .bind(key.as_str())
                .bind(record.bytes())
                .bind(etag.as_str())
                .bind(expected)
                .execute(&mut *tx)
                .await
                .map_err(backend)?;
                if done.rows_affected() != 1 {
                    return Err(StoreError::PreconditionFailed);
                }
                0
            }
            (_, false) => {
                if entries > 0 && guard_versioned != versioned {
                    return Err(kind_conflict(record));
                }
                sqlx::query(
                    "INSERT INTO catalog_records (catalog_id, record_key, record, etag) \
                     VALUES ($1, $2, $3, $4)",
                )
                .bind(catalog.as_str())
                .bind(key.as_str())
                .bind(record.bytes())
                .bind(etag.as_str())
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
    ) -> Result<Option<Stored<EntryRecord>>, StoreError> {
        let record_key = RecordKey::entry(key).map_err(key_error)?;
        let Some((bytes, etag)) = self.get(catalog, &record_key).await? else {
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
        let record_key = RecordKey::entry(key).map_err(key_error)?;
        let guard = RecordKey::guard(key.identifier()).map_err(key_error)?;
        let versioned = key.version().is_some();
        let mut tx = self.pool.begin().await.map_err(backend)?;
        let (_, entries) = lock_guard(&mut tx, catalog, &guard, versioned).await?;
        let done = sqlx::query(
            "DELETE FROM catalog_records WHERE catalog_id = $1 AND record_key = $2 \
             AND ($3::text IS NULL OR etag = $3)",
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

    async fn list_entries(
        &self,
        catalog: &CatalogId,
    ) -> Result<Vec<Stored<EntryRecord>>, StoreError> {
        let rows = sqlx::query(
            "SELECT record_key, record, etag FROM catalog_records \
             WHERE catalog_id = $1 AND left(record_key, 2) = $2",
        )
        .bind(catalog.as_str())
        .bind(RecordKey::ENTRY_PREFIX)
        .fetch_all(&self.pool)
        .await
        .map_err(backend)?;
        rows.iter()
            .map(|row| {
                let (bytes, etag) = read_row(row)?;
                let name: String = row.try_get("record_key").map_err(backend)?;
                let record = EntryRecord::from_stored(bytes)
                    .map_err(|e| corrupt(&RecordKey::from_stored_name(name), e))?;
                Ok(Stored { record, etag })
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
}
