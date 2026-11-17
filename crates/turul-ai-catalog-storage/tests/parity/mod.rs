//! The backend contract. Every backend's test file calls every check against its own store,
//! each check in a catalog of its own so backends sharing a table do not interfere.

#![allow(dead_code)]

use std::sync::Arc;

use futures::future::join_all;
use turul_ai_catalog::identity::EntryIdentifier;
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::{ArtifactSource, Members};
use turul_ai_catalog::parse::parse_entry;
use turul_ai_catalog_storage::{CatalogStore, Condition, Etag, StoreError, assemble};

use crate::common::*;

/// Names the catalog of one check. The run suffix keeps a re-run against a persistent database
/// from finding the previous run's records.
fn id(check: &str) -> turul_ai_catalog_storage::CatalogId {
    static RUN: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let run = RUN.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        format!("{}-{nanos}", std::process::id())
    });
    catalog(&format!("parity.{check}.{run}"))
}

pub async fn one_record_per_entry_key(store: &dyn CatalogStore) {
    let c = id("one-record-per-entry-key");
    let r = record("urn:air:a", Some("1"));
    store.put_entry(&c, &r, Condition::Any).await.unwrap();
    store.put_entry(&c, &r, Condition::Any).await.unwrap();
    assert_eq!(store.list_entries(&c).await.unwrap().len(), 1);
    store
        .put_entry(&c, &record("urn:air:a", Some("2")), Condition::Any)
        .await
        .unwrap();
    assert_eq!(store.list_entries(&c).await.unwrap().len(), 2);
    // An unversioned entry is keyed on its identifier alone.
    let b = record("urn:air:b", None);
    store.put_entry(&c, &b, Condition::Any).await.unwrap();
    store.put_entry(&c, &b, Condition::Any).await.unwrap();
    assert_eq!(store.list_entries(&c).await.unwrap().len(), 3);
    assert!(assemble(store, &c).await.is_ok());
}

pub async fn stored_entry_reads_back_byte_identical(store: &dyn CatalogStore) {
    let c = id("stored-entry-reads-back-byte-identical");
    let mut e = entry("urn:air:publisher-authorized", Some("1.0.0"));
    e.display_name = Some("Caf\u{e9} \u{1f600}".to_string());
    let mut ext = Members::default();
    ext.insert(
        "com.example.sourceIdentifier".to_string(),
        Json::String("old".to_string()),
    );
    e.extensions = Some(ext);
    e.unrecognized.insert("x-extra".to_string(), number("1.50"));
    let r = turul_ai_catalog_storage::EntryRecord::new(e.clone()).unwrap();
    let etag = store.put_entry(&c, &r, Condition::Absent).await.unwrap();
    let got = store.get_entry(&c, &e.key()).await.unwrap().unwrap();
    assert_eq!(got.record.bytes(), r.bytes());
    assert_eq!(got.record.entry(), &e);
    assert_eq!(
        got.record.entry().identifier.as_str(),
        "urn:air:publisher-authorized"
    );
    assert_eq!(got.etag, etag);
}

pub async fn entry_trust_manifest_payload_survives_storage(store: &dyn CatalogStore) {
    let c = id("entry-trust-manifest-payload-survives-storage");
    let text = r#"{"identifier":"urn:air:m","type":"application/mcp-server-card+json","url":"https://example.com/m.json","trustManifests":[{"contributor":"did:web:example.com","trustSchema":{"identifier":"https://example.com/schema","version":"1"},"attestations":[{"type":"https://example.com/att","uri":"https://example.com/a","mediaType":"application/json","size":100}],"x-n":1.0e2}]}"#;
    let e = parse_entry(text.as_bytes()).unwrap();
    let before: Vec<Vec<u8>> = e
        .trust_manifests
        .as_ref()
        .unwrap()
        .iter()
        .map(|m| m.to_json().to_canonical_vec().unwrap())
        .collect();
    assert!(!before.is_empty());
    let r = turul_ai_catalog_storage::EntryRecord::new(e.clone()).unwrap();
    store.put_entry(&c, &r, Condition::Any).await.unwrap();
    let got = store.get_entry(&c, &e.key()).await.unwrap().unwrap();
    let after: Vec<Vec<u8>> = got
        .record
        .entry()
        .trust_manifests
        .as_ref()
        .unwrap()
        .iter()
        .map(|m| m.to_json().to_canonical_vec().unwrap())
        .collect();
    assert_eq!(before, after);
}

