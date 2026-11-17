#![cfg(feature = "in-memory")]

mod common;

use common::*;
use futures::executor::block_on;
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::{Members, Signature};
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_storage::{
    CatalogStore, Condition, EntryRecord, ImportError, InMemoryCatalogStore, ModeKind, StoreError,
    assemble, import,
};

const DOC: &str = r#"{"specVersion":"1.0","host":{"displayName":"Op"},"entries":[{"identifier":"urn:air:a","type":"application/mcp-server-card+json","url":"https://example.com/a.json"}]}"#;

#[test]
fn import_loads_host_and_entries_and_assembles_the_same_document() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    let doc = parse(DOC.as_bytes()).unwrap();
    let etags = block_on(import(&store, &c, &doc, &writer("operator"))).unwrap();
    assert_eq!(etags.len(), 2);
    let out = block_on(assemble(&store, &c, now())).unwrap();
    assert_eq!(parse(&out.bytes).unwrap(), doc);
    assert_eq!(
        block_on(import(&store, &c, &doc, &writer("operator"))).unwrap(),
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
        block_on(import(&store, &catalog("c"), &doc, &writer("operator"))),
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
        block_on(import(&store, &catalog("c"), &doc, &writer("operator"))),
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
        block_on(import(&store, &catalog("c"), &doc, &writer("operator"))),
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
    match block_on(import(&store, &c, &doc, &writer("operator"))) {
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
    let etags = block_on(import(&store, &c, &doc, &writer("operator"))).unwrap();
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
        block_on(import(&store, &c, &doc, &writer("operator"))),
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
    block_on(import(&store, &c, &doc, &writer("first"))).unwrap();
    assert_eq!(
        block_on(import(&store, &c, &doc, &writer("second"))),
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
    block_on(import(&store, &c, &doc, &writer("operator"))).unwrap();
    let mut changed = doc.clone();
    changed.entries[0].description = Some("changed".to_string());
    block_on(import(&store, &c, &changed, &writer("operator"))).unwrap();
    assert_eq!(
        parse(&block_on(assemble(&store, &c, now())).unwrap().bytes).unwrap(),
        changed
    );
}
