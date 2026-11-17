#![cfg(any(feature = "in-memory", feature = "sqlite"))]

mod common;

use turul_ai_catalog::identity::{EntryIdentifier, EntryKey};
use turul_ai_catalog_storage::{
    CatalogStore, Condition, Expiry, RemoveError, StoreError, list_registrations,
    remove_registration,
};

use common::{catalog, curated, lease, now, record, ts};

fn key(identifier: &str, version: Option<&str>) -> EntryKey {
    EntryKey::new(
        EntryIdentifier::new(identifier),
        version.map(str::to_string),
    )
}

async fn seed(store: &dyn CatalogStore) {
    let c = catalog("c");
    for (id, version, provenance) in [
        ("b-curated", None, curated()),
        ("a-live", Some("1"), lease("svc", "2026-11-17T13:00:00Z")),
        ("a-live", Some("2"), lease("svc", "2026-11-17T14:00:00Z")),
        ("c-dead", None, lease("svc", "2026-11-17T11:00:00Z")),
        ("d-edge", None, lease("svc", "2026-11-17T12:00:00Z")),
    ] {
        store
            .put_entry(&c, &record(id, version), &provenance, Condition::Absent)
            .await
            .unwrap();
    }
    // another catalog is never listed
    store
        .put_entry(
            &catalog("other"),
            &record("x", None),
            &curated(),
            Condition::Absent,
        )
        .await
        .unwrap();
}

async fn list_reports_provenance_and_expiry_state(store: &dyn CatalogStore) {
    seed(store).await;
    let rows = list_registrations(store, &catalog("c"), now())
        .await
        .unwrap();
    let got: Vec<_> = rows.iter().map(ToString::to_string).collect();
    assert_eq!(
        got,
        [
            "a-live  1  lease  svc  live until 2026-11-17T13:00:00Z",
            "a-live  2  lease  svc  live until 2026-11-17T14:00:00Z",
            "b-curated  -  curated  operator  no expiry",
            "c-dead  -  lease  svc  expired at 2026-11-17T11:00:00Z",
            // a lease ends at its expiry instant, as in assembly
            "d-edge  -  lease  svc  expired at 2026-11-17T12:00:00Z",
        ]
    );
    assert_eq!(rows[2].expiry, Expiry::Never);
    assert_eq!(rows[3].expiry, Expiry::Expired(ts("2026-11-17T11:00:00Z")));
}

async fn remove_deletes_exactly_the_named_key(store: &dyn CatalogStore) {
    seed(store).await;
    let c = catalog("c");
    let removed = remove_registration(store, &c, &key("a-live", Some("1")))
        .await
        .unwrap();
    assert_eq!(removed, lease("svc", "2026-11-17T13:00:00Z"));
    let left: Vec<_> = list_registrations(store, &c, now())
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.key)
        .collect();
    assert_eq!(left.len(), 4);
    assert!(left.contains(&key("a-live", Some("2"))));
    assert!(!left.contains(&key("a-live", Some("1"))));
    // curated entries are removable too, and other catalogs are untouched
    remove_registration(store, &c, &key("b-curated", None))
        .await
        .unwrap();
    assert!(
        store
            .get_entry(&catalog("other"), &key("x", None))
            .await
            .unwrap()
            .is_some()
    );
}

async fn remove_of_a_missing_key_is_not_found(store: &dyn CatalogStore) {
    seed(store).await;
    let c = catalog("c");
    // the unversioned key is not any versioned entry
    assert_eq!(
        remove_registration(store, &c, &key("a-live", None)).await,
        Err(RemoveError::NotFound)
    );
    assert_eq!(
        remove_registration(store, &c, &key("nope", None)).await,
        Err(RemoveError::NotFound)
    );
    assert_eq!(list_registrations(store, &c, now()).await.unwrap().len(), 5);
}

async fn remove_loses_to_a_concurrent_refresh(store: &dyn CatalogStore) {
    let c = catalog("c");
    let k = key("svc", None);
    store
        .put_entry(
            &c,
            &record("svc", None),
            &lease("svc", "2026-11-17T11:00:00Z"),
            Condition::Absent,
        )
        .await
        .unwrap();
    let stale = store.get_entry(&c, &k).await.unwrap().unwrap().etag;
    store
        .put_entry(
            &c,
            &record("svc", None),
            &lease("svc", "2026-11-17T18:00:00Z"),
            Condition::Matches(stale.clone()),
        )
        .await
        .unwrap();
    // the conditioned delete the command issues fails on a version that has moved on
    assert_eq!(
        store.delete_entry(&c, &k, Some(&stale)).await,
        Err(StoreError::PreconditionFailed)
    );
    assert!(store.get_entry(&c, &k).await.unwrap().is_some());
}

macro_rules! checks {
    ($module:ident, $store:expr) => {
        mod $module {
            checks!(@one $store, list_reports_provenance_and_expiry_state);
            checks!(@one $store, remove_deletes_exactly_the_named_key);
            checks!(@one $store, remove_of_a_missing_key_is_not_found);
            checks!(@one $store, remove_loses_to_a_concurrent_refresh);
        }
    };
    (@one $store:expr, $name:ident) => {
        #[tokio::test]
        async fn $name() {
            super::$name(&$store).await;
        }
    };
}

#[cfg(feature = "in-memory")]
checks!(
    in_memory,
    turul_ai_catalog_storage::InMemoryCatalogStore::new()
);

#[cfg(feature = "sqlite")]
checks!(
    sqlite,
    turul_ai_catalog_storage::SqliteCatalogStore::connect(":memory:")
        .await
        .unwrap()
);
