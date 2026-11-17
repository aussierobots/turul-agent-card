//! The backend contract. Every backend's test file calls every check against its own store,
//! each check in a catalog of its own so backends sharing a table do not interfere.

#![allow(dead_code)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use futures::future::join_all;
use turul_ai_catalog::identity::EntryIdentifier;
use turul_ai_catalog::identity::EntryKey;
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::{ArtifactSource, Members};
use turul_ai_catalog::parse::parse_entry;
use turul_ai_catalog::timestamp::Timestamp;
use turul_ai_catalog_storage::{
    CatalogId, CatalogStore, Condition, EntryRecord, Etag, HostRecord, ModeKind, ModeRequest,
    Provenance, ProvenanceError, Registrar, RegistrationError, RegistrationPolicy, StoreError,
    Stored, StoredEntry, WriterGrant, assemble,
};

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
    store
        .put_entry(&c, &r, &curated(), Condition::Any)
        .await
        .unwrap();
    store
        .put_entry(&c, &r, &curated(), Condition::Any)
        .await
        .unwrap();
    assert_eq!(store.list_entries(&c).await.unwrap().len(), 1);
    store
        .put_entry(
            &c,
            &record("urn:air:a", Some("2")),
            &curated(),
            Condition::Any,
        )
        .await
        .unwrap();
    assert_eq!(store.list_entries(&c).await.unwrap().len(), 2);
    // An unversioned entry is keyed on its identifier alone.
    let b = record("urn:air:b", None);
    store
        .put_entry(&c, &b, &curated(), Condition::Any)
        .await
        .unwrap();
    store
        .put_entry(&c, &b, &curated(), Condition::Any)
        .await
        .unwrap();
    assert_eq!(store.list_entries(&c).await.unwrap().len(), 3);
    assert!(assemble(store, &c, now()).await.is_ok());
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
    let etag = store
        .put_entry(&c, &r, &curated(), Condition::Absent)
        .await
        .unwrap();
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
    store
        .put_entry(&c, &r, &curated(), Condition::Any)
        .await
        .unwrap();
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
    store
        .put_entry(&a, &r, &curated(), Condition::Absent)
        .await
        .unwrap();
    store
        .put_host(&a, &host(), Condition::Absent)
        .await
        .unwrap();
    assert!(store.get_entry(&b, &r.key()).await.unwrap().is_none());
    assert!(store.list_entries(&b).await.unwrap().is_empty());
    assert!(store.get_host(&b).await.unwrap().is_none());
    store
        .put_entry(&b, &r, &curated(), Condition::Absent)
        .await
        .unwrap();
    store.delete_entry(&b, &r.key(), None).await.unwrap();
    assert!(store.get_entry(&a, &r.key()).await.unwrap().is_some());
}

pub async fn absent_condition_refuses_an_existing_record(store: &dyn CatalogStore) {
    let c = id("absent-condition-refuses-an-existing-record");
    let r = record("urn:air:a", None);
    store
        .put_entry(&c, &r, &curated(), Condition::Absent)
        .await
        .unwrap();
    assert_eq!(
        store.put_entry(&c, &r, &curated(), Condition::Absent).await,
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
            &curated(),
            Condition::Absent,
        )
        .await
        .unwrap();
    e.description = Some("changed".to_string());
    let changed = turul_ai_catalog_storage::EntryRecord::new(e.clone()).unwrap();
    let second = store
        .put_entry(&c, &changed, &curated(), Condition::Matches(first.clone()))
        .await
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(
        store
            .put_entry(&c, &changed, &curated(), Condition::Matches(first))
            .await,
        Err(StoreError::PreconditionFailed)
    );
    let missing = record("urn:air:missing", None);
    assert_eq!(
        store
            .put_entry(&c, &missing, &curated(), Condition::Matches(second))
            .await,
        Err(StoreError::PreconditionFailed)
    );
}