pub async fn catalogs_are_isolated(store: &dyn CatalogStore) {
    let (a, b) = (id("catalogs-are-isolated.a"), id("catalogs-are-isolated.b"));
    let r = record("urn:air:a", None);
    store.put_entry(&a, &r, Condition::Absent).await.unwrap();
    store
        .put_host(&a, &host(), Condition::Absent)
        .await
        .unwrap();
    assert!(store.get_entry(&b, &r.key()).await.unwrap().is_none());
    assert!(store.list_entries(&b).await.unwrap().is_empty());
    assert!(store.get_host(&b).await.unwrap().is_none());
    store.put_entry(&b, &r, Condition::Absent).await.unwrap();
    store.delete_entry(&b, &r.key(), None).await.unwrap();
    assert!(store.get_entry(&a, &r.key()).await.unwrap().is_some());
}

pub async fn absent_condition_refuses_an_existing_record(store: &dyn CatalogStore) {
    let c = id("absent-condition-refuses-an-existing-record");
    let r = record("urn:air:a", None);
    store.put_entry(&c, &r, Condition::Absent).await.unwrap();
    assert_eq!(
        store.put_entry(&c, &r, Condition::Absent).await,
        Err(StoreError::PreconditionFailed)
    );
    store
        .put_host(&c, &host(), Condition::Absent)
        .await
        .unwrap();
    assert_eq!(
        store.put_host(&c, &host(), Condition::Absent).await,
        Err(StoreError::PreconditionFailed)
    );
}

pub async fn matches_condition_refuses_a_stale_etag(store: &dyn CatalogStore) {
    let c = id("matches-condition-refuses-a-stale-etag");
    let mut e = entry("urn:air:a", None);
    let first = store
        .put_entry(
            &c,
            &turul_ai_catalog_storage::EntryRecord::new(e.clone()).unwrap(),
            Condition::Absent,
        )
        .await
        .unwrap();
    e.description = Some("changed".to_string());
    let changed = turul_ai_catalog_storage::EntryRecord::new(e.clone()).unwrap();
    let second = store
        .put_entry(&c, &changed, Condition::Matches(first.clone()))
        .await
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(
        store
            .put_entry(&c, &changed, Condition::Matches(first))
            .await,
        Err(StoreError::PreconditionFailed)
    );
    let missing = record("urn:air:missing", None);
    assert_eq!(
        store
            .put_entry(&c, &missing, Condition::Matches(second))
            .await,
        Err(StoreError::PreconditionFailed)
    );
}

pub async fn delete_with_a_stale_etag_is_refused(store: &dyn CatalogStore) {
    let c = id("delete-with-a-stale-etag-is-refused");
    let r = record("urn:air:a", None);
    let etag = store.put_entry(&c, &r, Condition::Absent).await.unwrap();
    let stale = Etag::of(b"other");
    assert_eq!(
        store.delete_entry(&c, &r.key(), Some(&stale)).await,
        Err(StoreError::PreconditionFailed)
    );
    assert!(store.get_entry(&c, &r.key()).await.unwrap().is_some());
    store.delete_entry(&c, &r.key(), Some(&etag)).await.unwrap();
    assert!(store.get_entry(&c, &r.key()).await.unwrap().is_none());
    assert_eq!(
        store.delete_entry(&c, &r.key(), Some(&etag)).await,
        Err(StoreError::PreconditionFailed)
    );
}

pub async fn delete_of_a_missing_record_is_not_found(store: &dyn CatalogStore) {
    let c = id("delete-of-a-missing-record-is-not-found");
    assert_eq!(
        store
            .delete_entry(&c, &record("urn:air:a", None).key(), None)
            .await,
        Err(StoreError::NotFound)
    );
}

pub async fn concurrent_absent_puts_have_one_winner(store: Arc<dyn CatalogStore>) {
    let c = id("concurrent-absent-puts-have-one-winner");
    let puts = (0..8).map(|i| {
        let store = Arc::clone(&store);
        let c = c.clone();
        async move {
            let mut e = entry("urn:air:a", None);
            e.description = Some(format!("writer {i}"));
            let r = turul_ai_catalog_storage::EntryRecord::new(e).unwrap();
            store.put_entry(&c, &r, Condition::Absent).await
        }
    });
    let results = join_all(puts).await;
    assert_eq!(
        results.iter().filter(|r| r.is_ok()).count(),
        1,
        "{results:?}"
    );
    assert!(
        results
            .iter()
            .filter(|r| r.is_err())
            .all(|r| *r == Err(StoreError::PreconditionFailed))
    );
}

