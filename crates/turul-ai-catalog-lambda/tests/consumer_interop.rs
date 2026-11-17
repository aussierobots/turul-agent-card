//! Lambda to consumers: our client and ai-catalog-rust accept the served response and agree on values.

mod support;

use support::{body_bytes, request, served_from};
use turul_ai_catalog::identity::{MEDIA_TYPE, WELL_KNOWN_PATH};
use turul_ai_catalog::json::{Json, read};
use turul_ai_catalog_client::{Fetched, accept_response};
use turul_ai_catalog_lambda::handle;

const SPEC_EXAMPLE: &str = include_str!("fixtures/ai-catalog.json");

const MINIMAL: &str = r#"{"specVersion":"1.0","entries":[{"identifier":"urn:example:a","type":"application/a2a-agent-card+json","url":"https://example.com/a.json"}]}"#;

fn field<'a>(json: &'a Json, name: &str) -> Option<&'a Json> {
    match json {
        Json::Object(m) => m.get(name),
        _ => None,
    }
}

/// specVersion, host.displayName, host.identifier, and per entry in order identifier, type, version, url.
fn values(json: &Json) -> Vec<Option<Json>> {
    let mut out = vec![field(json, "specVersion").cloned()];
    let host = field(json, "host");
    out.push(host.and_then(|h| field(h, "displayName")).cloned());
    out.push(host.and_then(|h| field(h, "identifier")).cloned());
    if let Some(Json::Array(entries)) = field(json, "entries") {
        for e in entries {
            for name in ["identifier", "type", "version", "url"] {
                out.push(field(e, name).cloned());
            }
        }
    }
    out
}

// #well-known-uri, #location-independence
#[test]
fn client_and_sdk_accept_the_served_catalog_and_agree_on_values() {
    for document in [MINIMAL, SPEC_EXAMPLE] {
        let served = served_from(document);
        let r = handle(&served, &request("GET", WELL_KNOWN_PATH));
        let content_type = r
            .headers()
            .get("content-type")
            .map(|v| v.to_str().expect("ascii").to_owned());
        assert_eq!(content_type.as_deref(), Some(MEDIA_TYPE));
        let body = body_bytes(&r);

        // Our client: accepted, and no media-type-generic warning.
        let accepted = accept_response(&Fetched::new(
            r.status().as_u16(),
            content_type,
            body.clone(),
        ))
        .expect("client accepts the response");
        assert!(accepted.warnings.is_empty(), "{:?}", accepted.warnings);

        // ai-catalog-rust: parses and reports no errors (its level is never compared).
        let sdk = ai_catalog::parse_slice(&body).expect("sdk parses the body");
        let verdict = ai_catalog_validate::validate(&sdk);
        assert!(verdict.errors.is_empty(), "{:?}", verdict.errors);

        // Both agree on values.
        let ours = read(&body).expect("body reads");
        let theirs = read(
            ai_catalog::to_string(&sdk)
                .expect("sdk serialises")
                .as_bytes(),
        )
        .expect("sdk output reads");
        assert_eq!(values(&ours), values(&theirs));
    }
}
