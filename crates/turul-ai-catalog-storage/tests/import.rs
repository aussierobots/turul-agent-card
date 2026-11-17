#![cfg(feature = "in-memory")]

mod common;

use std::sync::Mutex;

use async_trait::async_trait;
use common::*;
use futures::executor::block_on;
use turul_ai_catalog::identity::EntryKey;
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::{Members, Signature};
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_storage::{
    CatalogId, CatalogStore, Condition, EntryRecord, Etag, HostRecord, ImportError, ImportMode,
    InMemoryCatalogStore, ItemCounts, ModeKind, Provenance, PurgeReport, ScannedEntry, StoreError,
    Stored, StoredEntry, assemble, import,
};

const DOC: &str = r#"{"specVersion":"1.0","host":{"displayName":"Op"},"entries":[{"identifier":"urn:air:a","type":"application/mcp-server-card+json","url":"https://example.com/a.json"}]}"#;

#[test]
fn import_loads_host_and_entries_and_assembles_the_same_document() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    let doc = parse(DOC.as_bytes()).unwrap();
    let etags = block_on(import(
        &store,
        &c,
        &doc,
        &writer("operator"),
        ImportMode::Merge,
    ))
    .unwrap();
    assert_eq!(etags.len(), 2);
    let out = block_on(assemble(&store, &c, now())).unwrap();
    assert_eq!(parse(&out.bytes).unwrap(), doc);
    assert_eq!(
        block_on(import(
            &store,
            &c,
            &doc,
            &writer("operator"),
            ImportMode::Merge
        ))
        .unwrap(),
        etags
    );
}

#[test]
fn signed_document_is_refused() {
    let mut doc = parse(DOC.as_bytes()).unwrap();
    doc.signature = Some(Signature::new(
        "did:web:example.com".to_string(),
        "did-web-v1".to_string(),
        "2026-01-01T00:00:00Z".to_string(),
        "a..b".to_string(),
    ));
    assert!(doc.signature.is_some());
    let store = InMemoryCatalogStore::new();
    assert_eq!(
        block_on(import(
            &store,
            &catalog("c"),
            &doc,
            &writer("operator"),
            ImportMode::Merge
        )),
        Err(ImportError::Signed)
    );
}

#[test]
fn top_level_extensions_are_refused() {
    let mut doc = parse(DOC.as_bytes()).unwrap();
    let mut ext = Members::default();
    ext.insert("com.example.x".to_string(), Json::Null);
    doc.extensions = Some(ext);
    let store = InMemoryCatalogStore::new();
    assert_eq!(
        block_on(import(
            &store,
            &catalog("c"),
            &doc,
            &writer("operator"),
            ImportMode::Merge
        )),
        Err(ImportError::Unstorable {
            pointer: "/extensions".to_string()
        })
    );
}

#[test]
fn unrecognized_top_level_members_are_refused() {
    let doc = parse(
        DOC.replace(r#""entries""#, r#""other":1,"entries""#)
            .as_bytes(),
    )
    .unwrap();
    let store = InMemoryCatalogStore::new();
    assert_eq!(
        block_on(import(
            &store,
            &catalog("c"),
            &doc,
            &writer("operator"),
            ImportMode::Merge
        )),
        Err(ImportError::Unstorable {
            pointer: "/other".to_string()
        })
    );
}

// #multi-version-entries: two entries with one entry key would collapse into one record; the
// document is refused before anything is written.
#[test]
fn duplicate_entries_are_refused_and_nothing_is_written() {
    let dup = r#"{"specVersion":"1.0","entries":[{"identifier":"urn:air:a","type":"t/x","url":"https://example.com/1"},{"identifier":"urn:air:a","type":"t/x","url":"https://example.com/2"}]}"#;
    let doc = parse(dup.as_bytes()).unwrap();
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    match block_on(import(
        &store,
        &c,
        &doc,
        &writer("operator"),
        ImportMode::Merge,
    )) {
        Err(ImportError::Invalid { errors }) => {
            assert!(
                errors.iter().any(|e| e.contains("/entries/1")),
                "{errors:?}"
            )
        }
        other => panic!("expected Invalid, got {other:?}"),
    }
    assert!(
        block_on(turul_ai_catalog_storage::CatalogStore::list_entries(
            &store, &c
        ))
        .unwrap()
        .is_empty()
    );
}

#[test]
fn import_records_curated_provenance() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    let doc = parse(DOC.as_bytes()).unwrap();
    let etags = block_on(import(
        &store,
        &c,
        &doc,
        &writer("operator"),
        ImportMode::Merge,
    ))
    .unwrap();
    let key = doc.entries[0].key();
    let got = block_on(store.get_entry(&c, &key)).unwrap().unwrap();
    assert_eq!(got.provenance, curated_by("operator"));
    // The host is first, then the entry whose version covers its provenance.
    assert_eq!(etags[1], got.etag);
}