pub async fn writes_are_visible_to_the_next_read(store: &dyn CatalogStore) {
    let c = id("writes-are-visible-to-the-next-read");
    for i in 0..5 {
        let r = record(&format!("urn:air:e{i}"), None);
        store.put_entry(&c, &r, Condition::Absent).await.unwrap();
        assert!(store.get_entry(&c, &r.key()).await.unwrap().is_some());
        assert_eq!(store.list_entries(&c).await.unwrap().len(), i + 1);
    }
    store.put_host(&c, &host(), Condition::Any).await.unwrap();
    assert!(store.get_host(&c).await.unwrap().is_some());
    let gone = record("urn:air:e0", None);
    store.delete_entry(&c, &gone.key(), None).await.unwrap();
    assert!(store.get_entry(&c, &gone.key()).await.unwrap().is_none());
    assert_eq!(store.list_entries(&c).await.unwrap().len(), 4);
}

pub async fn list_returns_every_entry(store: &dyn CatalogStore) {
    list_returns_n_entries(store, 300).await
}

/// `count` entries each listed once; a backend test may pass a count large enough to span pages.
pub async fn list_returns_n_entries(store: &dyn CatalogStore, count: usize) {
    let c = id("list-returns-every-entry");
    for i in 0..count {
        let r = record(&format!("urn:air:n{i:05}"), None);
        store.put_entry(&c, &r, Condition::Any).await.unwrap();
    }
    let listed = store.list_entries(&c).await.unwrap();
    assert_eq!(listed.len(), count);
    let mut ids: Vec<_> = listed
        .iter()
        .map(|s| s.record.entry().identifier.as_str().to_string())
        .collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), count);
    assert!(
        listed
            .iter()
            .all(|s| matches!(s.record.entry().source, ArtifactSource::Url(_)))
    );
}

pub const EXPECTED_ASSEMBLY: &str = r#"{"entries":[{"identifier":"urn:air:a","type":"application/mcp-server-card+json","url":"https://example.com/card.json","version":"1"},{"identifier":"urn:air:a","type":"application/mcp-server-card+json","url":"https://example.com/card.json","version":"2"},{"identifier":"urn:air:b","type":"application/mcp-server-card+json","url":"https://example.com/card.json"}],"host":{"displayName":"Example Operator"},"specVersion":"1.0"}"#;

pub async fn same_state_assembles_to_the_same_bytes(store: &dyn CatalogStore) {
    let c = id("same-state-assembles-to-the-same-bytes");
    for (i, v) in [
        ("urn:air:b", None),
        ("urn:air:a", Some("2")),
        ("urn:air:a", Some("1")),
    ] {
        store
            .put_entry(&c, &record(i, v), Condition::Any)
            .await
            .unwrap();
    }
    store.put_host(&c, &host(), Condition::Any).await.unwrap();
    let out = assemble(store, &c).await.unwrap();
    assert_eq!(std::str::from_utf8(&out.bytes).unwrap(), EXPECTED_ASSEMBLY);
}

pub async fn etag_is_the_digest_of_the_record_bytes(store: &dyn CatalogStore) {
    let c = id("etag-is-the-digest-of-the-record-bytes");
    let r = record("urn:air:a", None);
    let etag = store.put_entry(&c, &r, Condition::Absent).await.unwrap();
    assert_eq!(etag, Etag::of(r.bytes()));
    let got = store.get_entry(&c, &r.key()).await.unwrap().unwrap();
    assert_eq!(got.etag, Etag::of(got.record.bytes()));
    assert_eq!(got.etag.as_str().len(), 64);
    let h = host();
    let host_etag = store.put_host(&c, &h, Condition::Absent).await.unwrap();
    assert_eq!(host_etag, Etag::of(h.bytes()));
    assert_eq!(store.get_host(&c).await.unwrap().unwrap().etag, host_etag);
}