pub async fn delete_with_a_stale_etag_is_refused(store: &dyn CatalogStore) {
    let c = id("delete-with-a-stale-etag-is-refused");
    let r = record("urn:air:a", None);
    let etag = store
        .put_entry(&c, &r, &curated(), Condition::Absent)
        .await
        .unwrap();
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
            store.put_entry(&c, &r, &curated(), Condition::Absent).await
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
        store
            .put_entry(&c, &r, &curated(), Condition::Absent)
            .await
            .unwrap();
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
        store
            .put_entry(&c, &r, &curated(), Condition::Any)
            .await
            .unwrap();
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

pub async fn same_state_and_now_assemble_to_the_same_bytes(store: &dyn CatalogStore) {
    let c = id("same-state-assembles-to-the-same-bytes");
    for (i, v) in [
        ("urn:air:b", None),
        ("urn:air:a", Some("2")),
        ("urn:air:a", Some("1")),
    ] {
        store
            .put_entry(&c, &record(i, v), &curated(), Condition::Any)
            .await
            .unwrap();
    }
    store.put_host(&c, &host(), Condition::Any).await.unwrap();
    let out = assemble(store, &c, now()).await.unwrap();
    assert_eq!(std::str::from_utf8(&out.bytes).unwrap(), EXPECTED_ASSEMBLY);
    assert!(out.omitted.is_empty() && out.next_expiry.is_none());

    // A lease that is live at `now` is served and sets `next_expiry`; at its expiry it is
    // omitted, whichever backend holds it.
    let expires = "2026-11-17T12:00:10Z";
    store
        .put_entry(
            &c,
            &record("urn:air:c", None),
            &lease("w", expires),
            Condition::Absent,
        )
        .await
        .unwrap();
    let live = assemble(store, &c, now()).await.unwrap();
    assert!(
        String::from_utf8(live.bytes.clone())
            .unwrap()
            .contains("urn:air:c")
    );
    assert_eq!(live.next_expiry, Some(ts(expires)));
    assert_eq!(assemble(store, &c, now()).await.unwrap(), live);
    let gone = assemble(store, &c, now_shifted(10_000_000_000, false))
        .await
        .unwrap();
    assert_eq!(std::str::from_utf8(&gone.bytes).unwrap(), EXPECTED_ASSEMBLY);
    assert_eq!(gone.omitted.len(), 1);
    assert_eq!(gone.omitted[0].provenance, lease("w", expires));
}

// #multi-version-entries: an expired lease still holds its identifier's version kind, so the
// other kind is refused until the record is deleted.
pub async fn expired_lease_keeps_its_version_kind(store: &dyn CatalogStore) {
    let c = id("expired-lease-keeps-its-version-kind");
    let expired = lease("w", "2026-11-17T11:00:00Z");
    store
        .put_entry(&c, &record("urn:air:a", None), &expired, Condition::Absent)
        .await
        .unwrap();
    let late = now();
    assert_eq!(assemble(store, &c, late).await.unwrap().omitted.len(), 1);
    for condition in [Condition::Any, Condition::Absent] {
        assert_eq!(
            store
                .put_entry(&c, &record("urn:air:a", Some("1")), &curated(), condition)
                .await,
            kind_conflict("urn:air:a")
        );
    }
    let stored = store
        .get_entry(&c, &record("urn:air:a", None).key())
        .await
        .unwrap()
        .unwrap();
    store
        .delete_entry(&c, &record("urn:air:a", None).key(), Some(&stored.etag))
        .await
        .unwrap();
    store
        .put_entry(
            &c,
            &record("urn:air:a", Some("1")),
            &curated(),
            Condition::Absent,
        )
        .await
        .unwrap();
}

pub async fn etag_is_the_digest_of_entry_and_provenance(store: &dyn CatalogStore) {
    let c = id("etag-is-the-digest-of-entry-and-provenance");
    let r = record("urn:air:a", None);
    let p = lease("svc", "2026-11-17T12:00:00Z");
    let etag = store
        .put_entry(&c, &r, &p, Condition::Absent)
        .await
        .unwrap();
    assert_eq!(etag, Etag::of_entry(&r, &p).unwrap());
    assert_ne!(etag, Etag::of(r.bytes()));
    let got = store.get_entry(&c, &r.key()).await.unwrap().unwrap();
    assert_eq!(
        got.etag,
        Etag::of_entry(&got.record, &got.provenance).unwrap()
    );
    assert_eq!(got.etag.as_str().len(), 64);
    assert_eq!(store.list_entries(&c).await.unwrap()[0].etag, etag);
    // The same record under other provenance is another version.
    let other = store
        .put_entry(
            &c,
            &r,
            &lease("svc", "2026-11-17T12:00:01Z"),
            Condition::Any,
        )
        .await
        .unwrap();
    assert_ne!(other, etag);
    // The host's version is still the digest of its record bytes.
    let h = host();
    let host_etag = store.put_host(&c, &h, Condition::Absent).await.unwrap();
    assert_eq!(host_etag, Etag::of(h.bytes()));
    assert_eq!(store.get_host(&c).await.unwrap().unwrap().etag, host_etag);
}

/// A provenance whose bytes would not read back (an expiry past year 9999) is refused before
/// anything is written, so no backend holds a record its own reads would refuse.
pub async fn provenance_that_would_not_read_back_is_refused(store: &dyn CatalogStore) {
    let c = id("provenance-that-would-not-read-back-is-refused");
    let r = record("urn:air:a", None);
    let past_9999 = Provenance::Lease {
        writer: writer("svc"),
        expires_at: Timestamp::from_system_time(
            std::time::UNIX_EPOCH + Duration::from_secs(253_402_300_800),
        ),
    };
    let refused = |result: Result<Etag, StoreError>| {
        matches!(
            result,
            Err(StoreError::CorruptProvenance {
                error: ProvenanceError::ExpiresAt,
                ..
            })
        )
    };
    for condition in [Condition::Absent, Condition::Any] {
        assert!(refused(
            store.put_entry(&c, &r, &past_9999, condition).await
        ));
    }
    assert!(store.get_entry(&c, &r.key()).await.unwrap().is_none());
    let p = lease("svc", "2026-11-17T12:00:00Z");
    let etag = store
        .put_entry(&c, &r, &p, Condition::Absent)
        .await
        .unwrap();
    for condition in [Condition::Any, Condition::Matches(etag.clone())] {
        assert!(refused(
            store.put_entry(&c, &r, &past_9999, condition).await
        ));
    }
    let got = store.get_entry(&c, &r.key()).await.unwrap().unwrap();
    assert_eq!((got.etag, got.provenance), (etag, p));
}

/// Run by each backend that stores a mode name, after it wrote an entry item for `record` under
/// `c` whose mode is `"other"` and whose stored version is `etag`: a write over it is corrupt
/// provenance, never a backend error. The in-memory store holds a typed mode and cannot hold one.
pub async fn unknown_stored_mode_is_corrupt_provenance(
    store: &dyn CatalogStore,
    c: &CatalogId,
    record: &EntryRecord,
    etag: &Etag,
) {
    for p in [curated(), lease("operator", "2026-11-17T12:00:00Z")] {
        for condition in [Condition::Any, Condition::Matches(etag.clone())] {
            let result = store.put_entry(c, record, &p, condition).await;
            assert!(
                matches!(
                    result,
                    Err(StoreError::CorruptProvenance {
                        error: ProvenanceError::Mode,
                        ..
                    })
                ),
                "{result:?}"
            );
        }
    }
}

pub async fn provenance_reads_back_exactly(store: &dyn CatalogStore) {
    let c = id("provenance-reads-back-exactly");
    let cases = [
        (record("urn:air:curated", None), curated_by("op.1")),
        (
            record("urn:air:leased", None),
            lease("svc-1", "2026-11-17T12:00:00.25Z"),
        ),
    ];
    for (r, p) in &cases {
        let etag = store.put_entry(&c, r, p, Condition::Absent).await.unwrap();
        let got = store.get_entry(&c, &r.key()).await.unwrap().unwrap();
        assert_eq!((&got.provenance, &got.etag), (p, &etag));
        assert_eq!(got.provenance.to_bytes().unwrap(), p.to_bytes().unwrap());
    }
    let listed = store.list_entries(&c).await.unwrap();
    assert_eq!(listed.len(), 2);
    for (r, p) in &cases {
        let got = listed.iter().find(|s| s.record.key() == r.key()).unwrap();
        assert_eq!(&got.provenance, p);
    }
    // A replace in the same mode stores the new provenance whole.
    let (r, _) = &cases[1];
    let later = lease("svc-2", "2027-01-01T00:00:00Z");
    store
        .put_entry(&c, r, &later, Condition::Any)
        .await
        .unwrap();
    let got = store.get_entry(&c, &r.key()).await.unwrap().unwrap();
    assert_eq!(got.provenance, later);
}

// Guarantee 8: a put never replaces a record stored in the other mode, whatever its condition.
pub async fn lease_and_curated_puts_refuse_each_other(store: &dyn CatalogStore) {
    let c = id("lease-and-curated-puts-refuse-each-other");
    let expiry = "2030-01-01T00:00:00Z";
    for (name, first, second, stored) in [
        (
            "urn:air:a",
            curated(),
            lease("svc", expiry),
            ModeKind::Curated,
        ),
        (
            "urn:air:b",
            lease("svc", expiry),
            curated(),
            ModeKind::Lease,
        ),
    ] {
        let r = record(name, None);
        let etag = store
            .put_entry(&c, &r, &first, Condition::Absent)
            .await
            .unwrap();
        let conflict = Err(StoreError::ModeConflict { stored });
        assert_eq!(
            store.put_entry(&c, &r, &second, Condition::Any).await,
            conflict
        );
        assert_eq!(
            store
                .put_entry(&c, &r, &second, Condition::Matches(etag.clone()))
                .await,
            conflict
        );
        // A precondition is decided before the mode.
        assert_eq!(
            store.put_entry(&c, &r, &second, Condition::Absent).await,
            Err(StoreError::PreconditionFailed)
        );
        assert_eq!(
            store
                .put_entry(&c, &r, &second, Condition::Matches(Etag::of(b"other")))
                .await,
            Err(StoreError::PreconditionFailed)
        );
        let got = store.get_entry(&c, &r.key()).await.unwrap().unwrap();
        assert_eq!((got.provenance, got.etag), (first, etag));
        // Deleting frees the key for the other mode.
        store.delete_entry(&c, &r.key(), None).await.unwrap();
        store
            .put_entry(&c, &r, &second, Condition::Absent)
            .await
            .unwrap();
    }
    assert!(assemble(store, &c, now()).await.is_ok());
}

// The version covers the expiry: a refresh that changes only the expiry changes it, so a reader
// of the old lease cannot overwrite the refreshed one.
pub async fn matches_condition_refuses_a_stale_lease_etag(store: &dyn CatalogStore) {
    let c = id("matches-condition-refuses-a-stale-lease-etag");
    let r = record("urn:air:a", None);
    let first = store
        .put_entry(
            &c,
            &r,
            &lease("svc", "2026-11-17T12:00:00Z"),
            Condition::Absent,
        )
        .await
        .unwrap();
    let refreshed = lease("svc", "2026-11-17T13:00:00Z");
    let second = store
        .put_entry(&c, &r, &refreshed, Condition::Matches(first.clone()))
        .await
        .unwrap();
    assert_ne!(first, second);
    assert_eq!(
        store
            .put_entry(
                &c,
                &r,
                &lease("svc", "2026-11-17T12:30:00Z"),
                Condition::Matches(first)
            )
            .await,
        Err(StoreError::PreconditionFailed)
    );
    let got = store.get_entry(&c, &r.key()).await.unwrap().unwrap();
    assert_eq!((got.provenance, got.etag), (refreshed, second));
}

/// Writers racing one key in both modes with `Condition::Any` leave records of one mode, and
/// every put of the other mode is refused.
pub async fn racing_cross_mode_puts_leave_one_mode(store: Arc<dyn CatalogStore>) {
    const WRITERS: usize = 16;
    for round in 0..4 {
        let c = id(&format!("racing-cross-mode-puts-leave-one-mode.{round}"));
        let puts = (0..WRITERS).map(|i| {
            let store = Arc::clone(&store);
            let c = c.clone();
            let leased = (i + round) % 2 == 1;
            async move {
                let mut e = entry("urn:air:a", None);
                e.description = Some(format!("writer {i}"));
                let r = EntryRecord::new(e).unwrap();
                let provenance = if leased {
                    lease(&format!("w{i}"), "2030-01-01T00:00:00Z")
                } else {
                    curated_by(&format!("w{i}"))
                };
                (
                    leased,
                    store.put_entry(&c, &r, &provenance, Condition::Any).await,
                )
            }
        });
        let results = join_all(puts).await;
        let listed = store.list_entries(&c).await.unwrap();
        assert_eq!(listed.len(), 1);
        let won = listed[0].provenance.kind();
        assert_eq!(
            listed[0].etag,
            Etag::of_entry(&listed[0].record, &listed[0].provenance).unwrap()
        );
        for (leased, result) in &results {
            let kind = if *leased {
                ModeKind::Lease
            } else {
                ModeKind::Curated
            };
            if kind == won {
                assert!(result.is_ok(), "{result:?}");
            } else {
                assert_eq!(result, &Err(StoreError::ModeConflict { stored: won }));
            }
        }
        assert!(assemble(store.as_ref(), &c, now()).await.is_ok());
    }
}

/// Instances that read one lease and refresh it under `Matches` of the version read: one commits
/// per version, the rest are refused, and the stored expiry is the winner's.
pub async fn concurrent_lease_refreshes_on_one_etag_have_one_winner(store: Arc<dyn CatalogStore>) {
    const INSTANCES: usize = 16;
    let c = id("concurrent-lease-refreshes-on-one-etag-have-one-winner");
    let r = record("urn:air:a", None);
    let mut etag = store
        .put_entry(
            &c,
            &r,
            &lease("svc", "2026-11-17T12:00:00Z"),
            Condition::Absent,
        )
        .await
        .unwrap();
    for round in 1..=4 {
        let refreshes = (0..INSTANCES).map(|i| {
            let (store, c, r, etag) = (Arc::clone(&store), c.clone(), r.clone(), etag.clone());
            async move {
                let expires = format!("2026-11-17T1{round}:00:{i:02}Z");
                let result = store
                    .put_entry(&c, &r, &lease("svc", &expires), Condition::Matches(etag))
                    .await;
                (expires, result)
            }
        });
        let results = join_all(refreshes).await;
        let winners: Vec<_> = results.iter().filter(|(_, r)| r.is_ok()).collect();
        assert_eq!(winners.len(), 1, "{results:?}");
        assert!(
            results
                .iter()
                .filter(|(_, r)| r.is_err())
                .all(|(_, r)| *r == Err(StoreError::PreconditionFailed))
        );
        let got = store.get_entry(&c, &r.key()).await.unwrap().unwrap();
        assert_eq!(got.provenance, lease("svc", &winners[0].0));
        assert_eq!(Ok(got.etag.clone()), winners[0].1);
        etag = got.etag;
    }
}

pub async fn recreated_identical_record_has_the_same_etag(store: &dyn CatalogStore) {
    let c = id("recreated-identical-record-has-the-same-etag");
    let r = record("urn:air:a", None);
    let first = store
        .put_entry(&c, &r, &curated(), Condition::Absent)
        .await
        .unwrap();
    store
        .delete_entry(&c, &r.key(), Some(&first))
        .await
        .unwrap();
    let second = store
        .put_entry(&c, &r, &curated(), Condition::Absent)
        .await
        .unwrap();
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
        .put_entry(
            &c,
            &record("urn:air:a", Some("1")),
            &curated(),
            Condition::Absent,
        )
        .await
        .unwrap();
    for condition in [Condition::Any, Condition::Absent] {
        assert_eq!(
            store
                .put_entry(&c, &record("urn:air:a", None), &curated(), condition)
                .await,
            kind_conflict("urn:air:a")
        );
    }
    assert_eq!(
        stored_keys(store, &c).await,
        [("urn:air:a".to_string(), Some("1".to_string()))]
    );
    assert!(assemble(store, &c, now()).await.is_ok());
}

// #multi-version-entries: the reverse direction; an empty `version` is a present version.
pub async fn versioned_entry_beside_an_unversioned_one_is_refused(store: &dyn CatalogStore) {
    let c = id("versioned-entry-beside-an-unversioned-one-is-refused");
    store
        .put_entry(
            &c,
            &record("urn:air:a", None),
            &curated(),
            Condition::Absent,
        )
        .await
        .unwrap();
    for version in ["1", ""] {
        for condition in [Condition::Any, Condition::Absent] {
            assert_eq!(
                store
                    .put_entry(
                        &c,
                        &record("urn:air:a", Some(version)),
                        &curated(),
                        condition
                    )
                    .await,
                kind_conflict("urn:air:a")
            );
        }
    }
    assert_eq!(
        stored_keys(store, &c).await,
        [("urn:air:a".to_string(), None)]
    );
    assert!(assemble(store, &c, now()).await.is_ok());
}

pub async fn same_version_kind_puts_are_accepted(store: &dyn CatalogStore) {
    let c = id("same-version-kind-puts-are-accepted");
    for version in ["1", "2", ""] {
        store
            .put_entry(
                &c,
                &record("urn:air:a", Some(version)),
                &curated(),
                Condition::Absent,
            )
            .await
            .unwrap();
    }
    let b = record("urn:air:b", None);
    let first = store
        .put_entry(&c, &b, &curated(), Condition::Absent)
        .await
        .unwrap();
    store
        .put_entry(&c, &b, &curated(), Condition::Any)
        .await
        .unwrap();
    store
        .put_entry(&c, &b, &curated(), Condition::Matches(first))
        .await
        .unwrap();
    store
        .put_entry(
            &c,
            &record("urn:air:c", Some("1")),
            &curated(),
            Condition::Any,
        )
        .await
        .unwrap();
    assert_eq!(store.list_entries(&c).await.unwrap().len(), 5);
    assert!(assemble(store, &c, now()).await.is_ok());
}

pub async fn deleting_the_last_record_of_a_kind_frees_the_identifier(store: &dyn CatalogStore) {
    let c = id("deleting-the-last-record-of-a-kind-frees-the-identifier");
    let (a, a1, a2) = (
        record("urn:air:a", None),
        record("urn:air:a", Some("1")),
        record("urn:air:a", Some("2")),
    );
    store
        .put_entry(&c, &a1, &curated(), Condition::Absent)
        .await
        .unwrap();
    let a2_etag = store
        .put_entry(&c, &a2, &curated(), Condition::Absent)
        .await
        .unwrap();
    store.delete_entry(&c, &a1.key(), None).await.unwrap();
    assert_eq!(
        store.put_entry(&c, &a, &curated(), Condition::Any).await,
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
        store.put_entry(&c, &a, &curated(), Condition::Any).await,
        kind_conflict("urn:air:a")
    );
    store
        .delete_entry(&c, &a2.key(), Some(&a2_etag))
        .await
        .unwrap();
    store
        .put_entry(&c, &a, &curated(), Condition::Absent)
        .await
        .unwrap();
    assert_eq!(
        store.put_entry(&c, &a1, &curated(), Condition::Any).await,
        kind_conflict("urn:air:a")
    );
    store.delete_entry(&c, &a.key(), None).await.unwrap();
    store
        .put_entry(&c, &a1, &curated(), Condition::Absent)
        .await
        .unwrap();
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
            &curated(),
            Condition::Absent,
        )
        .await
        .unwrap();
    e.description = Some("changed".to_string());
    let changed = turul_ai_catalog_storage::EntryRecord::new(e).unwrap();
    assert_eq!(
        store
            .put_entry(&c, &changed, &curated(), Condition::Absent)
            .await,
        Err(StoreError::PreconditionFailed)
    );
    assert_eq!(
        store
            .put_entry(
                &c,
                &changed,
                &curated(),
                Condition::Matches(Etag::of(b"other"))
            )
            .await,
        Err(StoreError::PreconditionFailed)
    );
    let second = store
        .put_entry(&c, &changed, &curated(), Condition::Matches(first.clone()))
        .await
        .unwrap();
    assert_eq!(second, Etag::of_entry(&changed, &curated()).unwrap());
    let unversioned = record("urn:air:a", None);
    assert_eq!(
        store
            .put_entry(&c, &unversioned, &curated(), Condition::Matches(second))
            .await,
        Err(StoreError::PreconditionFailed)
    );
    assert_eq!(
        store
            .put_entry(&c, &unversioned, &curated(), Condition::Absent)
            .await,
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
                (
                    versioned,
                    store.put_entry(&c, &r, &curated(), Condition::Any).await,
                )
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
        assert!(assemble(store.as_ref(), &c, now()).await.is_ok());
    }
}

