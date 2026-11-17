//! `SnapshotCache`: a stale snapshot is re-assembled, a failed re-assembly keeps the last good one
//! and reports the failure.

mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::executor::block_on;
use support::{body_bytes, catalog_id, request, store_with};
use turul_ai_catalog::identity::{EntryKey, WELL_KNOWN_PATH};
use turul_ai_catalog_lambda::{ColdStartError, ServedCatalog, SnapshotCache, handle};
use turul_ai_catalog_storage::{
    AssembleError, CatalogId, CatalogStore, Condition, EntryRecord, Etag, HostRecord,
    InMemoryCatalogStore, StoreError, Stored,
};

const ONE: &str = r#"{"specVersion":"1.0","entries":[{"identifier":"urn:example:a","type":"application/a2a-agent-card+json","url":"https://example.com/a.json"}]}"#;

fn second_entry() -> EntryRecord {
    let mut doc = turul_ai_catalog::parse::parse(ONE.as_bytes()).expect("parses");
    let mut entry = doc.entries.remove(0);
    entry.identifier = turul_ai_catalog::identity::EntryIdentifier::new("urn:example:b");
    EntryRecord::new(entry).expect("record")
}

/// An in-memory store that can be made to fail every read.
struct Switch {
    inner: InMemoryCatalogStore,
    down: AtomicBool,
}

impl Switch {
    fn check(&self) -> Result<(), StoreError> {
        if self.down.load(Ordering::SeqCst) {
            Err(StoreError::Backend("down".into()))
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl CatalogStore for Switch {
    fn backend_name(&self) -> &'static str {
        "switch"
    }
    async fn put_entry(
        &self,
        c: &CatalogId,
        r: &EntryRecord,
        k: Condition,
    ) -> Result<Etag, StoreError> {
        self.inner.put_entry(c, r, k).await
    }
    async fn get_entry(
        &self,
        c: &CatalogId,
        k: &EntryKey,
    ) -> Result<Option<Stored<EntryRecord>>, StoreError> {
        self.inner.get_entry(c, k).await
    }
    async fn delete_entry(
        &self,
        c: &CatalogId,
        k: &EntryKey,
        e: Option<&Etag>,
    ) -> Result<(), StoreError> {
        self.inner.delete_entry(c, k, e).await
    }
    async fn list_entries(&self, c: &CatalogId) -> Result<Vec<Stored<EntryRecord>>, StoreError> {
        self.check()?;
        self.inner.list_entries(c).await
    }
    async fn put_host(
        &self,
        c: &CatalogId,
        r: &HostRecord,
        k: Condition,
    ) -> Result<Etag, StoreError> {
        self.inner.put_host(c, r, k).await
    }
    async fn get_host(&self, c: &CatalogId) -> Result<Option<Stored<HostRecord>>, StoreError> {
        self.check()?;
        self.inner.get_host(c).await
    }
}

fn setup(max_age: Duration) -> (Switch, SnapshotCache) {
    let store = Switch {
        inner: store_with(ONE),
        down: AtomicBool::new(false),
    };
    let served = block_on(ServedCatalog::assemble(&store, &catalog_id())).expect("assembles");
    (store, SnapshotCache::new(served, max_age))
}

fn add_second(store: &Switch) {
    block_on(store.put_entry(&catalog_id(), &second_entry(), Condition::Any)).expect("put");
}

fn body(served: &ServedCatalog) -> Vec<u8> {
    body_bytes(&handle(served, &request("GET", WELL_KNOWN_PATH)))
}

#[test]
fn fresh_snapshot_is_not_reassembled() {
    let (store, cache) = setup(Duration::from_secs(3600));
    let before = body(&block_on(cache.current(&store, &catalog_id())).0);
    add_second(&store);
    let after = body(&block_on(cache.current(&store, &catalog_id())).0);
    assert_eq!(before, after);
}

#[test]
fn stale_snapshot_is_reassembled() {
    let (store, cache) = setup(Duration::ZERO);
    let before = body(&block_on(cache.current(&store, &catalog_id())).0);
    add_second(&store);
    let after = body(&block_on(cache.current(&store, &catalog_id())).0);
    assert_ne!(before, after);
    assert!(
        String::from_utf8(after)
            .expect("utf8")
            .contains("urn:example:b")
    );
}

#[test]
fn failed_reassembly_keeps_the_last_good_snapshot() {
    let (store, cache) = setup(Duration::ZERO);
    let before = body(&block_on(cache.current(&store, &catalog_id())).0);
    add_second(&store);
    store.down.store(true, Ordering::SeqCst);
    let (during, error) = block_on(cache.current(&store, &catalog_id()));
    assert_eq!(before, body(&during));
    assert!(matches!(
        error,
        Some(ColdStartError::Assemble(AssembleError::Store(_)))
    ));
    store.down.store(false, Ordering::SeqCst);
    let after = body(&block_on(cache.current(&store, &catalog_id())).0);
    assert!(
        String::from_utf8(after)
            .expect("utf8")
            .contains("urn:example:b")
    );
}

// #multi-version-entries: a versioned `urn:example:a` beside the stored unversioned one breaks
// "`identifier` alone MUST be unique". The store accepts the record; the refresh fails, keeps the
// last good snapshot and reports why.
#[test]
fn cross_entry_error_keeps_the_last_good_snapshot_and_is_reported() {
    let (store, cache) = setup(Duration::ZERO);
    let before = body(&block_on(cache.current(&store, &catalog_id())).0);
    let mut doc = turul_ai_catalog::parse::parse(ONE.as_bytes()).expect("parses");
    let mut versioned = doc.entries.remove(0);
    versioned.version = Some("1".to_string());
    block_on(store.put_entry(
        &catalog_id(),
        &EntryRecord::new(versioned).expect("entry passes the record gate"),
        Condition::Absent,
    ))
    .expect("the store accepts the record");
    let (during, error) = block_on(cache.current(&store, &catalog_id()));
    assert_eq!(before, body(&during));
    assert!(matches!(
        error,
        Some(ColdStartError::Assemble(AssembleError::Invalid(_)))
    ));
}

// A lasting refresh failure is retried and reported once per `max_age`, not on every request.
#[test]
fn failed_refresh_is_retried_once_per_max_age() {
    let (store, cache) = setup(Duration::from_millis(50));
    store.down.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(60));
    let (_, first) = block_on(cache.current(&store, &catalog_id()));
    assert!(
        first.is_some(),
        "a stale snapshot retries and reports the failure"
    );
    let (_, second) = block_on(cache.current(&store, &catalog_id()));
    assert!(
        second.is_none(),
        "the next request within max_age does not retry"
    );
    std::thread::sleep(Duration::from_millis(60));
    let (_, third) = block_on(cache.current(&store, &catalog_id()));
    assert!(
        third.is_some(),
        "after another max_age the failure is retried and reported again"
    );
}