#[test]
fn import_refuses_a_lease_record() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    let doc = parse(DOC.as_bytes()).unwrap();
    let leased = EntryRecord::new(doc.entries[0].clone()).unwrap();
    block_on(store.put_entry(
        &c,
        &leased,
        &lease("service", "2030-01-01T00:00:00Z"),
        Condition::Absent,
    ))
    .unwrap();
    assert_eq!(
        block_on(import(
            &store,
            &c,
            &doc,
            &writer("operator"),
            ImportMode::Merge
        )),
        Err(ImportError::Store(StoreError::ModeConflict {
            stored: ModeKind::Lease
        }))
    );
    let got = block_on(store.get_entry(&c, &leased.key()))
        .unwrap()
        .unwrap();
    assert_eq!(got.provenance, lease("service", "2030-01-01T00:00:00Z"));
}

#[test]
fn import_under_another_writer_id_is_refused() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    let doc = parse(DOC.as_bytes()).unwrap();
    block_on(import(
        &store,
        &c,
        &doc,
        &writer("first"),
        ImportMode::Merge,
    ))
    .unwrap();
    assert_eq!(
        block_on(import(
            &store,
            &c,
            &doc,
            &writer("second"),
            ImportMode::Merge
        )),
        Err(ImportError::OwnedByAnotherWriter)
    );
    let got = block_on(store.get_entry(&c, &doc.entries[0].key()))
        .unwrap()
        .unwrap();
    assert_eq!(got.provenance, curated_by("first"));
}

#[test]
fn import_replaces_its_own_changed_entry() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    let doc = parse(DOC.as_bytes()).unwrap();
    block_on(import(
        &store,
        &c,
        &doc,
        &writer("operator"),
        ImportMode::Merge,
    ))
    .unwrap();
    let mut changed = doc.clone();
    changed.entries[0].description = Some("changed".to_string());
    block_on(import(
        &store,
        &c,
        &changed,
        &writer("operator"),
        ImportMode::Merge,
    ))
    .unwrap();
    assert_eq!(
        parse(&block_on(assemble(&store, &c, now())).unwrap().bytes).unwrap(),
        changed
    );
}

fn replace() -> ImportMode {
    ImportMode::Replace { now: now() }
}

fn document(host: bool, entries: &[(&str, Option<&str>)]) -> turul_ai_catalog::model::AiCatalog {
    let mut doc = turul_ai_catalog::model::AiCatalog::new(
        entries.iter().map(|(i, v)| entry(i, *v)).collect(),
    );
    if host {
        doc.host = Some(host_info());
    }
    doc
}

fn host_info() -> turul_ai_catalog::model::HostInfo {
    turul_ai_catalog::model::HostInfo::new("Document Host".to_string())
}

fn put(store: &dyn CatalogStore, c: &CatalogId, id: &str, v: Option<&str>, p: &Provenance) {
    block_on(store.put_entry(c, &record(id, v), p, Condition::Absent)).unwrap();
}

fn keys(store: &dyn CatalogStore, c: &CatalogId) -> Vec<(String, Option<String>)> {
    let mut keys: Vec<_> = block_on(store.list_entries(c))
        .unwrap()
        .into_iter()
        .map(|s| {
            let k = s.record.key();
            (
                k.identifier().as_str().to_string(),
                k.version().map(str::to_string),
            )
        })
        .collect();
    keys.sort();
    keys
}

fn key(id: &str, v: Option<&str>) -> (String, Option<String>) {
    (id.to_string(), v.map(str::to_string))
}

