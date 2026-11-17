//! `SnapshotCache`: a stale snapshot is re-assembled, a failed re-assembly keeps the last good one
//! and reports the failure.

mod support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::executor::block_on;
use support::{body_bytes, catalog_id, fixture_writer, request, store_with};
use turul_ai_catalog::identity::{EntryKey, WELL_KNOWN_PATH};
use turul_ai_catalog::timestamp::Timestamp;
use turul_ai_catalog_lambda::{ColdStartError, ServedCatalog, SnapshotCache, handle};
use turul_ai_catalog_storage::{
    AssembleError, CatalogId, CatalogStore, Condition, EntryRecord, Etag, HostRecord,
    InMemoryCatalogStore, Provenance, StoreError, Stored, StoredEntry,
};

const ONE: &str = r#"{"specVersion":"1.0","entries":[{"identifier":"urn:example:a","type":"application/a2a-agent-card+json","url":"https://example.com/a.json"}]}"#;

fn curated() -> Provenance {
    Provenance::Curated {
        writer: fixture_writer(),
    }
}

fn second_entry() -> EntryRecord {
    let mut doc = turul_ai_catalog::parse::parse(ONE.as_bytes()).expect("parses");
    let mut entry = doc.entries.remove(0);
    entry.identifier = turul_ai_catalog::identity::EntryIdentifier::new("urn:example:b");
    EntryRecord::new(entry).expect("record")
}

