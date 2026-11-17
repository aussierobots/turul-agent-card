#![cfg(feature = "sqlite")]

mod common;
mod parity;

use std::sync::Arc;

use turul_ai_catalog_storage::{CatalogStore, SqliteCatalogStore};

async fn store() -> SqliteCatalogStore {
    SqliteCatalogStore::connect(":memory:").await.unwrap()
}

macro_rules! parity_checks {
    ($($name:ident),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $name() {
                parity::$name(&store().await).await;
            }
        )*
    };
}

parity_checks!(
    one_record_per_entry_key,
    stored_entry_reads_back_byte_identical,
    entry_trust_manifest_payload_survives_storage,
    catalogs_are_isolated,
    absent_condition_refuses_an_existing_record,
    matches_condition_refuses_a_stale_etag,
    delete_with_a_stale_etag_is_refused,
    delete_of_a_missing_record_is_not_found,
    writes_are_visible_to_the_next_read,
    list_returns_every_entry,
    same_state_and_now_assemble_to_the_same_bytes,
    expired_lease_keeps_its_version_kind,
    provenance_reads_back_exactly,
    provenance_that_would_not_read_back_is_refused,
    etag_is_the_digest_of_entry_and_provenance,
    lease_and_curated_puts_refuse_each_other,
    matches_condition_refuses_a_stale_lease_etag,
    recreated_identical_record_has_the_same_etag,
    unversioned_entry_beside_versioned_ones_is_refused,
    versioned_entry_beside_an_unversioned_one_is_refused,
    same_version_kind_puts_are_accepted,
    deleting_the_last_record_of_a_kind_frees_the_identifier,
    conditional_puts_keep_their_preconditions,
);

/// Checks that need a store they can share, such as with a `Registrar`.
macro_rules! shared_parity_checks {
    ($($name:ident),* $(,)?) => {
        $(
            #[tokio::test]
            async fn $name() {
                parity::$name(Arc::new(store().await)).await;
            }
        )*
    };
}

shared_parity_checks!(
    registered_identifier_reads_back_exactly,
    another_writer_cannot_take_over_an_entry,
    unregister_by_another_writer_is_refused,
    unregister_frees_the_identifier,
);

#[tokio::test]
async fn concurrent_absent_puts_have_one_winner() {
    let shared: Arc<dyn CatalogStore> = Arc::new(store().await);
    parity::concurrent_absent_puts_have_one_winner(shared).await;
}

#[tokio::test]
async fn racing_version_kinds_leave_one_kind() {
    let shared: Arc<dyn CatalogStore> = Arc::new(store().await);
    parity::racing_version_kinds_leave_one_kind(shared).await;
}

#[tokio::test]
async fn racing_cross_mode_puts_leave_one_mode() {
    let shared: Arc<dyn CatalogStore> = Arc::new(store().await);
    parity::racing_cross_mode_puts_leave_one_mode(shared).await;
}

#[tokio::test]
async fn concurrent_lease_refreshes_on_one_etag_have_one_winner() {
    let shared: Arc<dyn CatalogStore> = Arc::new(store().await);
    parity::concurrent_lease_refreshes_on_one_etag_have_one_winner(shared).await;
}

#[tokio::test]
async fn many_instances_refreshing_one_lease_all_succeed() {
    let shared: Arc<dyn CatalogStore> = Arc::new(store().await);
    parity::many_instances_refreshing_one_lease_all_succeed(shared).await;
}

#[tokio::test]
async fn backend_is_named() {
    assert_eq!(store().await.backend_name(), "sqlite");
}

