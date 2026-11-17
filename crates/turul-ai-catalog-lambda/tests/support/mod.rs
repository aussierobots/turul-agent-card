//! Shared request and response helpers for the lambda tests.

#![allow(dead_code)]

use futures::executor::block_on;
use lambda_http::http::Request as Builder;
use lambda_http::{Body, Request, Response};
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_lambda::ServedCatalog;
use turul_ai_catalog_storage::{CatalogId, InMemoryCatalogStore, import};

pub fn request(method: &str, uri: &str) -> Request {
    Builder::builder()
        .method(method)
        .uri(uri)
        .body(Body::Empty)
        .expect("request builds")
}

pub fn body_bytes(response: &Response<Body>) -> Vec<u8> {
    match response.body() {
        Body::Empty => Vec::new(),
        Body::Text(s) => s.as_bytes().to_vec(),
        Body::Binary(b) => b.clone(),
        #[allow(unreachable_patterns)]
        _ => panic!("unexpected body variant"),
    }
}

pub fn catalog_id() -> CatalogId {
    CatalogId::new("test").expect("valid catalog id")
}

/// An in-memory store holding the records of `document`.
pub fn store_with(document: &str) -> InMemoryCatalogStore {
    let store = InMemoryCatalogStore::new();
    let parsed = parse(document.as_bytes()).expect("fixture parses");
    block_on(import(&store, &catalog_id(), &parsed)).expect("fixture imports");
    store
}

/// The catalog served after importing `document`.
pub fn served_from(document: &str) -> ServedCatalog {
    match block_on(ServedCatalog::assemble(
        &store_with(document),
        &catalog_id(),
    )) {
        Ok(s) => s,
        Err(e) => panic!("fixture must assemble: {e}"),
    }
}
