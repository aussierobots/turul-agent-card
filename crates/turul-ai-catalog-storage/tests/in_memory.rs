#![cfg(feature = "in-memory")]

mod common;
mod parity;

use std::sync::Arc;

use futures::executor::block_on;
use turul_ai_catalog_storage::{CatalogStore, InMemoryCatalogStore};

fn store() -> InMemoryCatalogStore {
    InMemoryCatalogStore::new()
}

macro_rules! parity_checks {
    ($($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                block_on(parity::$name(&store()));
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
    purge_removes_every_item_of_the_catalog,
    purge_of_an_empty_catalog_removes_nothing,
    count_catalog_is_what_purge_removes,
    purge_leaves_other_catalogs_untouched,
    overwrite_replaces_a_record_of_either_mode,
    overwrite_requires_the_stored_version,
    overwrite_of_a_host_requires_its_version,
    scan_entries_matches_list_entries,
);

/// Checks that need a store they can share, such as with a `Registrar`.
macro_rules! shared_parity_checks {
    ($($name:ident),* $(,)?) => {
        $(
            #[test]
            fn $name() {
                block_on(parity::$name(Arc::new(store())));
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

#[test]
fn concurrent_absent_puts_have_one_winner() {
    let shared: Arc<dyn CatalogStore> = Arc::new(store());
    block_on(parity::concurrent_absent_puts_have_one_winner(shared));
}

#[test]
fn racing_version_kinds_leave_one_kind() {
    let shared: Arc<dyn CatalogStore> = Arc::new(store());
    block_on(parity::racing_version_kinds_leave_one_kind(shared));
}

#[test]
fn racing_cross_mode_puts_leave_one_mode() {
    let shared: Arc<dyn CatalogStore> = Arc::new(store());
    block_on(parity::racing_cross_mode_puts_leave_one_mode(shared));
}

#[test]
fn concurrent_lease_refreshes_on_one_etag_have_one_winner() {
    let shared: Arc<dyn CatalogStore> = Arc::new(store());
    block_on(parity::concurrent_lease_refreshes_on_one_etag_have_one_winner(shared));
}

#[test]
fn many_instances_refreshing_one_lease_all_succeed() {
    let shared: Arc<dyn CatalogStore> = Arc::new(store());
    block_on(parity::many_instances_refreshing_one_lease_all_succeed(
        shared,
    ));
}

#[test]
fn purge_removes_many_entries() {
    block_on(parity::purge_removes_many_entries(&store(), 300));
}

#[test]
fn backend_is_named() {
    assert_eq!(store().backend_name(), "in-memory");
}
