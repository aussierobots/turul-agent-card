//! Routing and response contract of the well-known handler.

mod support;

use std::time::SystemTime;

use futures::executor::block_on;
use support::{body_bytes, catalog_id, request, served_from, store_with};
use turul_ai_catalog::canonical::to_canonical_vec;
use turul_ai_catalog::identity::{MEDIA_TYPE, WELL_KNOWN_PATH};
use turul_ai_catalog::parse::parse;
use turul_ai_catalog::validate::{Conformance, ConformanceLevel, validate};
use turul_ai_catalog_lambda::{ALLOW, handle};
use turul_ai_catalog_storage::assemble;

const SPEC_EXAMPLE: &str = include_str!("fixtures/ai-catalog.json");

// Indentation, unsorted members and `1.0`: a writer would change every one of them.
const UNCANONICAL: &str = r#"{
    "entries": [
        {
            "url":   "https://example.com/a.json",
            "type": "application/a2a-agent-card+json",
            "identifier": "urn:example:a",
            "data2": 1.0
        }
    ],
    "specVersion": "1.0"
}"#;

const NO_HOST: &str = r#"{"specVersion":"1.0","entries":[{"identifier":"urn:example:a","type":"application/a2a-agent-card+json","url":"https://example.com/a.json"}]}"#;

fn content_type(r: &lambda_http::Response<lambda_http::Body>) -> Vec<String> {
    r.headers()
        .get_all("content-type")
        .iter()
        .map(|v| v.to_str().expect("ascii header").to_owned())
        .collect()
}

/// What `assemble` yields for the records of `document`.
fn assembled_bytes(document: &str) -> Vec<u8> {
    block_on(assemble(
        &store_with(document),
        &catalog_id(),
        SystemTime::now(),
    ))
    .expect("assembles")
    .bytes
}

// #well-known-uri
#[test]
fn get_serves_the_assembled_catalog_at_the_well_known_path() {
    let r = handle(&served_from(SPEC_EXAMPLE), &request("GET", WELL_KNOWN_PATH));
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(body_bytes(&r), assembled_bytes(SPEC_EXAMPLE));
}

// #location-independence, #media-type-registration-application-ai-catalog-json
#[test]
fn content_type_is_the_ai_catalog_media_type_without_parameters() {
    let s = served_from(SPEC_EXAMPLE);
    for method in ["GET", "HEAD"] {
        let r = handle(&s, &request(method, WELL_KNOWN_PATH));
        assert_eq!(content_type(&r), vec![MEDIA_TYPE.to_owned()], "{method}");
    }
}

// #level-2-discoverable-catalog
#[test]
fn discoverable_catalog_is_served_at_the_well_known_path() {
    let report = validate(&parse(SPEC_EXAMPLE.as_bytes()).expect("parses"));
    assert_eq!(
        report.conformance(),
        Conformance::Met(ConformanceLevel::Discoverable)
    );
    let s = served_from(SPEC_EXAMPLE);
    let r = handle(&s, &request("GET", WELL_KNOWN_PATH));
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(body_bytes(&r), assembled_bytes(SPEC_EXAMPLE));
}

#[test]
fn get_body_is_the_canonical_assembled_bytes() {
    let s = served_from(UNCANONICAL);
    let r = handle(&s, &request("GET", WELL_KNOWN_PATH));
    assert_eq!(r.status().as_u16(), 200);
    let body = body_bytes(&r);
    assert_eq!(body, assembled_bytes(UNCANONICAL));
    assert_ne!(body, UNCANONICAL.as_bytes());
    assert_eq!(
        body,
        to_canonical_vec(&parse(&body).expect("parses")).expect("canonical")
    );
}

