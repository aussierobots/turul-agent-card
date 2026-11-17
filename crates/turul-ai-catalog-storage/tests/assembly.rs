mod common;

use common::*;
use futures::executor::block_on;
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_storage::{CatalogStore, Condition, InMemoryCatalogStore, assemble};

#[test]
fn empty_store_assembles_to_a_valid_empty_catalog() {
    let store = InMemoryCatalogStore::new();
    let out = block_on(assemble(&store, &catalog("c"))).unwrap();
    assert_eq!(out.bytes, br#"{"entries":[],"specVersion":"1.0"}"#);
    assert!(!out.report.has_errors());
}

#[test]
fn assembled_document_has_no_signature() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    block_on(store.put_host(&c, &host(), Condition::Any)).unwrap();
    block_on(store.put_entry(&c, &record("urn:air:a", None), Condition::Any)).unwrap();
    let out = block_on(assemble(&store, &c)).unwrap();
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
        block_on(store.put_entry(&c, &record(id, v), Condition::Any)).unwrap();
    }
    let doc = parse(&block_on(assemble(&store, &c)).unwrap().bytes).unwrap();
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

// #multi-version-entries: with `version` absent, "`identifier` alone MUST be unique". Each record
// passes the gate alone and the store accepts both; only the assembled whole fails.
#[test]
fn a_cross_entry_error_fails_assembly() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    block_on(store.put_entry(&c, &record("a", None), Condition::Any)).unwrap();
    block_on(store.put_entry(&c, &record("a", Some("1")), Condition::Any)).unwrap();
    assert!(matches!(
        block_on(assemble(&store, &c)),
        Err(turul_ai_catalog_storage::AssembleError::Invalid(_))
    ));
}
