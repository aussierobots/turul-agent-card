//! Shared request and response helpers for the lambda tests.

#![allow(dead_code)]

use lambda_http::http::Request as Builder;
use lambda_http::{Body, Request, Response};

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

/// Bytes with the `'static` lifetime `ServedCatalog::load` requires.
pub fn leak(text: String) -> &'static [u8] {
    Box::leak(text.into_bytes().into_boxed_slice())
}