// RFC 9110 section 9.3.2: HEAD carries the GET header fields and no content.
#[test]
fn head_returns_the_get_headers_and_no_body() {
    let s = served_from(SPEC_EXAMPLE);
    let get = handle(&s, &request("GET", WELL_KNOWN_PATH));
    let head = handle(&s, &request("HEAD", WELL_KNOWN_PATH));
    assert_eq!(head.status(), get.status());
    assert_eq!(head.headers(), get.headers());
    assert!(body_bytes(&head).is_empty());
}

#[test]
fn other_paths_are_not_found() {
    let s = served_from(SPEC_EXAMPLE);
    for (method, path) in [
        ("GET", "/"),
        ("GET", "/other"),
        ("HEAD", "/other"),
        ("POST", "/other"),
        ("DELETE", "/other"),
    ] {
        let r = handle(&s, &request(method, path));
        assert_eq!(r.status().as_u16(), 404, "{method} {path}");
        assert!(r.headers().is_empty(), "{method} {path}");
        assert!(body_bytes(&r).is_empty(), "{method} {path}");
    }
}

// RFC 9110 section 15.5.6: a 405 carries Allow.
#[test]
fn other_methods_are_not_allowed_with_an_allow_header() {
    assert_eq!(ALLOW, "GET, HEAD");
    let s = served_from(SPEC_EXAMPLE);
    for method in ["POST", "PUT", "DELETE", "OPTIONS"] {
        let r = handle(&s, &request(method, WELL_KNOWN_PATH));
        assert_eq!(r.status().as_u16(), 405, "{method}");
        let allow: Vec<_> = r.headers().get_all("allow").iter().collect();
        assert_eq!(allow.len(), 1, "{method}");
        assert_eq!(allow[0], ALLOW, "{method}");
        assert!(body_bytes(&r).is_empty(), "{method}");
    }
}

// #well-known-uri: a catalog without `host` is a valid Level 1 catalog.
#[test]
fn minimal_catalog_without_host_is_served() {
    let report = validate(&parse(NO_HOST.as_bytes()).expect("parses"));
    assert_eq!(
        report.conformance(),
        Conformance::Met(ConformanceLevel::Minimal)
    );
    let s = served_from(NO_HOST);
    let r = handle(&s, &request("GET", WELL_KNOWN_PATH));
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(body_bytes(&r), assembled_bytes(NO_HOST));
}

// #dynamic-discovery: content that varies by query is out of scope, so the query is ignored.
#[test]
fn query_string_does_not_change_the_response() {
    let s = served_from(SPEC_EXAMPLE);
    let plain = handle(&s, &request("GET", WELL_KNOWN_PATH));
    let queried = handle(&s, &request("GET", &format!("{WELL_KNOWN_PATH}?a=1&b=2")));
    assert_eq!(queried.status(), plain.status());
    assert_eq!(queried.headers(), plain.headers());
    assert_eq!(body_bytes(&queried), body_bytes(&plain));
}

// #well-known-uri: the path is the literal registered path, compared exactly.
#[test]
fn near_miss_paths_are_not_found() {
    let s = served_from(SPEC_EXAMPLE);
    for path in [
        "/.well-known/ai-catalog.json/",
        "/.well-known/AI-catalog.json",
        "/.well-known/ai-catalog",
        "/ai-catalog.json",
        "/.well-known/ai%2Dcatalog.json",
    ] {
        let r = handle(&s, &request("GET", path));
        assert_eq!(r.status().as_u16(), 404, "{path}");
    }
}

#[test]
fn accept_header_does_not_change_the_response() {
    let s = served_from(SPEC_EXAMPLE);
    let plain = handle(&s, &request("GET", WELL_KNOWN_PATH));
    let mut req = request("GET", WELL_KNOWN_PATH);
    req.headers_mut()
        .insert("accept", "text/html".parse().expect("header value"));
    let r = handle(&s, &req);
    assert_eq!(r.status(), plain.status());
    assert_eq!(r.headers(), plain.headers());
    assert_eq!(body_bytes(&r), body_bytes(&plain));
}
