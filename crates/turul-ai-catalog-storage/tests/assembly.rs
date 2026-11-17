#![cfg(feature = "in-memory")]

mod common;

use async_trait::async_trait;
use common::*;
use futures::executor::block_on;
use turul_ai_catalog::identity::EntryKey;
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_storage::{
    AssembleError, CatalogId, CatalogStore, Condition, EntryRecord, Etag, HostRecord,
    InMemoryCatalogStore, OmissionReason, Provenance, StoreError, Stored, StoredEntry, assemble,
};

#[test]
fn empty_store_assembles_to_a_valid_empty_catalog() {
    let store = InMemoryCatalogStore::new();
    let out = block_on(assemble(&store, &catalog("c"), now())).unwrap();
    assert_eq!(out.bytes, br#"{"entries":[],"specVersion":"1.0"}"#);
    assert!(!out.report.has_errors());
}

#[test]
fn assembled_document_has_no_signature() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    block_on(store.put_host(&c, &host(), Condition::Any)).unwrap();
    block_on(store.put_entry(&c, &record("urn:air:a", None), &curated(), Condition::Any)).unwrap();
    let out = block_on(assemble(&store, &c, now())).unwrap();
    let doc = parse(&out.bytes).unwrap();
    assert!(doc.signature.is_none() && doc.additional_signatures.is_none());
    assert!(doc.host.is_some() && doc.extensions.is_none());
}

#[test]
fn entries_are_ordered_by_entry_key() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    for (id, v) in [
        ("c", None),
        ("b", Some("1")),
        ("a", Some("2")),
        ("a", Some("1")),
    ] {
        block_on(store.put_entry(&c, &record(id, v), &curated(), Condition::Any)).unwrap();
    }
    let doc = parse(&block_on(assemble(&store, &c, now())).unwrap().bytes).unwrap();
    let order: Vec<_> = doc
        .entries
        .iter()
        .map(|e| (e.identifier.as_str().to_string(), e.version.clone()))
        .collect();
    let want = [
        ("a", Some("1")),
        ("a", Some("2")),
        ("b", Some("1")),
        ("c", None),
    ]
    .map(|(i, v)| (i.to_string(), v.map(str::to_string)));
    assert_eq!(order, want);
}

// the spec rejects an unversioned entry beside versioned ones, so absent-before-present is the
// derived key order, asserted on keys.
#[test]
fn absent_version_orders_before_present_versions() {
    assert!(entry("a", None).key() < entry("a", Some("")).key());
    assert!(entry("a", Some("9")).key() < entry("b", None).key());
}

/// A store whose records were written before `put_entry` refused other-kind entries: it lists
/// whatever it was given.
struct Listed(Vec<EntryRecord>);

fn keys_of(doc: &turul_ai_catalog::model::AiCatalog) -> Vec<EntryKey> {
    doc.entries.iter().map(|e| e.key()).collect()
}

#[async_trait]
impl CatalogStore for Listed {
    fn backend_name(&self) -> &'static str {
        "listed"
    }
    async fn put_entry(
        &self,
        _: &CatalogId,
        _: &EntryRecord,
        _: &Provenance,
        _: Condition,
    ) -> Result<Etag, StoreError> {
        unreachable!()
    }
    async fn get_entry(
        &self,
        _: &CatalogId,
        _: &EntryKey,
    ) -> Result<Option<StoredEntry>, StoreError> {
        unreachable!()
    }
    async fn delete_entry(
        &self,
        _: &CatalogId,
        _: &EntryKey,
        _: Option<&Etag>,
    ) -> Result<(), StoreError> {
        unreachable!()
    }
    async fn list_entries(&self, _: &CatalogId) -> Result<Vec<StoredEntry>, StoreError> {
        Ok(self
            .0
            .iter()
            .map(|r| StoredEntry {
                record: r.clone(),
                provenance: curated(),
                etag: Etag::of_entry(r, &curated()).unwrap(),
            })
            .collect())
    }
    async fn put_host(
        &self,
        _: &CatalogId,
        _: &HostRecord,
        _: Condition,
    ) -> Result<Etag, StoreError> {
        unreachable!()
    }
    async fn get_host(&self, _: &CatalogId) -> Result<Option<Stored<HostRecord>>, StoreError> {
        Ok(None)
    }
}

