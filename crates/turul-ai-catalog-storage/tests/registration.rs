#![cfg(feature = "in-memory")]

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use futures::executor::block_on;
use turul_ai_catalog::identity::{EntryIdentifier, EntryKey};
use turul_ai_catalog::model::{ArtifactSource, CatalogEntry, Publisher};
use turul_ai_catalog_storage::{
    CatalogId, CatalogStore, Condition, EntryRecord, Etag, InMemoryCatalogStore, ModeKind,
    ModeRequest, Provenance, PublisherVerifier, Registrar, RegistrationError, RegistrationPolicy,
    StoreError, StoredEntry, VerifierRefusal, WriterGrant, WriterId,
};

use common::{catalog, entry, now, ts, writer};

const TTL: Duration = Duration::from_secs(400);
const SLACK: Duration = Duration::from_secs(100);
const ID: &str = "urn:air:example.com:mcp:svc";

/// A verifier that records each call and answers with a fixed result.
struct Stub {
    calls: Mutex<Vec<String>>,
    refusal: Option<&'static str>,
}

impl Stub {
    fn accepting() -> Self {
        Self {
            calls: Mutex::default(),
            refusal: None,
        }
    }
    fn refusing(reason: &'static str) -> Self {
        Self {
            calls: Mutex::default(),
            refusal: Some(reason),
        }
    }
    fn calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

#[async_trait]
impl PublisherVerifier for Stub {
    async fn verify_publisher(
        &self,
        entry: &CatalogEntry,
        _now: SystemTime,
    ) -> Result<(), VerifierRefusal> {
        self.calls
            .lock()
            .unwrap()
            .push(entry.identifier.as_str().to_string());
        match self.refusal {
            None => Ok(()),
            Some(r) => Err(VerifierRefusal(r.to_string())),
        }
    }
}

/// An in-memory store that counts reads and writes and can refuse every put with one error.
struct Spy {
    inner: InMemoryCatalogStore,
    gets: AtomicUsize,
    puts: AtomicUsize,
    refuse_puts: Mutex<Option<StoreError>>,
}

impl Spy {
    fn new() -> Self {
        Self {
            inner: InMemoryCatalogStore::new(),
            gets: AtomicUsize::new(0),
            puts: AtomicUsize::new(0),
            refuse_puts: Mutex::new(None),
        }
    }
    fn gets(&self) -> usize {
        self.gets.load(Ordering::SeqCst)
    }
    fn puts(&self) -> usize {
        self.puts.load(Ordering::SeqCst)
    }
    fn refuse_puts(&self, error: StoreError) {
        *self.refuse_puts.lock().unwrap() = Some(error);
    }
}

#[async_trait]
impl CatalogStore for Spy {
    fn backend_name(&self) -> &'static str {
        "spy"
    }
    async fn put_entry(
        &self,
        catalog: &CatalogId,
        record: &EntryRecord,
        provenance: &Provenance,
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        self.puts.fetch_add(1, Ordering::SeqCst);
        if let Some(e) = self.refuse_puts.lock().unwrap().clone() {
            return Err(e);
        }
        self.inner
            .put_entry(catalog, record, provenance, condition)
            .await
    }
    async fn get_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
    ) -> Result<Option<StoredEntry>, StoreError> {
        self.gets.fetch_add(1, Ordering::SeqCst);
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
        record: &turul_ai_catalog_storage::HostRecord,
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        self.inner.put_host(catalog, record, condition).await
    }
    async fn get_host(
        &self,
        catalog: &CatalogId,
    ) -> Result<
        Option<turul_ai_catalog_storage::Stored<turul_ai_catalog_storage::HostRecord>>,
        StoreError,
    > {
        self.inner.get_host(catalog).await
    }
}

fn grant(publishers: &[&str], prefixes: &[&str], signed: bool) -> WriterGrant {
    WriterGrant::new(
        publishers.iter().map(|s| s.to_string()),
        prefixes.iter().map(|s| s.to_string()),
        signed,
    )
    .unwrap()
}