const TTL: Duration = Duration::from_secs(400);
const SLACK: Duration = Duration::from_secs(100);
const SVC: &str = "urn:air:example.com:mcp:svc";

/// `svc` and `other` may register leases and curated entries under `example.com`.
fn policy() -> RegistrationPolicy {
    let grant = || {
        WriterGrant::new(["example.com".to_string()], [], false)
            .unwrap()
            .with_curated()
    };
    RegistrationPolicy::new([(writer("svc"), grant()), (writer("other"), grant())], TTL).unwrap()
}

fn registrar(store: Arc<dyn CatalogStore>, c: &CatalogId) -> Registrar {
    Registrar::new(store, c.clone(), policy(), None).unwrap()
}

fn key(identifier: &str, version: Option<&str>) -> EntryKey {
    record(identifier, version).key()
}

// #catalog-projection rule 1: the identifier is preserved exactly.
pub async fn registered_identifier_reads_back_exactly(shared: Arc<dyn CatalogStore>) {
    let store = shared.as_ref();
    let c = id("registered-identifier-reads-back-exactly");
    let r = registrar(Arc::clone(&shared), &c);
    let identifier = "urn:air:example.com:mcp:Svc.v1_Mixed-Case";
    r.register(
        &writer("svc"),
        entry(identifier, None),
        ModeRequest::Lease,
        now(),
    )
    .await
    .unwrap();
    let got = store
        .get_entry(&c, &key(identifier, None))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got.record.entry().identifier.as_str(), identifier);
    let listed = store.list_entries(&c).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].record.entry().identifier.as_str(), identifier);
    let served = assemble(store, &c, now()).await.unwrap();
    assert!(
        String::from_utf8(served.bytes)
            .unwrap()
            .contains(identifier)
    );
}