// #multi-version-entries: with `version` absent, "`identifier` alone MUST be unique". Every
// backend refuses the second put, but records stored before it did so still reach assembly, whose
// whole-document validation refuses them.
#[test]
fn cross_entry_error_still_fails_assembly() {
    let store = Listed(vec![record("a", None), record("a", Some("1"))]);
    let Err(AssembleError::Invalid(report)) = block_on(assemble(&store, &catalog("c"), now()))
    else {
        panic!("assembly must refuse an unversioned entry beside a versioned one");
    };
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|d| d.code.as_str() == "version-missing-in-group")
    );
}

const EXPIRES: &str = "2026-11-17T12:00:00Z";

fn put(store: &InMemoryCatalogStore, c: &CatalogId, r: &EntryRecord, p: &Provenance) {
    block_on(store.put_entry(c, r, p, Condition::Any)).unwrap();
}

// An expired lease is left out of the served document and reported
// with its key, provenance and reason, never dropped silently.
#[test]
fn expired_lease_is_omitted_and_reported() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    put(&store, &c, &record("urn:air:a", None), &curated());
    let expired = lease("w", "2026-11-17T11:59:59Z");
    put(&store, &c, &record("urn:air:b", None), &expired);
    let out = block_on(assemble(&store, &c, now())).unwrap();
    let doc = parse(&out.bytes).unwrap();
    assert_eq!(keys_of(&doc), [record("urn:air:a", None).key()]);
    assert_eq!(out.omitted.len(), 1);
    let o = &out.omitted[0];
    assert_eq!(o.key, record("urn:air:b", None).key());
    assert_eq!(o.provenance, expired);
    assert!(matches!(
        &o.reason,
        OmissionReason::LeaseExpired { expires_at } if *expires_at == ts("2026-11-17T11:59:59Z")
    ));
}

// A lease is live while `now < expires_at`, the half-open window of `expiresAt`.
#[test]
fn lease_is_live_until_expires_at() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    put(&store, &c, &record("urn:air:b", None), &lease("w", EXPIRES));
    let live = block_on(assemble(&store, &c, now_shifted(1, true))).unwrap();
    assert_eq!(parse(&live.bytes).unwrap().entries.len(), 1);
    assert!(live.omitted.is_empty());
    let at = block_on(assemble(&store, &c, now())).unwrap();
    assert!(parse(&at.bytes).unwrap().entries.is_empty());
    assert_eq!(at.omitted.len(), 1);
}

// Every listed record is in the document or in the omissions, never both, never neither.
#[test]
fn every_listed_entry_is_served_or_omitted() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    put(&store, &c, &record("urn:air:a", None), &curated());
    put(
        &store,
        &c,
        &record("urn:air:b", None),
        &lease("w", "2026-11-17T13:00:00Z"),
    );
    put(
        &store,
        &c,
        &record("urn:air:c", None),
        &lease("w", "2026-11-17T11:00:00Z"),
    );
    put(
        &store,
        &c,
        &record("urn:air:d", Some("1")),
        &lease("w", EXPIRES),
    );
    let out = block_on(assemble(&store, &c, now())).unwrap();
    let mut seen = keys_of(&parse(&out.bytes).unwrap());
    let served = seen.len();
    seen.extend(out.omitted.iter().map(|o| o.key.clone()));
    seen.sort();
    let mut listed: Vec<_> = block_on(store.list_entries(&c))
        .unwrap()
        .iter()
        .map(|s| s.record.key())
        .collect();
    listed.sort();
    assert_eq!(seen, listed);
    assert_eq!((served, out.omitted.len()), (2, 2));
}

// #catalog-snapshot-coverage: changing a contribution changes the snapshot; the document still
// carries no catalog signature.
#[test]
fn lease_expiry_changes_the_assembled_bytes_and_adds_no_signature() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    put(&store, &c, &record("urn:air:a", None), &curated());
    put(&store, &c, &record("urn:air:b", None), &lease("w", EXPIRES));
    let before = block_on(assemble(&store, &c, now_shifted(1, true))).unwrap();
    let after = block_on(assemble(&store, &c, now())).unwrap();
    assert_ne!(before.bytes, after.bytes);
    for out in [before, after] {
        let doc = parse(&out.bytes).unwrap();
        assert!(doc.signature.is_none() && doc.additional_signatures.is_none());
    }
}