/// `svc` and `other` may register under `example.com` and the prefix `internal:`; `signed` the
/// same with a publisher signature required; `curator` only `example.com`. All four may register
/// curated entries; `runtime` has `svc`'s identifiers and leases only.
fn policy() -> RegistrationPolicy {
    let open = || grant(&["example.com"], &["internal:"], false);
    RegistrationPolicy::new(
        [
            (writer("svc"), open().with_curated()),
            (writer("other"), open().with_curated()),
            (
                writer("signed"),
                grant(&["example.com"], &["internal:"], true).with_curated(),
            ),
            (
                writer("curator"),
                grant(&["example.com"], &[], false).with_curated(),
            ),
            (writer("runtime"), open()),
        ],
        TTL,
    )
    .unwrap()
}

fn registrar(store: &Arc<impl CatalogStore + 'static>, verifier: Option<&Arc<Stub>>) -> Registrar {
    // The policy has a signing writer, so a registrar needs some verifier; callers that pass
    // none never reach it.
    let verifier: Arc<dyn PublisherVerifier> = match verifier {
        Some(v) => v.clone(),
        None => Arc::new(Stub::accepting()),
    };
    Registrar::new(store.clone(), catalog("main"), policy(), Some(verifier)).unwrap()
}

fn lease_of(
    registered: &turul_ai_catalog_storage::Registered,
) -> turul_ai_catalog::timestamp::Timestamp {
    match &registered.provenance {
        Provenance::Lease { expires_at, .. } => *expires_at,
        other => panic!("not a lease: {other:?}"),
    }
}

fn at(offset: Duration) -> SystemTime {
    now() + offset
}

fn register(
    r: &Registrar,
    who: &str,
    e: CatalogEntry,
    mode: ModeRequest,
    when: SystemTime,
) -> Result<turul_ai_catalog_storage::Registered, RegistrationError> {
    block_on(r.register(&writer(who), e, mode, when))
}

#[test]
fn writer_id_grammar() {
    for ok in ["a", "operator.1", "A_b-c.9", &"x".repeat(128)] {
        assert_eq!(WriterId::new(ok).unwrap().as_str(), ok);
    }
    for bad in ["", "10.0.0.1:443", "a b", "w/x", "bücher", &"x".repeat(129)] {
        assert!(WriterId::new(bad).is_err(), "{bad:?}");
    }
}

// Nothing in the storage crate reads the clock: callers pass `now`.
#[test]
fn storage_source_never_reads_the_clock() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for entry in std::fs::read_dir(src).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        for call in ["SystemTime::now", "Instant::now"] {
            assert!(!text.contains(call), "{} reads the clock", path.display());
        }
    }
}

#[test]
fn unknown_writer_is_refused() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let r = registrar(&store, None);
    assert_eq!(
        register(&r, "nobody", entry(ID, None), ModeRequest::Lease, now()),
        Err(RegistrationError::UnknownWriter)
    );
    let key = EntryKey::new(EntryIdentifier::new(ID), None);
    assert_eq!(
        block_on(r.unregister(&writer("nobody"), &key)),
        Err(RegistrationError::UnknownWriter)
    );
}

// #catalog-projection rule 3: an operator that is neither the publisher nor its delegate may not
// write a `urn:air` identifier under another domain.
#[test]
fn urn_air_outside_the_writers_publishers_is_refused() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let r = registrar(&store, None);
    assert_eq!(
        register(
            &r,
            "svc",
            entry("urn:air:other.org:mcp:svc", None),
            ModeRequest::Lease,
            now()
        ),
        Err(RegistrationError::PublisherNotAllowed {
            publisher: "other.org".to_string()
        })
    );
}

