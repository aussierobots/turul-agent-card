#![cfg(feature = "postgres")]

mod common;
mod parity;

use std::sync::Arc;

use turul_ai_catalog_storage::{CatalogStore, PostgresCatalogStore};

const URL: &str = "TURUL_AI_CATALOG_POSTGRES_URL";

async fn store() -> PostgresCatalogStore {
    let url = std::env::var(URL).unwrap_or_else(|_| panic!("set {URL} to run the postgres tests"));
    PostgresCatalogStore::connect(&url).await.unwrap()
}

macro_rules! parity_checks {
    ($($name:ident),* $(,)?) => {
        $(
            #[tokio::test]
            #[ignore = "needs TURUL_AI_CATALOG_POSTGRES_URL"]
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
);

#[tokio::test]
#[ignore = "needs TURUL_AI_CATALOG_POSTGRES_URL"]
async fn concurrent_absent_puts_have_one_winner() {
    let shared: Arc<dyn CatalogStore> = Arc::new(store().await);
    parity::concurrent_absent_puts_have_one_winner(shared).await;
}

#[tokio::test]
#[ignore = "needs TURUL_AI_CATALOG_POSTGRES_URL"]
async fn backend_is_named() {
    assert_eq!(store().await.backend_name(), "postgres");
}
