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
    same_state_assembles_to_the_same_bytes,
    etag_is_the_digest_of_the_record_bytes,
    recreated_identical_record_has_the_same_etag,
    unversioned_entry_beside_versioned_ones_is_refused,
    versioned_entry_beside_an_unversioned_one_is_refused,
    same_version_kind_puts_are_accepted,
    deleting_the_last_record_of_a_kind_frees_the_identifier,
    conditional_puts_keep_their_preconditions,
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
            .put_entry(&c, &r, turul_ai_catalog_storage::Condition::Absent)
            .await
            .unwrap()
    };
    let store = SqliteCatalogStore::connect(path).await.unwrap();
    let got = store.get_entry(&c, &r.key()).await.unwrap().unwrap();
    assert_eq!(got.etag, etag);
    assert_eq!(got.record.bytes(), r.bytes());
    std::fs::remove_dir_all(&dir).unwrap();
}
