//! A carried document is served through the same import and assembly gate as the store path.

use std::time::SystemTime;

use futures::executor::block_on;
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_lambda::{ColdStartError, ServedCatalog, handle};
use turul_ai_catalog_storage::{AssembleError, CatalogId, InMemoryCatalogStore, WriterId, import};

mod support;
use support::{body_bytes, request};
use turul_ai_catalog::identity::WELL_KNOWN_PATH;

const DOC: &str = r#"{"specVersion":"1.0","host":{"displayName":"Example"},"entries":[{"identifier":"urn:air:example.com:mcp:b","type":"application/mcp-server-card+json","url":"https://example.com/b.json"},{"identifier":"urn:air:example.com:a2a:a","type":"application/a2a-agent-card+json","url":"https://example.com/a.json"}]}"#;

#[test]
fn serves_the_same_bytes_as_importing_then_assembling() {
    let document = parse(DOC.as_bytes()).expect("parses");
    let served = block_on(ServedCatalog::from_document(&document)).expect("serves");
    let store = InMemoryCatalogStore::new();
    let id = CatalogId::new("other").expect("id");
    block_on(import(
        &store,
        &id,
        &document,
        &WriterId::new("operator").unwrap(),
    ))
    .expect("imports");
    let direct =
        block_on(ServedCatalog::assemble(&store, &id, SystemTime::now())).expect("assembles");
    let a = body_bytes(&handle(&served, &request("GET", WELL_KNOWN_PATH)));
    let b = body_bytes(&handle(&direct, &request("GET", WELL_KNOWN_PATH)));
    assert_eq!(a, b);
}

// #catalog-snapshot-coverage: a signed catalog cannot be re-assembled, so it is refused.
#[test]
fn a_signed_document_is_refused() {
    let signed = DOC.replacen(
        r#""entries""#,
        r#""signature":{"signer":"did:web:example.com","profile":"did-web-v1","issuedAt":"2026-01-01T00:00:00Z","jws":"e30..c2ln"},"entries""#,
        1,
    );
    let document = parse(signed.as_bytes()).expect("parses");
    let err = block_on(ServedCatalog::from_document(&document)).expect_err("refused");
    assert!(matches!(err, ColdStartError::Import(_)), "{err:?}");
}

// #multi-version-entries: an invalid document is refused before anything is stored.
#[test]
fn an_invalid_document_fails_assembly() {
    let invalid = r#"{"specVersion":"1.0","entries":[{"identifier":"urn:x:a","type":"t/x","url":"https://e.com/1"},{"identifier":"urn:x:a","type":"t/x","url":"https://e.com/2"}]}"#;
    let document = parse(invalid.as_bytes()).expect("parses");
    let err = block_on(ServedCatalog::from_document(&document)).expect_err("refused");
    assert!(
        matches!(
            err,
            ColdStartError::Assemble(AssembleError::Invalid(_)) | ColdStartError::Import(_)
        ),
        "{err:?}"
    );
}