// #the-did-web-publisher-profile: no case folding, subdomain or suffix match.
#[test]
fn publisher_match_is_exact() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let r = registrar(&store, None);
    for publisher in ["Example.com", "sub.example.com", "example.com.evil"] {
        assert_eq!(
            register(
                &r,
                "svc",
                entry(&format!("urn:air:{publisher}:mcp:svc"), None),
                ModeRequest::Lease,
                now()
            ),
            Err(RegistrationError::PublisherNotAllowed {
                publisher: publisher.to_string()
            }),
            "{publisher}"
        );
    }
}

#[test]
fn non_urn_air_outside_every_prefix_is_refused() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let r = registrar(&store, None);
    for id in ["external:svc", "Internal:svc", "svc"] {
        assert_eq!(
            register(&r, "svc", entry(id, None), ModeRequest::Lease, now()),
            Err(RegistrationError::IdentifierNotAllowed),
            "{id}"
        );
    }
    assert_eq!(
        register(
            &r,
            "curator",
            entry("internal:svc", None),
            ModeRequest::Lease,
            now()
        ),
        Err(RegistrationError::IdentifierNotAllowed)
    );
}

#[test]
fn non_urn_air_under_a_prefix_is_accepted() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let r = registrar(&store, None);
    let got = register(
        &r,
        "svc",
        entry("internal:svc", None),
        ModeRequest::Lease,
        now(),
    )
    .unwrap();
    assert!(got.written);
    let stored = block_on(store.get_entry(
        &catalog("main"),
        &EntryKey::new(EntryIdentifier::new("internal:svc"), None),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(stored.record.entry().identifier.as_str(), "internal:svc");
}

#[test]
fn malformed_urn_air_is_refused() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let policy = RegistrationPolicy::new(
        [(writer("svc"), grant(&["example.com"], &["URN"], false))],
        TTL,
    )
    .unwrap();
    let r = Registrar::new(store.clone(), catalog("main"), policy, None).unwrap();
    for id in ["urn:air:example.com:only-two", "URN:AIR:example.com:a:b"] {
        assert_eq!(
            register(&r, "svc", entry(id, None), ModeRequest::Lease, now()),
            Err(RegistrationError::MalformedUrnAir),
            "{id}"
        );
    }
}

// #catalog-projection: an operator "MUST NOT infer publisher authorization from the artifact's
// URL or an unsigned `publisher` field".
#[test]
fn url_and_publisher_field_never_grant_authority() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let r = registrar(&store, None);
    let mut e = entry("urn:air:evil.example:mcp:svc", None);
    e.source = ArtifactSource::Url("https://example.com/card.json".to_string());
    e.publisher = Some(Publisher::new(
        "did:web:example.com".to_string(),
        "Example".to_string(),
    ));
    assert_eq!(
        register(&r, "svc", e, ModeRequest::Lease, now()),
        Err(RegistrationError::PublisherNotAllowed {
            publisher: "evil.example".to_string()
        })
    );
}

// #catalog-projection: a new retrieval URL changes nothing about identity.
#[test]
fn reregistration_with_a_new_url_is_an_update() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let r = registrar(&store, None);
    let first = register(&r, "svc", entry(ID, None), ModeRequest::Lease, now()).unwrap();
    let mut moved = entry(ID, None);
    moved.source = ArtifactSource::Url("https://elsewhere.example/card.json".to_string());
    let second = register(&r, "svc", moved, ModeRequest::Lease, now()).unwrap();
    assert!(second.written);
    assert_ne!(first.etag, second.etag);
    let listed = block_on(store.list_entries(&catalog("main"))).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        listed[0].record.entry().source,
        ArtifactSource::Url("https://elsewhere.example/card.json".to_string())
    );
}

// #the-did-web-publisher-profile: "The entry MUST use the standard `urn:air` syntax."
#[test]
fn signature_required_refuses_non_urn_air() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let stub = Arc::new(Stub::accepting());
    let r = registrar(&store, Some(&stub));
    assert_eq!(
        register(
            &r,
            "signed",
            entry("internal:svc", None),
            ModeRequest::Lease,
            now()
        ),
        Err(RegistrationError::SignatureRequiresUrnAir)
    );
    assert_eq!(stub.calls(), 0);
}