#[test]
fn replace_makes_the_catalog_hold_exactly_the_document() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    block_on(
        store.put_host(
            &c,
            &HostRecord::new(turul_ai_catalog::model::HostInfo::new(
                "Old Host".to_string(),
            ))
            .unwrap(),
            Condition::Any,
        ),
    )
    .unwrap();
    put(&store, &c, "urn:air:a", None, &curated_by("other"));
    put(
        &store,
        &c,
        "urn:air:expired",
        None,
        &lease("svc", "2020-01-01T00:00:00Z"),
    );
    put(&store, &c, "urn:air:gone", Some("1"), &curated_by("other"));
    put(
        &store,
        &c,
        "urn:air:gone-lease",
        None,
        &lease("svc", "2020-01-01T00:00:00Z"),
    );
    let doc = document(
        true,
        &[
            ("urn:air:a", None),
            ("urn:air:expired", None),
            ("urn:air:new", Some("2")),
        ],
    );
    let etags = block_on(import(&store, &c, &doc, &writer("operator"), replace())).unwrap();
    assert_eq!(etags.len(), 4);
    assert_eq!(
        keys(&store, &c),
        vec![
            key("urn:air:a", None),
            key("urn:air:expired", None),
            key("urn:air:new", Some("2")),
        ]
    );
    for stored in block_on(store.list_entries(&c)).unwrap() {
        assert_eq!(stored.provenance, curated());
    }
    assert_eq!(
        block_on(store.get_host(&c))
            .unwrap()
            .unwrap()
            .record
            .bytes(),
        HostRecord::new(host_info()).unwrap().bytes()
    );
    let fresh = InMemoryCatalogStore::new();
    block_on(import(
        &fresh,
        &c,
        &doc,
        &writer("operator"),
        ImportMode::Merge,
    ))
    .unwrap();
    assert_eq!(
        block_on(assemble(&store, &c, now())).unwrap().bytes,
        block_on(assemble(&fresh, &c, now())).unwrap().bytes
    );
}

#[test]
fn replace_overwrites_an_expired_lease() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    put(
        &store,
        &c,
        "urn:air:a",
        None,
        &lease("svc", "2020-01-01T00:00:00Z"),
    );
    let doc = document(false, &[("urn:air:a", None)]);
    block_on(import(&store, &c, &doc, &writer("operator"), replace())).unwrap();
    let got = block_on(store.get_entry(&c, &doc.entries[0].key()))
        .unwrap()
        .unwrap();
    assert_eq!(got.provenance, curated());
}

#[test]
fn replace_changes_an_identifiers_version_kind() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    put(&store, &c, "urn:air:a", None, &curated_by("other"));
    put(&store, &c, "urn:air:b", Some("1"), &curated_by("other"));
    let doc = document(false, &[("urn:air:a", Some("1")), ("urn:air:b", None)]);
    block_on(import(&store, &c, &doc, &writer("operator"), replace())).unwrap();
    assert_eq!(
        keys(&store, &c),
        vec![key("urn:air:a", Some("1")), key("urn:air:b", None)]
    );
}

#[test]
fn replace_of_a_hostless_document_keeps_the_stored_host() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    block_on(store.put_host(&c, &host(), Condition::Any)).unwrap();
    let before = block_on(store.get_host(&c)).unwrap();
    block_on(import(
        &store,
        &c,
        &document(false, &[("urn:air:a", None)]),
        &writer("operator"),
        replace(),
    ))
    .unwrap();
    assert_eq!(block_on(store.get_host(&c)).unwrap(), before);
}

#[test]
fn replace_refuses_a_live_lease_and_writes_nothing() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    block_on(store.put_host(&c, &host(), Condition::Any)).unwrap();
    put(&store, &c, "urn:air:a", None, &curated_by("other"));
    put(
        &store,
        &c,
        "urn:air:z",
        None,
        &lease("svc", "2030-01-01T00:00:00Z"),
    );
    put(
        &store,
        &c,
        "urn:air:m",
        Some("1"),
        &lease("svc2", "2031-01-01T00:00:00Z"),
    );
    let snapshot = |s: &InMemoryCatalogStore| {
        let mut entries = block_on(s.list_entries(&c)).unwrap();
        entries.sort_by_key(|e| e.record.key());
        (entries, block_on(s.get_host(&c)).unwrap())
    };
    let before = snapshot(&store);
    let doc = document(true, &[("urn:air:a", None), ("urn:air:z", None)]);
    match block_on(import(&store, &c, &doc, &writer("operator"), replace())) {
        Err(ImportError::LiveLeases { leases }) => {
            let listed: Vec<_> = leases.iter().map(|(k, _)| k.clone()).collect();
            assert_eq!(
                listed,
                vec![
                    record("urn:air:m", Some("1")).key(),
                    record("urn:air:z", None).key()
                ]
            );
            assert_eq!(leases[0].1, lease("svc2", "2031-01-01T00:00:00Z"));
        }
        other => panic!("expected LiveLeases, got {other:?}"),
    }
    assert_eq!(snapshot(&store), before);
}