/// An in-memory store that can be made to fail every read, or to list beside its own records one
/// that its `put_entry` refuses, as a store written before that refusal can.
struct Switch {
    inner: InMemoryCatalogStore,
    down: AtomicBool,
    written_before_the_guard: Mutex<Option<EntryRecord>>,
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
        p: &Provenance,
        k: Condition,
    ) -> Result<Etag, StoreError> {
        self.inner.put_entry(c, r, p, k).await
    }
    async fn get_entry(
        &self,
        c: &CatalogId,
        k: &EntryKey,
    ) -> Result<Option<StoredEntry>, StoreError> {
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
    async fn list_entries(&self, c: &CatalogId) -> Result<Vec<StoredEntry>, StoreError> {
        self.check()?;
        let mut stored = self.inner.list_entries(c).await?;
        if let Some(record) = self.written_before_the_guard.lock().expect("lock").clone() {
            let provenance = curated();
            stored.push(StoredEntry {
                etag: Etag::of_entry(&record, &provenance).unwrap(),
                provenance,
                record,
            });
        }
        Ok(stored)
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

/// A clock the test moves.
#[derive(Clone)]
struct Clock(Arc<Mutex<SystemTime>>);

impl Clock {
    fn new() -> Self {
        // 2026-11-17T12:00:00Z
        Self(Arc::new(Mutex::new(
            UNIX_EPOCH + Duration::from_secs(1_794_916_800),
        )))
    }
    fn now(&self) -> SystemTime {
        *self.0.lock().expect("lock")
    }
    fn advance(&self, by: Duration) {
        *self.0.lock().expect("lock") += by;
    }
    fn reader(&self) -> impl Fn() -> SystemTime + Send + Sync + 'static {
        let clock = self.clone();
        move || clock.now()
    }
}

fn switch() -> Switch {
    Switch {
        inner: store_with(ONE),
        down: AtomicBool::new(false),
        written_before_the_guard: Mutex::new(None),
    }
}

fn cache_over(store: &Switch, clock: &Clock, max_age: Duration) -> SnapshotCache {
    let served =
        block_on(ServedCatalog::assemble(store, &catalog_id(), clock.now())).expect("assembles");
    SnapshotCache::new(served, max_age, clock.reader())
}

fn setup(max_age: Duration) -> (Switch, SnapshotCache, Clock) {
    let (store, clock) = (switch(), Clock::new());
    let cache = cache_over(&store, &clock, max_age);
    (store, cache, clock)
}

/// Stores `urn:example:b` as a lease that ends `ttl` after the clock's current time.
fn add_lease(store: &Switch, clock: &Clock, ttl: Duration) {
    let provenance = Provenance::Lease {
        writer: fixture_writer(),
        expires_at: Timestamp::from_system_time(clock.now() + ttl),
    };
    block_on(store.put_entry(&catalog_id(), &second_entry(), &provenance, Condition::Any))
        .expect("put");
}

fn add_second(store: &Switch) {
    block_on(store.put_entry(&catalog_id(), &second_entry(), &curated(), Condition::Any))
        .expect("put");
}

fn body(served: &ServedCatalog) -> Vec<u8> {
    body_bytes(&handle(served, &request("GET", WELL_KNOWN_PATH)))
}

#[test]
fn fresh_snapshot_is_not_reassembled() {
    let (store, cache, _) = setup(Duration::from_secs(3600));
    let before = body(&block_on(cache.current(&store, &catalog_id())).0);
    add_second(&store);
    let after = body(&block_on(cache.current(&store, &catalog_id())).0);
    assert_eq!(before, after);
}

#[test]
fn stale_snapshot_is_reassembled() {
    let (store, cache, _) = setup(Duration::ZERO);
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
    let (store, cache, _) = setup(Duration::ZERO);
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
// "`identifier` alone MUST be unique". The store refuses that put, so the record is listed as a store
// written before the refusal lists it; the refresh fails, keeps the last good snapshot and reports
// why.
#[test]
fn cross_entry_error_keeps_the_last_good_snapshot_and_is_reported() {
    let (store, cache, _) = setup(Duration::ZERO);
    let before = body(&block_on(cache.current(&store, &catalog_id())).0);
    let mut doc = turul_ai_catalog::parse::parse(ONE.as_bytes()).expect("parses");
    let mut versioned = doc.entries.remove(0);
    versioned.version = Some("1".to_string());
    let versioned = EntryRecord::new(versioned).expect("entry passes the record gate");
    assert!(matches!(
        block_on(store.put_entry(&catalog_id(), &versioned, &curated(), Condition::Absent)),
        Err(StoreError::VersionKindConflict { .. })
    ));
    *store.written_before_the_guard.lock().expect("lock") = Some(versioned);
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
    let max_age = Duration::from_secs(50);
    let (store, cache, clock) = setup(max_age);
    store.down.store(true, Ordering::SeqCst);
    clock.advance(max_age);
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
    clock.advance(max_age);
    let (_, third) = block_on(cache.current(&store, &catalog_id()));
    assert!(
        third.is_some(),
        "after another max_age the failure is retried and reported again"
    );
}

fn serves_b(served: &ServedCatalog) -> bool {
    String::from_utf8(body(served))
        .expect("utf8")
        .contains("urn:example:b")
}

// A lease expiring before `max_age` is gone at the first request at or after its expiry, not a
// whole `max_age` later.
#[test]
fn snapshot_is_reassembled_at_the_next_lease_expiry() {
    let (store, clock) = (switch(), Clock::new());
    add_lease(&store, &clock, Duration::from_secs(10));
    let cache = cache_over(&store, &clock, Duration::from_secs(3600));
    assert!(serves_b(&block_on(cache.current(&store, &catalog_id())).0));
    clock.advance(Duration::from_millis(9_999));
    assert!(serves_b(&block_on(cache.current(&store, &catalog_id())).0));
    clock.advance(Duration::from_millis(1));
    let (at_expiry, error) = block_on(cache.current(&store, &catalog_id()));
    assert!(error.is_none() && !serves_b(&at_expiry));
}

// With `max_age` shorter than the lease, the lease leaves at the next refresh.
#[test]
fn expired_lease_leaves_the_served_catalog_at_the_next_refresh() {
    let (store, cache, clock) = setup(Duration::from_secs(60));
    add_lease(&store, &clock, Duration::from_secs(30));
    clock.advance(Duration::from_secs(20));
    let (served, _) = block_on(cache.current(&store, &catalog_id()));
    assert!(!serves_b(&served), "the cache is not yet due");
    clock.advance(Duration::from_secs(40));
    let (served, _) = block_on(cache.current(&store, &catalog_id()));
    assert!(!serves_b(&served));
    assert_eq!(served.omitted().len(), 1);
}

// Each assembly carries its own omission list: a snapshot reports what its assembly left out.
#[test]
fn omissions_are_reported_on_each_assembly() {
    let (store, cache, clock) = setup(Duration::from_secs(60));
    let (first, _) = block_on(cache.current(&store, &catalog_id()));
    assert!(first.omitted().is_empty());
    add_lease(&store, &clock, Duration::from_secs(90));
    clock.advance(Duration::from_secs(60));
    let (second, _) = block_on(cache.current(&store, &catalog_id()));
    assert!(serves_b(&second) && second.omitted().is_empty());
    clock.advance(Duration::from_secs(60));
    let (third, _) = block_on(cache.current(&store, &catalog_id()));
    let lines: Vec<_> = third.omitted().iter().map(ToString::to_string).collect();
    assert_eq!(lines.len(), 1);
    assert!(
        lines[0].contains("urn:example:b")
            && lines[0].contains("fixture")
            && lines[0].contains("lease expired"),
        "{lines:?}"
    );
}

// A failed refresh after an expiry keeps the last good snapshot and retries once per `max_age`.
#[test]
fn failed_refresh_after_an_expiry_is_retried_once_per_max_age() {
    let max_age = Duration::from_secs(3600);
    let (store, clock) = (switch(), Clock::new());
    add_lease(&store, &clock, Duration::from_secs(10));
    let cache = cache_over(&store, &clock, max_age);
    store.down.store(true, Ordering::SeqCst);
    clock.advance(Duration::from_secs(10));
    let (kept, first) = block_on(cache.current(&store, &catalog_id()));
    assert!(
        serves_b(&kept) && first.is_some(),
        "the failure is reported"
    );
    clock.advance(Duration::from_secs(10));
    let (_, second) = block_on(cache.current(&store, &catalog_id()));
    assert!(second.is_none(), "no retry within max_age");
    clock.advance(max_age);
    let (_, third) = block_on(cache.current(&store, &catalog_id()));
    assert!(third.is_some(), "retried after max_age");
}