#[test]
fn required_signature_is_checked_by_the_verifier() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let ok = Arc::new(Stub::accepting());
    let got = register(
        &registrar(&store, Some(&ok)),
        "signed",
        entry(ID, None),
        ModeRequest::Lease,
        now(),
    )
    .unwrap();
    assert!(got.written);
    assert_eq!(ok.calls(), 1);

    let store = Arc::new(InMemoryCatalogStore::new());
    let no = Arc::new(Stub::refusing("no manifest"));
    assert_eq!(
        register(
            &registrar(&store, Some(&no)),
            "signed",
            entry(ID, None),
            ModeRequest::Lease,
            now()
        ),
        Err(RegistrationError::PublisherSignature(VerifierRefusal(
            "no manifest".to_string()
        )))
    );
    assert!(
        block_on(store.list_entries(&catalog("main")))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn signature_not_required_never_calls_the_verifier() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let stub = Arc::new(Stub::refusing("never asked"));
    let r = registrar(&store, Some(&stub));
    register(&r, "svc", entry(ID, None), ModeRequest::Lease, now()).unwrap();
    register(
        &r,
        "curator",
        entry("urn:air:example.com:mcp:c", None),
        ModeRequest::Curated,
        now(),
    )
    .unwrap();
    assert_eq!(stub.calls(), 0);
}

// Signature acceptance ends at `expiresAt`, so a refresh that can no longer verify must not
// extend the lease.
#[test]
fn refresh_with_an_expired_signature_does_not_extend_the_lease() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let accept = Arc::new(Stub::accepting());
    let first = register(
        &registrar(&store, Some(&accept)),
        "signed",
        entry(ID, None),
        ModeRequest::Lease,
        now(),
    )
    .unwrap();
    let expired = Arc::new(Stub::refusing("signature expired"));
    let due = at(TTL - SLACK + Duration::from_secs(1));
    assert!(matches!(
        register(
            &registrar(&store, Some(&expired)),
            "signed",
            entry(ID, None),
            ModeRequest::Lease,
            due
        ),
        Err(RegistrationError::PublisherSignature(_))
    ));
    let stored = block_on(store.get_entry(
        &catalog("main"),
        &EntryKey::new(EntryIdentifier::new(ID), None),
    ))
    .unwrap()
    .unwrap();
    assert_eq!(stored.provenance, first.provenance);
    assert_eq!(stored.etag, first.etag);
}

#[test]
fn curated_registration_over_a_lease_is_refused() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let r = registrar(&store, None);
    register(&r, "svc", entry(ID, None), ModeRequest::Lease, now()).unwrap();
    assert_eq!(
        register(&r, "svc", entry(ID, None), ModeRequest::Curated, now()),
        Err(RegistrationError::ModeConflict {
            stored: ModeKind::Lease
        })
    );
    let other = "urn:air:example.com:mcp:other";
    register(&r, "svc", entry(other, None), ModeRequest::Curated, now()).unwrap();
    assert_eq!(
        register(&r, "svc", entry(other, None), ModeRequest::Lease, now()),
        Err(RegistrationError::ModeConflict {
            stored: ModeKind::Curated
        })
    );
}

// A curated entry never expires, so the operator grants it; a writer cannot choose it on its own.
#[test]
fn curated_registration_needs_a_curated_grant() {
    let store = Arc::new(Spy::new());
    let r = registrar(&store, None);
    assert_eq!(
        register(&r, "runtime", entry(ID, None), ModeRequest::Curated, now()),
        Err(RegistrationError::CuratedNotGranted)
    );
    assert_eq!((store.gets(), store.puts()), (0, 0));
    register(&r, "runtime", entry(ID, None), ModeRequest::Lease, now()).unwrap();
    let other = "urn:air:example.com:mcp:other";
    register(&r, "svc", entry(other, None), ModeRequest::Curated, now()).unwrap();
}