// #catalog-projection rule 5: an identifier "MUST NOT be reassigned"; an expired lease is still
// its writer's.
pub async fn another_writer_cannot_take_over_an_entry(shared: Arc<dyn CatalogStore>) {
    let store = shared.as_ref();
    let c = id("another-writer-cannot-take-over-an-entry");
    let r = registrar(Arc::clone(&shared), &c);
    let curated = "urn:air:example.com:mcp:curated";
    let leased = r
        .register(&writer("svc"), entry(SVC, None), ModeRequest::Lease, now())
        .await
        .unwrap();
    let kept = r
        .register(
            &writer("svc"),
            entry(curated, None),
            ModeRequest::Curated,
            now(),
        )
        .await
        .unwrap();
    for (identifier, mode, when) in [
        (SVC, ModeRequest::Lease, now()),
        (SVC, ModeRequest::Lease, now() + TTL * 10),
        (curated, ModeRequest::Curated, now()),
    ] {
        assert_eq!(
            r.register(&writer("other"), entry(identifier, None), mode, when)
                .await,
            Err(RegistrationError::OwnedByAnotherWriter),
            "{identifier}"
        );
    }
    let got = store.get_entry(&c, &key(SVC, None)).await.unwrap().unwrap();
    assert_eq!((got.etag, got.provenance), (leased.etag, leased.provenance));
    let got = store
        .get_entry(&c, &key(curated, None))
        .await
        .unwrap()
        .unwrap();
    assert_eq!((got.etag, got.provenance), (kept.etag, kept.provenance));
}