/// A lease whose `expires_at` is exactly `now` is expired, so it is replaced.
#[test]
fn replace_treats_a_lease_expiring_now_as_expired() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    put(
        &store,
        &c,
        "urn:air:a",
        None,
        &lease("svc", "2026-11-17T12:00:00Z"),
    );
    let doc = document(false, &[("urn:air:a", None)]);
    block_on(import(&store, &c, &doc, &writer("operator"), replace())).unwrap();
    assert_eq!(keys(&store, &c), vec![key("urn:air:a", None)]);
}

/// A store that logs each write and delete and assembles the catalog after it.
struct Recording {
    inner: InMemoryCatalogStore,
    log: Mutex<Vec<String>>,
    assembled: Mutex<Vec<Result<Vec<u8>, String>>>,
    write_before_scan_returns: Mutex<Option<(EntryRecord, Provenance)>>,
    delete_before_scan_returns: Mutex<Option<EntryKey>>,
}

impl Recording {
    fn new() -> Self {
        Self {
            inner: InMemoryCatalogStore::new(),
            log: Mutex::new(Vec::new()),
            assembled: Mutex::new(Vec::new()),
            write_before_scan_returns: Mutex::new(None),
            delete_before_scan_returns: Mutex::new(None),
        }
    }

    async fn step(&self, what: String) {
        self.log.lock().unwrap().push(what);
        let result = assemble(&self.inner, &catalog("c"), now())
            .await
            .map(|a| a.bytes)
            .map_err(|e| e.to_string());
        self.assembled.lock().unwrap().push(result);
    }
}

#[async_trait]
impl CatalogStore for Recording {
    fn backend_name(&self) -> &'static str {
        "recording"
    }
    async fn put_entry(
        &self,
        c: &CatalogId,
        r: &EntryRecord,
        p: &Provenance,
        k: Condition,
    ) -> Result<Etag, StoreError> {
        let result = self.inner.put_entry(c, r, p, k).await;
        if result.is_ok() {
            self.step(format!("put {}", r.key().identifier().as_str()))
                .await;
        }
        result
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
        let result = self.inner.delete_entry(c, k, e).await;
        self.step(format!("delete {}", k.identifier().as_str()))
            .await;
        result
    }
    async fn list_entries(&self, c: &CatalogId) -> Result<Vec<StoredEntry>, StoreError> {
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
        self.inner.get_host(c).await
    }
    async fn scan_entries(&self, c: &CatalogId) -> Result<Vec<ScannedEntry>, StoreError> {
        let scanned = self.inner.scan_entries(c).await?;
        let late = self.write_before_scan_returns.lock().unwrap().take();
        if let Some((record, provenance)) = late {
            self.inner
                .put_entry(c, &record, &provenance, Condition::Any)
                .await?;
        }
        let gone = self.delete_before_scan_returns.lock().unwrap().take();
        if let Some(key) = gone {
            self.inner.delete_entry(c, &key, None).await?;
        }
        Ok(scanned)
    }
    async fn count_catalog(&self, c: &CatalogId) -> Result<ItemCounts, StoreError> {
        self.inner.count_catalog(c).await
    }
    async fn purge_catalog(&self, c: &CatalogId) -> Result<PurgeReport, StoreError> {
        self.inner.purge_catalog(c).await
    }
}