// Writers granted one publisher share its identifiers: one may add a version of an identifier
// another registered, but never overwrite or remove the other's record.
#[test]
fn co_granted_writers_share_an_identifier_but_not_its_records() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let r = registrar(&store, None);
    register(
        &r,
        "svc",
        entry(ID, Some("1.0.0")),
        ModeRequest::Lease,
        now(),
    )
    .unwrap();
    register(
        &r,
        "other",
        entry(ID, Some("1.0.1")),
        ModeRequest::Lease,
        now(),
    )
    .unwrap();
    assert_eq!(
        register(
            &r,
            "other",
            entry(ID, Some("1.0.0")),
            ModeRequest::Lease,
            now()
        ),
        Err(RegistrationError::OwnedByAnotherWriter)
    );
    let key = EntryKey::new(EntryIdentifier::new(ID), Some("1.0.1".to_string()));
    assert_eq!(
        block_on(r.unregister(&writer("svc"), &key)),
        Err(RegistrationError::OwnedByAnotherWriter)
    );
    let mut writers: Vec<String> = block_on(store.list_entries(&catalog("main")))
        .unwrap()
        .iter()
        .map(|s| s.provenance.writer().as_str().to_string())
        .collect();
    writers.sort();
    assert_eq!(writers, ["other", "svc"]);
}

#[test]
fn registration_of_another_writers_entry_is_refused() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let r = registrar(&store, None);
    register(&r, "svc", entry(ID, None), ModeRequest::Lease, now()).unwrap();
    assert_eq!(
        register(&r, "other", entry(ID, None), ModeRequest::Lease, now()),
        Err(RegistrationError::OwnedByAnotherWriter)
    );
    // An expired lease is still its owner's.
    assert_eq!(
        register(
            &r,
            "other",
            entry(ID, None),
            ModeRequest::Lease,
            at(TTL * 10)
        ),
        Err(RegistrationError::OwnedByAnotherWriter)
    );
}

// A refresh writes only once the stored expiry is less than three quarters of the lease away.
#[test]
fn refresh_is_a_no_op_until_due() {
    let store = Arc::new(Spy::new());
    let r = registrar(&store, None);
    let first = register(&r, "svc", entry(ID, None), ModeRequest::Lease, now()).unwrap();
    assert_eq!(lease_of(&first), ts("2026-11-17T12:06:40Z"));
    assert_eq!(store.puts(), 1);

    // Stored expiry equals `when + TTL - SLACK`: not due.
    let not_due = register(&r, "svc", entry(ID, None), ModeRequest::Lease, at(SLACK)).unwrap();
    assert!(!not_due.written);
    assert_eq!(not_due.etag, first.etag);
    assert_eq!(not_due.provenance, first.provenance);
    assert_eq!(store.puts(), 1);

    let when = at(SLACK + Duration::from_nanos(1));
    let due = register(&r, "svc", entry(ID, None), ModeRequest::Lease, when).unwrap();
    assert!(due.written);
    assert_eq!(store.puts(), 2);
    assert_eq!(
        lease_of(&due),
        turul_ai_catalog::timestamp::Timestamp::from_system_time(when + TTL)
    );
    assert_ne!(due.etag, first.etag);
}

#[test]
fn refresh_never_shortens_a_lease() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let r = registrar(&store, None);
    let first = register(&r, "svc", entry(ID, None), ModeRequest::Lease, now()).unwrap();

    // A refresh by an instance whose clock is behind is not due, so it writes nothing.
    let behind = now() - Duration::from_secs(1000);
    let refreshed = register(&r, "svc", entry(ID, None), ModeRequest::Lease, behind).unwrap();
    assert!(!refreshed.written);
    assert_eq!(refreshed.provenance, first.provenance);

    // An update by that instance keeps the later stored expiry.
    let mut changed = entry(ID, None);
    changed.description = Some("new".to_string());
    let updated = register(&r, "svc", changed, ModeRequest::Lease, behind).unwrap();
    assert!(updated.written);
    assert_eq!(lease_of(&updated), lease_of(&first));

    // A due refresh moves the expiry later by more than the slack.
    let later = at(TTL);
    let mut changed = entry(ID, None);
    changed.description = Some("new".to_string());
    let due = register(&r, "svc", changed, ModeRequest::Lease, later).unwrap();
    assert!(due.written);
    assert!(lease_of(&due) > lease_of(&first));
}