// #dynamic-discovery: expiry depends on the clock, never on the caller. Equal records and equal
// `now` give equal output, whatever the write order, and a `now` that crosses no expiry changes
// nothing.
#[test]
fn assembly_depends_only_on_records_and_now() {
    let records = [
        (record("urn:air:a", None), curated()),
        (
            record("urn:air:b", None),
            lease("w", "2026-11-17T13:00:00Z"),
        ),
        (
            record("urn:air:c", None),
            lease("w", "2026-11-17T11:00:00Z"),
        ),
    ];
    let c = catalog("c");
    let forward = InMemoryCatalogStore::new();
    let backward = InMemoryCatalogStore::new();
    for (r, p) in &records {
        put(&forward, &c, r, p);
    }
    for (r, p) in records.iter().rev() {
        put(&backward, &c, r, p);
    }
    let a = block_on(assemble(&forward, &c, now())).unwrap();
    assert_eq!(a, block_on(assemble(&backward, &c, now())).unwrap());
    assert_eq!(a, block_on(assemble(&forward, &c, now())).unwrap());
    let later = block_on(assemble(&forward, &c, now_shifted(60_000_000_000, false))).unwrap();
    assert_eq!(a, later);
}

#[test]
fn next_expiry_is_the_earliest_served_lease_expiry() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    put(&store, &c, &record("urn:air:a", None), &curated());
    assert_eq!(
        block_on(assemble(&store, &c, now())).unwrap().next_expiry,
        None
    );
    put(
        &store,
        &c,
        &record("urn:air:e", None),
        &lease("w", "2026-11-17T11:00:00Z"),
    );
    let only_dead = block_on(assemble(&store, &c, now())).unwrap();
    assert_eq!((only_dead.next_expiry, only_dead.omitted.len()), (None, 1));
    put(
        &store,
        &c,
        &record("urn:air:b", None),
        &lease("w", "2026-11-17T12:00:10Z"),
    );
    put(
        &store,
        &c,
        &record("urn:air:c", None),
        &lease("w", "2026-11-17T12:00:05Z"),
    );
    let out = block_on(assemble(&store, &c, now())).unwrap();
    assert_eq!(out.next_expiry, Some(ts("2026-11-17T12:00:05Z")));
}

// An entry that passed an earlier release's gate and fails the current one is omitted and
// reported with its own report; the rest stays served. Only a backend that reads stored bytes
// without gating them can hold one, so the row is written raw.
#[cfg(feature = "sqlite")]
#[tokio::test]
async fn entry_failing_the_current_gate_is_omitted_and_reported() {
    use turul_ai_catalog_storage::SqliteCatalogStore;
    let dir = std::env::temp_dir().join(format!("catalog-assembly-gate-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("catalog.db");
    let path = path.to_str().unwrap();
    let c = catalog("c");
    let bad = record("urn:air:bad", None);
    {
        let store = SqliteCatalogStore::connect(path).await.unwrap();
        store
            .put_entry(
                &c,
                &record("urn:air:good", None),
                &curated(),
                Condition::Absent,
            )
            .await
            .unwrap();
        store
            .put_entry(&c, &bad, &curated(), Condition::Absent)
            .await
            .unwrap();
        let pool = sqlx::SqlitePool::connect(&format!("sqlite://{path}"))
            .await
            .unwrap();
        let tampered = String::from_utf8(bad.bytes().to_vec())
            .unwrap()
            .replace("https://example.com/card.json", "");
        sqlx::query(
            "UPDATE catalog_records SET record = ?1 WHERE catalog_id = ?2 AND record_key = ?3",
        )
        .bind(tampered.into_bytes())
        .bind(c.as_str())
        .bind(bad.record_key().unwrap().as_str())
        .execute(&pool)
        .await
        .unwrap();
        pool.close().await;
        let out = assemble(&store, &c, now()).await.unwrap();
        assert_eq!(
            keys_of(&parse(&out.bytes).unwrap()),
            [record("urn:air:good", None).key()]
        );
        assert_eq!(out.omitted.len(), 1);
        assert_eq!(out.omitted[0].key, bad.key());
        let OmissionReason::Invalid(report) = &out.omitted[0].reason else {
            panic!("the entry must be omitted as invalid");
        };
        assert!(report.has_errors());
    }
    std::fs::remove_dir_all(&dir).unwrap();
}