pub async fn recreated_identical_record_has_the_same_etag(store: &dyn CatalogStore) {
    let c = id("recreated-identical-record-has-the-same-etag");
    let r = record("urn:air:a", None);
    let first = store.put_entry(&c, &r, Condition::Absent).await.unwrap();
    store
        .delete_entry(&c, &r.key(), Some(&first))
        .await
        .unwrap();
    let second = store.put_entry(&c, &r, Condition::Absent).await.unwrap();
    assert_eq!(first, second);
}

fn kind_conflict(identifier: &str) -> Result<Etag, StoreError> {
    Err(StoreError::VersionKindConflict {
        identifier: EntryIdentifier::new(identifier),
    })
}

/// The identifiers, as `(identifier, version)`, of every stored entry.
async fn stored_keys(
    store: &dyn CatalogStore,
    c: &turul_ai_catalog_storage::CatalogId,
) -> Vec<(String, Option<String>)> {
    let mut keys: Vec<_> = store
        .list_entries(c)
        .await
        .unwrap()
        .into_iter()
        .map(|s| {
            let e = s.record.into_entry();
            (e.identifier.as_str().to_string(), e.version)
        })
        .collect();
    keys.sort();
    keys
}

// #multi-version-entries: "When `version` is absent, `identifier` alone MUST be unique", so an
// unversioned entry is refused beside versioned ones of its identifier.
pub async fn unversioned_entry_beside_versioned_ones_is_refused(store: &dyn CatalogStore) {
    let c = id("unversioned-entry-beside-versioned-ones-is-refused");
    store
        .put_entry(&c, &record("urn:air:a", Some("1")), Condition::Absent)
        .await
        .unwrap();
    for condition in [Condition::Any, Condition::Absent] {
        assert_eq!(
            store
                .put_entry(&c, &record("urn:air:a", None), condition)
                .await,
            kind_conflict("urn:air:a")
        );
    }
    assert_eq!(
        stored_keys(store, &c).await,
        [("urn:air:a".to_string(), Some("1".to_string()))]
    );
    assert!(assemble(store, &c).await.is_ok());
}

// #multi-version-entries: the reverse direction; an empty `version` is a present version.
pub async fn versioned_entry_beside_an_unversioned_one_is_refused(store: &dyn CatalogStore) {
    let c = id("versioned-entry-beside-an-unversioned-one-is-refused");
    store
        .put_entry(&c, &record("urn:air:a", None), Condition::Absent)
        .await
        .unwrap();
    for version in ["1", ""] {
        for condition in [Condition::Any, Condition::Absent] {
            assert_eq!(
                store
                    .put_entry(&c, &record("urn:air:a", Some(version)), condition)
                    .await,
                kind_conflict("urn:air:a")
            );
        }
    }
    assert_eq!(
        stored_keys(store, &c).await,
        [("urn:air:a".to_string(), None)]
    );
    assert!(assemble(store, &c).await.is_ok());
}

pub async fn same_version_kind_puts_are_accepted(store: &dyn CatalogStore) {
    let c = id("same-version-kind-puts-are-accepted");
    for version in ["1", "2", ""] {
        store
            .put_entry(&c, &record("urn:air:a", Some(version)), Condition::Absent)
            .await
            .unwrap();
    }
    let b = record("urn:air:b", None);
    let first = store.put_entry(&c, &b, Condition::Absent).await.unwrap();
    store.put_entry(&c, &b, Condition::Any).await.unwrap();
    store
        .put_entry(&c, &b, Condition::Matches(first))
        .await
        .unwrap();
    store
        .put_entry(&c, &record("urn:air:c", Some("1")), Condition::Any)
        .await
        .unwrap();
    assert_eq!(store.list_entries(&c).await.unwrap().len(), 5);
    assert!(assemble(store, &c).await.is_ok());
}