pub async fn unregister_by_another_writer_is_refused(shared: Arc<dyn CatalogStore>) {
    let store = shared.as_ref();
    let c = id("unregister-by-another-writer-is-refused");
    let r = registrar(Arc::clone(&shared), &c);
    r.register(&writer("svc"), entry(SVC, None), ModeRequest::Lease, now())
        .await
        .unwrap();
    assert_eq!(
        r.unregister(&writer("other"), &key(SVC, None)).await,
        Err(RegistrationError::OwnedByAnotherWriter)
    );
    assert!(
        store
            .get_entry(&c, &key(SVC, None))
            .await
            .unwrap()
            .is_some()
    );
}

pub async fn unregister_frees_the_identifier(shared: Arc<dyn CatalogStore>) {
    let store = shared.as_ref();
    let c = id("unregister-frees-the-identifier");
    let r = registrar(Arc::clone(&shared), &c);
    r.register(&writer("svc"), entry(SVC, None), ModeRequest::Lease, now())
        .await
        .unwrap();
    assert_eq!(
        r.register(
            &writer("svc"),
            entry(SVC, Some("1")),
            ModeRequest::Lease,
            now()
        )
        .await,
        Err(RegistrationError::Store(StoreError::VersionKindConflict {
            identifier: EntryIdentifier::new(SVC)
        }))
    );
    r.unregister(&writer("svc"), &key(SVC, None)).await.unwrap();
    assert!(
        store
            .get_entry(&c, &key(SVC, None))
            .await
            .unwrap()
            .is_none()
    );
    // The version kind and the key are free: another writer may now take both.
    r.register(
        &writer("other"),
        entry(SVC, Some("1")),
        ModeRequest::Lease,
        now(),
    )
    .await
    .unwrap();
    assert_eq!(
        r.unregister(&writer("svc"), &key(SVC, None)).await,
        Err(RegistrationError::NotFound)
    );
}