#[test]
fn unregister_removes_the_writers_entry_in_either_mode() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let r = registrar(&store, None);
    let other = "urn:air:example.com:mcp:other";
    register(&r, "svc", entry(ID, None), ModeRequest::Lease, now()).unwrap();
    register(&r, "svc", entry(other, None), ModeRequest::Curated, now()).unwrap();
    for id in [ID, other] {
        let key = EntryKey::new(EntryIdentifier::new(id), None);
        assert_eq!(block_on(r.unregister(&writer("svc"), &key)), Ok(()));
    }
    assert!(
        block_on(store.list_entries(&catalog("main")))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn unregister_needs_current_authority() {
    let store = Arc::new(InMemoryCatalogStore::new());
    register(
        &registrar(&store, None),
        "svc",
        entry(ID, None),
        ModeRequest::Lease,
        now(),
    )
    .unwrap();
    let narrowed = RegistrationPolicy::new(
        [(writer("svc"), grant(&["elsewhere.example"], &[], false))],
        TTL,
    )
    .unwrap();
    let r = Registrar::new(store.clone(), catalog("main"), narrowed, None).unwrap();
    let key = EntryKey::new(EntryIdentifier::new(ID), None);
    assert_eq!(
        block_on(r.unregister(&writer("svc"), &key)),
        Err(RegistrationError::PublisherNotAllowed {
            publisher: "example.com".to_string()
        })
    );
    assert_eq!(
        block_on(store.list_entries(&catalog("main")))
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn unregister_of_a_missing_entry_is_not_found() {
    let store = Arc::new(InMemoryCatalogStore::new());
    let r = registrar(&store, None);
    let key = EntryKey::new(EntryIdentifier::new(ID), None);
    assert_eq!(
        block_on(r.unregister(&writer("svc"), &key)),
        Err(RegistrationError::NotFound)
    );
}

// Each request below fails only at its own step, and no step after a refusal runs: the store is
// untouched before step 5 and the verifier is called only at step 6.
#[test]
fn checks_run_in_the_documented_order() {
    let invalid = || {
        let mut e = entry("", None);
        e.display_name = Some(String::new());
        e
    };
    // Steps 1 to 4 do no I/O.
    let local: Vec<(&str, CatalogEntry, ModeRequest, RegistrationError)> = vec![
        (
            "nobody",
            invalid(),
            ModeRequest::Curated,
            RegistrationError::UnknownWriter,
        ),
        (
            "runtime",
            invalid(),
            ModeRequest::Curated,
            RegistrationError::CuratedNotGranted,
        ),
        (
            "svc",
            entry("", None),
            ModeRequest::Lease,
            RegistrationError::Record(EntryRecord::new(entry("", None)).unwrap_err()),
        ),
        (
            "signed",
            entry("urn:air:other.org:mcp:x", None),
            ModeRequest::Lease,
            RegistrationError::PublisherNotAllowed {
                publisher: "other.org".to_string(),
            },
        ),
        (
            "signed",
            entry("internal:x", None),
            ModeRequest::Lease,
            RegistrationError::SignatureRequiresUrnAir,
        ),
    ];
    for (who, e, mode, expected) in local {
        let store = Arc::new(Spy::new());
        let stub = Arc::new(Stub::accepting());
        let r = registrar(&store, Some(&stub));
        assert_eq!(register(&r, who, e, mode, now()), Err(expected));
        assert_eq!((store.gets(), store.puts(), stub.calls()), (0, 0, 0));
    }

    // Step 5 reads the store and stops before the verifier: mode first, then writer.
    let store = Arc::new(Spy::new());
    let stub = Arc::new(Stub::accepting());
    let r = registrar(&store, Some(&stub));
    register(&r, "other", entry(ID, None), ModeRequest::Curated, now()).unwrap();
    let puts = store.puts();
    assert_eq!(
        register(&r, "signed", entry(ID, None), ModeRequest::Lease, now()),
        Err(RegistrationError::ModeConflict {
            stored: ModeKind::Curated
        })
    );
    assert_eq!(
        register(&r, "signed", entry(ID, None), ModeRequest::Curated, now()),
        Err(RegistrationError::OwnedByAnotherWriter)
    );
    assert_eq!((store.puts(), stub.calls()), (puts, 0));

    // Step 6 calls the verifier and writes nothing; step 7 writes.
    let store = Arc::new(Spy::new());
    let no = Arc::new(Stub::refusing("no"));
    assert_eq!(
        register(
            &registrar(&store, Some(&no)),
            "signed",
            entry(ID, None),
            ModeRequest::Lease,
            now()
        ),
        Err(RegistrationError::PublisherSignature(VerifierRefusal(
            "no".to_string()
        )))
    );
    assert_eq!((store.puts(), no.calls()), (0, 1));
    let yes = Arc::new(Stub::accepting());
    register(
        &registrar(&store, Some(&yes)),
        "signed",
        entry(ID, None),
        ModeRequest::Lease,
        now(),
    )
    .unwrap();
    assert_eq!((store.puts(), yes.calls()), (1, 1));
}

// Other writers change the entry on every attempt: bounded retries, then `Contended`, with one
// verification however many attempts.
#[test]
fn contention_beyond_the_attempt_bound_is_reported() {
    let store = Arc::new(Spy::new());
    let stub = Arc::new(Stub::accepting());
    register(
        &registrar(&store, Some(&stub)),
        "signed",
        entry(ID, None),
        ModeRequest::Lease,
        now(),
    )
    .unwrap();
    store.refuse_puts(StoreError::PreconditionFailed);
    let (gets, puts, calls) = (store.gets(), store.puts(), stub.calls());
    let mut changed = entry(ID, None);
    changed.description = Some("new".to_string());
    assert_eq!(
        register(
            &registrar(&store, Some(&stub)),
            "signed",
            changed,
            ModeRequest::Lease,
            now()
        ),
        Err(RegistrationError::Contended)
    );
    assert_eq!(store.gets() - gets, 8);
    assert_eq!(store.puts() - puts, 8);
    assert_eq!(stub.calls() - calls, 1);
}

// A store that exhausted its own retries reports `Contended`; the registrar passes it on as
// contention, after one attempt, not as a store failure.
#[test]
fn store_contention_is_reported_as_contended() {
    let store = Arc::new(Spy::new());
    store.refuse_puts(StoreError::Contended);
    assert_eq!(
        register(
            &registrar(&store, None),
            "svc",
            entry(ID, None),
            ModeRequest::Lease,
            now()
        ),
        Err(RegistrationError::Contended)
    );
    assert_eq!(store.puts(), 1);
}

// A lease ending past 9999-12-31T23:59:59Z has no RFC 3339 form, so it is refused before the
// store is read; a curated entry has no expiry and is written.
#[test]
fn lease_ending_past_year_9999_is_refused_before_any_io() {
    let store = Arc::new(Spy::new());
    let r = registrar(&store, None);
    let late = SystemTime::UNIX_EPOCH + Duration::from_secs(253_402_300_800) - TTL / 2;
    assert_eq!(
        register(&r, "svc", entry(ID, None), ModeRequest::Lease, late),
        Err(RegistrationError::ExpiryOutOfRange)
    );
    assert_eq!((store.gets(), store.puts()), (0, 0));
    assert!(register(&r, "svc", entry(ID, None), ModeRequest::Curated, late).is_ok());
}