/// Stored `a` and `b@1`, document `a@1`, `b`, `keep` and a new `n`, with `gone` stored and absent
/// from the document.
#[test]
fn replace_writes_before_it_deletes() {
    let store = Recording::new();
    let c = catalog("c");
    let o = curated_by("other");
    for (id, v) in [
        ("urn:air:a", None),
        ("urn:air:b", Some("1")),
        ("urn:air:keep", None),
        ("urn:air:gone", None),
    ] {
        block_on(
            store
                .inner
                .put_entry(&c, &record(id, v), &o, Condition::Absent),
        )
        .unwrap();
    }
    let doc = document(
        false,
        &[
            ("urn:air:a", Some("1")),
            ("urn:air:b", None),
            ("urn:air:keep", None),
            ("urn:air:n", None),
        ],
    );
    block_on(import(&store, &c, &doc, &writer("operator"), replace())).unwrap();
    let log = store.log.lock().unwrap().clone();
    let first_delete = log.iter().position(|l| l.starts_with("delete")).unwrap();
    let last_delete = log.iter().rposition(|l| l.starts_with("delete")).unwrap();
    // Before any delete: the entries whose kind did not change. After the last: the deferred ones.
    assert_eq!(
        log[..first_delete]
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        ["put urn:air:keep", "put urn:air:n"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );
    assert_eq!(
        log[last_delete + 1..]
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>(),
        ["put urn:air:a", "put urn:air:b"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );
    // No assembly during the replace failed or served an empty catalog, and `keep`, stored before
    // and of an unchanged kind, is served throughout.
    for assembled in store.assembled.lock().unwrap().iter() {
        let bytes = assembled
            .as_ref()
            .expect("an assembly during a replace never fails");
        let served = parse(bytes).unwrap();
        assert!(!served.entries.is_empty());
        assert!(
            served
                .entries
                .iter()
                .any(|e| e.identifier.as_str() == "urn:air:keep")
        );
    }
    assert_eq!(
        keys(&store.inner, &c),
        vec![
            key("urn:air:a", Some("1")),
            key("urn:air:b", None),
            key("urn:air:keep", None),
            key("urn:air:n", None),
        ]
    );
}

#[test]
fn replace_stops_when_a_record_changed_after_the_scan() {
    let store = Recording::new();
    let c = catalog("c");
    put(&store.inner, &c, "urn:air:a", None, &curated_by("other"));
    let absent = record("urn:air:absent", None);
    block_on(
        store
            .inner
            .put_entry(&c, &absent, &curated_by("other"), Condition::Absent),
    )
    .unwrap();
    let mut changed = entry("urn:air:absent", None);
    changed.description = Some("rewritten".to_string());
    *store.write_before_scan_returns.lock().unwrap() = Some((
        EntryRecord::new(changed).unwrap(),
        curated_by("late-writer"),
    ));
    let doc = document(false, &[("urn:air:a", None)]);
    assert_eq!(
        block_on(import(&store, &c, &doc, &writer("operator"), replace())),
        Err(ImportError::Store(StoreError::PreconditionFailed))
    );
    let kept = block_on(store.inner.get_entry(&c, &absent.key()))
        .unwrap()
        .unwrap();
    assert_eq!(kept.provenance, curated_by("late-writer"));
}

#[test]
fn replace_does_not_overwrite_a_lease_registered_after_the_scan() {
    let store = Recording::new();
    let c = catalog("c");
    let late = record("urn:air:late", None);
    let registered = lease("svc", "2999-01-01T00:00:00Z");
    *store.write_before_scan_returns.lock().unwrap() = Some((late.clone(), registered.clone()));
    let doc = document(false, &[("urn:air:late", None)]);
    assert_eq!(
        block_on(import(&store, &c, &doc, &writer("operator"), replace())),
        Err(ImportError::Store(StoreError::PreconditionFailed))
    );
    let kept = block_on(store.inner.get_entry(&c, &late.key()))
        .unwrap()
        .unwrap();
    assert_eq!(kept.provenance, registered);
}

#[test]
fn replace_does_not_overwrite_a_lease_refreshed_after_the_scan() {
    let store = Recording::new();
    let c = catalog("c");
    let a = record("urn:air:a", None);
    put(
        &store.inner,
        &c,
        "urn:air:a",
        None,
        &lease("svc", "2000-01-01T00:00:00Z"),
    );
    let refreshed = lease("svc", "2999-01-01T00:00:00Z");
    *store.write_before_scan_returns.lock().unwrap() = Some((a.clone(), refreshed.clone()));
    let doc = document(false, &[("urn:air:a", None)]);
    assert_eq!(
        block_on(import(&store, &c, &doc, &writer("operator"), replace())),
        Err(ImportError::Store(StoreError::PreconditionFailed))
    );
    let kept = block_on(store.inner.get_entry(&c, &a.key()))
        .unwrap()
        .unwrap();
    assert_eq!(kept.provenance, refreshed);
}

#[test]
fn replace_tolerates_an_absent_entry_deleted_after_the_scan() {
    let store = Recording::new();
    let c = catalog("c");
    put(&store.inner, &c, "urn:air:a", None, &curated_by("other"));
    put(&store.inner, &c, "urn:air:gone", None, &curated_by("other"));
    *store.delete_before_scan_returns.lock().unwrap() = Some(record("urn:air:gone", None).key());
    let doc = document(false, &[("urn:air:a", None)]);
    block_on(import(&store, &c, &doc, &writer("operator"), replace())).unwrap();
    assert_eq!(keys(&store.inner, &c), vec![key("urn:air:a", None)]);
}