/// A store that counts the puts that reach the backend.
struct CountingStore {
    inner: Arc<dyn CatalogStore>,
    puts: AtomicUsize,
}

#[async_trait]
impl CatalogStore for CountingStore {
    fn backend_name(&self) -> &'static str {
        self.inner.backend_name()
    }
    async fn put_entry(
        &self,
        catalog: &CatalogId,
        record: &EntryRecord,
        provenance: &Provenance,
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        let result = self
            .inner
            .put_entry(catalog, record, provenance, condition)
            .await;
        if result.is_ok() {
            self.puts.fetch_add(1, Ordering::SeqCst);
        }
        result
    }
    async fn get_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
    ) -> Result<Option<StoredEntry>, StoreError> {
        self.inner.get_entry(catalog, key).await
    }
    async fn delete_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
        expected: Option<&Etag>,
    ) -> Result<(), StoreError> {
        self.inner.delete_entry(catalog, key, expected).await
    }
    async fn list_entries(&self, catalog: &CatalogId) -> Result<Vec<StoredEntry>, StoreError> {
        self.inner.list_entries(catalog).await
    }
    async fn put_host(
        &self,
        catalog: &CatalogId,
        record: &HostRecord,
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        self.inner.put_host(catalog, record, condition).await
    }
    async fn get_host(
        &self,
        catalog: &CatalogId,
    ) -> Result<Option<Stored<HostRecord>>, StoreError> {
        self.inner.get_host(catalog).await
    }
}