pub async fn deleting_the_last_record_of_a_kind_frees_the_identifier(store: &dyn CatalogStore) {
    let c = id("deleting-the-last-record-of-a-kind-frees-the-identifier");
    let (a, a1, a2) = (
        record("urn:air:a", None),
        record("urn:air:a", Some("1")),
        record("urn:air:a", Some("2")),
    );
    store.put_entry(&c, &a1, Condition::Absent).await.unwrap();
    let a2_etag = store.put_entry(&c, &a2, Condition::Absent).await.unwrap();
    store.delete_entry(&c, &a1.key(), None).await.unwrap();
    assert_eq!(
        store.put_entry(&c, &a, Condition::Any).await,
        kind_conflict("urn:air:a")
    );
    // A refused delete leaves the identifier's kind in place.
    assert_eq!(
        store
            .delete_entry(&c, &a2.key(), Some(&Etag::of(b"other")))
            .await,
        Err(StoreError::PreconditionFailed)
    );
    assert_eq!(
        store.delete_entry(&c, &a1.key(), None).await,
        Err(StoreError::NotFound)
    );
    assert_eq!(
        store.put_entry(&c, &a, Condition::Any).await,
        kind_conflict("urn:air:a")
    );
    store
        .delete_entry(&c, &a2.key(), Some(&a2_etag))
        .await
        .unwrap();
    store.put_entry(&c, &a, Condition::Absent).await.unwrap();
    assert_eq!(
        store.put_entry(&c, &a1, Condition::Any).await,
        kind_conflict("urn:air:a")
    );
    store.delete_entry(&c, &a.key(), None).await.unwrap();
    store.put_entry(&c, &a1, Condition::Absent).await.unwrap();
    assert_eq!(
        stored_keys(store, &c).await,
        [("urn:air:a".to_string(), Some("1".to_string()))]
    );
}

// A precondition is decided before the version kind: only a new record can conflict.
pub async fn conditional_puts_keep_their_preconditions(store: &dyn CatalogStore) {
    let c = id("conditional-puts-keep-their-preconditions");
    let mut e = entry("urn:air:a", Some("1"));
    let first = store
        .put_entry(
            &c,
            &turul_ai_catalog_storage::EntryRecord::new(e.clone()).unwrap(),
            Condition::Absent,
        )
        .await
        .unwrap();
    e.description = Some("changed".to_string());
    let changed = turul_ai_catalog_storage::EntryRecord::new(e).unwrap();
    assert_eq!(
        store.put_entry(&c, &changed, Condition::Absent).await,
        Err(StoreError::PreconditionFailed)
    );
    assert_eq!(
        store
            .put_entry(&c, &changed, Condition::Matches(Etag::of(b"other")))
            .await,
        Err(StoreError::PreconditionFailed)
    );
    let second = store
        .put_entry(&c, &changed, Condition::Matches(first.clone()))
        .await
        .unwrap();
    assert_eq!(second, Etag::of(changed.bytes()));
    let unversioned = record("urn:air:a", None);
    assert_eq!(
        store
            .put_entry(&c, &unversioned, Condition::Matches(second))
            .await,
        Err(StoreError::PreconditionFailed)
    );
    assert_eq!(
        store.put_entry(&c, &unversioned, Condition::Absent).await,
        kind_conflict("urn:air:a")
    );
    let got = store.get_entry(&c, &changed.key()).await.unwrap().unwrap();
    assert_eq!(got.record.bytes(), changed.bytes());
}

/// Writers racing an unversioned `urn:air:a` against versioned ones leave entries of one kind.
pub async fn racing_version_kinds_leave_one_kind(store: Arc<dyn CatalogStore>) {
    const WRITERS: usize = 16;
    for round in 0..4 {
        let c = id(&format!("racing-version-kinds-leave-one-kind.{round}"));
        let puts = (0..WRITERS).map(|i| {
            let store = Arc::clone(&store);
            let c = c.clone();
            // Alternate kinds; which kind starts changes per round.
            let versioned = (i + round) % 2 == 1;
            async move {
                let mut e = entry("urn:air:a", versioned.then(|| i.to_string()).as_deref());
                e.description = Some(format!("writer {i}"));
                let r = turul_ai_catalog_storage::EntryRecord::new(e).unwrap();
                (versioned, store.put_entry(&c, &r, Condition::Any).await)
            }
        });
        let results = join_all(puts).await;
        let keys = stored_keys(store.as_ref(), &c).await;
        let versioned_won = keys[0].1.is_some();
        assert!(
            keys.iter().all(|(_, v)| v.is_some() == versioned_won),
            "{keys:?}"
        );
        for (versioned, result) in &results {
            if *versioned == versioned_won {
                assert!(result.is_ok(), "{result:?}");
            } else {
                assert_eq!(result, &kind_conflict("urn:air:a"));
            }
        }
        let winners = results.iter().filter(|(v, _)| *v == versioned_won).count();
        assert_eq!(keys.len(), if versioned_won { winners } else { 1 });
        assert!(assemble(store.as_ref(), &c).await.is_ok());
    }
}
