mod common;

use common::*;
use futures::executor::block_on;
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::{Members, Signature};
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_storage::{ImportError, InMemoryCatalogStore, assemble, import};

const DOC: &str = r#"{"specVersion":"1.0","host":{"displayName":"Op"},"entries":[{"identifier":"urn:air:a","type":"application/mcp-server-card+json","url":"https://example.com/a.json"}]}"#;

#[test]
fn import_loads_host_and_entries_and_assembles_the_same_document() {
    let store = InMemoryCatalogStore::new();
    let c = catalog("c");
    let doc = parse(DOC.as_bytes()).unwrap();
    let etags = block_on(import(&store, &c, &doc)).unwrap();
    assert_eq!(etags.len(), 2);
    let out = block_on(assemble(&store, &c)).unwrap();
    assert_eq!(parse(&out.bytes).unwrap(), doc);
    assert_eq!(block_on(import(&store, &c, &doc)).unwrap(), etags);
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
        block_on(import(&store, &catalog("c"), &doc)),
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
        block_on(import(&store, &catalog("c"), &doc)),
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
        block_on(import(&store, &catalog("c"), &doc)),
        Err(ImportError::Unstorable {
            pointer: "/other".to_string()
        })
    );
}