/// Many instances refreshing one due lease, each with a clock of its own within one `slack`
/// window, all succeed: one write commits and the others find the lease no longer due. Over
/// consecutive windows, one write per window.
pub async fn many_instances_refreshing_one_lease_all_succeed(store: Arc<dyn CatalogStore>) {
    const INSTANCES: u64 = 64;
    let c = id("many-instances-refreshing-one-lease-all-succeed");
    let counting = Arc::new(CountingStore {
        inner: Arc::clone(&store),
        puts: AtomicUsize::new(0),
    });
    let r = record(SVC, None);
    // Due at every `now` below: it expires before the first window begins.
    counting
        .put_entry(
            &c,
            &r,
            &lease("svc", "2026-11-17T12:00:01Z"),
            Condition::Absent,
        )
        .await
        .unwrap();
    let seeded = counting.puts.load(Ordering::SeqCst);
    for window in 0..4u32 {
        let start = now() + SLACK * 2 * window;
        let before = counting.puts.load(Ordering::SeqCst);
        let refreshes = (0..INSTANCES).map(|i| {
            let (counting, c) = (Arc::clone(&counting), c.clone());
            // Distinct clocks, every one less than `slack` after the first.
            let when = start + SLACK * (i as u32) / (INSTANCES as u32 + 1);
            async move {
                let r = registrar(counting, &c);
                let got = r
                    .register(&writer("svc"), entry(SVC, None), ModeRequest::Lease, when)
                    .await;
                (when, got)
            }
        });
        let results = join_all(refreshes).await;
        assert!(results.iter().all(|(_, got)| got.is_ok()), "{results:?}");
        let winners: Vec<_> = results
            .iter()
            .filter(|(_, got)| got.as_ref().is_ok_and(|g| g.written))
            .collect();
        assert_eq!(winners.len(), 1, "window {window}: {results:?}");
        assert_eq!(
            counting.puts.load(Ordering::SeqCst) - before,
            1,
            "window {window}"
        );
        let got = store.get_entry(&c, &r.key()).await.unwrap().unwrap();
        assert_eq!(got.record.bytes(), r.bytes());
        assert_eq!(
            got.provenance,
            Provenance::Lease {
                writer: writer("svc"),
                expires_at: turul_ai_catalog::timestamp::Timestamp::from_system_time(
                    winners[0].0 + TTL
                ),
            }
        );
    }
    assert_eq!(counting.puts.load(Ordering::SeqCst), seeded + 4);
}