#[tokio::test]
async fn records_survive_reconnecting_to_a_file() {
    let dir = std::env::temp_dir().join(format!("catalog-sqlite-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("catalog.db");
    let path = path.to_str().unwrap();
    let c = common::catalog("file-backed");
    let r = common::record("urn:air:a", None);
    let etag = {
        let store = SqliteCatalogStore::connect(path).await.unwrap();
        store
            .put_entry(
                &c,
                &r,
                &common::curated(),
                turul_ai_catalog_storage::Condition::Absent,
            )
            .await
            .unwrap()
    };
    let store = SqliteCatalogStore::connect(path).await.unwrap();
    let got = store.get_entry(&c, &r.key()).await.unwrap().unwrap();
    assert_eq!(got.etag, etag);
    assert_eq!(got.record.bytes(), r.bytes());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[tokio::test]
async fn table_from_an_earlier_release_fails_connect() {
    let dir = std::env::temp_dir().join(format!("catalog-sqlite-old-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("catalog.db");
    let path = path.to_str().unwrap();
    {
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{path}?mode=rwc"))
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE catalog_records (catalog_id TEXT NOT NULL, record_key TEXT NOT NULL, \
             record BLOB NOT NULL, etag TEXT NOT NULL, PRIMARY KEY (catalog_id, record_key))",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
    }
    let Err(turul_ai_catalog_storage::StoreError::Backend(message)) =
        SqliteCatalogStore::connect(path).await
    else {
        panic!("connect must refuse a table without provenance columns");
    };
    assert!(message.contains("provenance"), "{message}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The `CHECK` keeps a record without provenance, or a host with one, out of the table.
#[tokio::test]
async fn table_check_refuses_an_entry_row_without_provenance() {
    let dir = std::env::temp_dir().join(format!("catalog-sqlite-check-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("catalog.db");
    let path = path.to_str().unwrap();
    drop(SqliteCatalogStore::connect(path).await.unwrap());
    let pool = sqlx::SqlitePool::connect(&format!("sqlite://{path}"))
        .await
        .unwrap();
    let insert =
        |key: &'static str, provenance: Option<&'static str>, mode: Option<&'static str>| {
            let pool = pool.clone();
            async move {
                sqlx::query(
                    "INSERT INTO catalog_records \
                 (catalog_id, record_key, record, etag, provenance, mode) \
                 VALUES ('c', ?1, x'7b7d', 'e', ?2, ?3)",
                )
                .bind(key)
                .bind(provenance)
                .bind(mode)
                .execute(&pool)
                .await
            }
        };
    assert!(insert("e:1", None, None).await.is_err());
    assert!(insert("e:1", Some("{}"), Some("other")).await.is_err());
    assert!(insert("h", Some("{}"), Some("curated")).await.is_err());
    assert!(insert("e:1", Some("{}"), Some("curated")).await.is_ok());
    assert!(insert("h", None, None).await.is_ok());
    pool.close().await;
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A row whose mode is neither `lease` nor `curated`, written with the `CHECK` switched off.
#[tokio::test]
async fn unknown_stored_mode_is_corrupt_provenance() {
    use sqlx::Connection;
    use turul_ai_catalog_storage::Etag;

    let dir = std::env::temp_dir().join(format!("catalog-sqlite-mode-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("catalog.db");
    let path = path.to_str().unwrap();
    let store = SqliteCatalogStore::connect(path).await.unwrap();
    let c = common::catalog("unknown-mode");
    let r = common::record("urn:air:a", None);
    let etag = Etag::of_entry(&r, &common::curated()).unwrap();
    let mut raw = sqlx::SqliteConnection::connect(&format!("sqlite://{path}"))
        .await
        .unwrap();
    sqlx::query("PRAGMA ignore_check_constraints = ON")
        .execute(&mut raw)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO catalog_records (catalog_id, record_key, record, etag, provenance, mode) \
         VALUES (?1, ?2, ?3, ?4, ?5, 'other')",
    )
    .bind(c.as_str())
    .bind(r.record_key().unwrap().as_str())
    .bind(r.bytes())
    .bind(etag.as_str())
    .bind(common::curated().to_bytes().unwrap())
    .execute(&mut raw)
    .await
    .unwrap();
    raw.close().await.unwrap();
    parity::unknown_stored_mode_is_corrupt_provenance(&store, &c, &r, &etag).await;
    std::fs::remove_dir_all(&dir).unwrap();
}
