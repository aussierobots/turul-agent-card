mod common;

use async_trait::async_trait;
use common::*;
use futures::executor::block_on;
use turul_ai_catalog::identity::EntryKey;
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_storage::{
    AssembleError, CatalogId, CatalogStore, Condition, EntryRecord, Etag, HostRecord,
    InMemoryCatalogStore, StoreError, Stored, assemble,
};

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

/// A store whose records were written before `put_entry` refused other-kind entries: it lists
/// whatever it was given.
struct Listed(Vec<EntryRecord>);

#[async_trait]
impl CatalogStore for Listed {
    fn backend_name(&self) -> &'static str {
        "listed"
    }
    async fn put_entry(
        &self,
        _: &CatalogId,
        _: &EntryRecord,
        _: Condition,
    ) -> Result<Etag, StoreError> {
        unreachable!()
    }
    async fn get_entry(
        &self,
        _: &CatalogId,
        _: &EntryKey,
    ) -> Result<Option<Stored<EntryRecord>>, StoreError> {
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
    async fn list_entries(&self, _: &CatalogId) -> Result<Vec<Stored<EntryRecord>>, StoreError> {
        Ok(self
            .0
            .iter()
            .map(|r| Stored {
                record: r.clone(),
                etag: Etag::of(r.bytes()),
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
fn records_breaking_identifier_uniqueness_fail_assembly() {
    let store = Listed(vec![record("a", None), record("a", Some("1"))]);
    let Err(AssembleError::Invalid(report)) = block_on(assemble(&store, &catalog("c"))) else {
        panic!("assembly must refuse an unversioned entry beside a versioned one");
    };
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|d| d.code.as_str() == "version-missing-in-group")
    );
}
