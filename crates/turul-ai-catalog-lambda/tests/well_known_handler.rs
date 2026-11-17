//! Routing and response contract of the well-known handler.

mod support;

use support::{body_bytes, request};
use turul_ai_catalog::identity::{MEDIA_TYPE, WELL_KNOWN_PATH};
use turul_ai_catalog::parse::parse;
use turul_ai_catalog::validate::{Conformance, ConformanceLevel, validate};
use turul_ai_catalog_lambda::{ALLOW, BUNDLE, ServedCatalog, handle};

const SPEC_EXAMPLE: &[u8] = include_bytes!("fixtures/ai-catalog.json");

// Indentation, unsorted members and `1.0`: a writer would change every one of them.
const UNCANONICAL: &[u8] = br#"{
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

const NO_HOST: &[u8] = br#"{"specVersion":"1.0","entries":[{"identifier":"urn:example:a","type":"application/a2a-agent-card+json","url":"https://example.com/a.json"}]}"#;

fn served(bytes: &'static [u8]) -> ServedCatalog {
    match ServedCatalog::load(bytes) {
        Ok(s) => s,
        Err(e) => panic!("bundle must load: {e}"),
    }
}

fn content_type(r: &lambda_http::Response<lambda_http::Body>) -> Vec<String> {
    r.headers()
        .get_all("content-type")
        .iter()
        .map(|v| v.to_str().expect("ascii header").to_owned())
        .collect()
}

// #well-known-uri
#[test]
fn get_serves_the_bundle_at_the_well_known_path() {
    let s = served(BUNDLE);
    let r = handle(&s, &request("GET", WELL_KNOWN_PATH));
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(body_bytes(&r), BUNDLE);
}

// #location-independence, #media-type-registration-application-ai-catalog-json
#[test]
fn content_type_is_the_ai_catalog_media_type_without_parameters() {
    let s = served(BUNDLE);
    for method in ["GET", "HEAD"] {
        let r = handle(&s, &request(method, WELL_KNOWN_PATH));
        assert_eq!(content_type(&r), vec![MEDIA_TYPE.to_owned()], "{method}");
    }
}

// #level-2-discoverable-catalog
#[test]
fn discoverable_bundle_is_served_at_the_well_known_path() {
    let report = validate(&parse(SPEC_EXAMPLE).expect("parses"));
    assert_eq!(
        report.conformance(),
        Conformance::Met(ConformanceLevel::Discoverable)
    );
    let s = served(SPEC_EXAMPLE);
    let r = handle(&s, &request("GET", WELL_KNOWN_PATH));
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(body_bytes(&r), SPEC_EXAMPLE);
}

#[test]
fn get_body_is_the_bundle_bytes_verbatim() {
    let s = served(UNCANONICAL);
    let r = handle(&s, &request("GET", WELL_KNOWN_PATH));
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(body_bytes(&r), UNCANONICAL);
}

// RFC 9110 section 9.3.2: HEAD carries the GET header fields and no content.
#[test]
fn head_returns_the_get_headers_and_no_body() {
    let s = served(BUNDLE);
    let get = handle(&s, &request("GET", WELL_KNOWN_PATH));
    let head = handle(&s, &request("HEAD", WELL_KNOWN_PATH));
    assert_eq!(head.status(), get.status());
    assert_eq!(head.headers(), get.headers());
    assert!(body_bytes(&head).is_empty());
}

#[test]
fn other_paths_are_not_found() {
    let s = served(BUNDLE);
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
    let s = served(BUNDLE);
    for method in ["POST", "PUT", "DELETE", "OPTIONS"] {
        let r = handle(&s, &request(method, WELL_KNOWN_PATH));
        assert_eq!(r.status().as_u16(), 405, "{method}");
        let allow: Vec<_> = r.headers().get_all("allow").iter().collect();
        assert_eq!(allow.len(), 1, "{method}");
        assert_eq!(allow[0], ALLOW, "{method}");
        assert!(body_bytes(&r).is_empty(), "{method}");
    }
}

// #well-known-uri: a bundle without `host` is a valid Level 1 catalog.
#[test]
fn minimal_bundle_without_host_is_served() {
    let report = validate(&parse(NO_HOST).expect("parses"));
    assert_eq!(
        report.conformance(),
        Conformance::Met(ConformanceLevel::Minimal)
    );
    let s = served(NO_HOST);
    let r = handle(&s, &request("GET", WELL_KNOWN_PATH));
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(body_bytes(&r), NO_HOST);
}

// #dynamic-discovery: content that varies by query is out of scope, so the query is ignored.
#[test]
fn query_string_does_not_change_the_response() {
    let s = served(BUNDLE);
    let plain = handle(&s, &request("GET", WELL_KNOWN_PATH));
    let queried = handle(&s, &request("GET", &format!("{WELL_KNOWN_PATH}?a=1&b=2")));
    assert_eq!(queried.status(), plain.status());
    assert_eq!(queried.headers(), plain.headers());
    assert_eq!(body_bytes(&queried), body_bytes(&plain));
}

// #well-known-uri: the path is the literal registered path, compared exactly.
#[test]
fn near_miss_paths_are_not_found() {
    let s = served(BUNDLE);
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
    let s = served(BUNDLE);
    let plain = handle(&s, &request("GET", WELL_KNOWN_PATH));
    let mut req = request("GET", WELL_KNOWN_PATH);
    req.headers_mut()
        .insert("accept", "text/html".parse().expect("header value"));
    let r = handle(&s, &req);
    assert_eq!(r.status(), plain.status());
    assert_eq!(r.headers(), plain.headers());
    assert_eq!(body_bytes(&r), body_bytes(&plain));
}
