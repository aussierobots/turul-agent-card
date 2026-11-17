//! Cold-start gate contract: `ServedCatalog::load` over the bundle and over variants.

mod support;

use support::{body_bytes, leak, request};
use turul_ai_catalog::identity::WELL_KNOWN_PATH;
use turul_ai_catalog::json::{Json, read};
use turul_ai_catalog::parse::{ParseErrorKind, parse};
use turul_ai_catalog::validate::{Conformance, ConformanceLevel, Severity, validate};
use turul_ai_catalog_lambda::{BUNDLE, ColdStartError, ServedCatalog, handle};

const SPEC_EXAMPLE: &str = include_str!("fixtures/ai-catalog.json");

type Case = (&'static [u8], fn(&ParseErrorKind) -> bool, &'static str);

const ENTRY: &str = r#"{"identifier":"urn:example:a","type":"application/a2a-agent-card+json","url":"https://example.com/a.json"}"#;

fn doc(entry: &str) -> &'static [u8] {
    leak(format!(r#"{{"specVersion":"1.0","entries":[{entry}]}}"#))
}

/// The spec example with its one signature's `jws` replaced.
fn with_jws(jws: &str) -> &'static [u8] {
    let start = SPEC_EXAMPLE.find(r#""jws": ""#).expect("jws member") + r#""jws": ""#.len();
    let end = start + SPEC_EXAMPLE[start..].find('"').expect("jws end");
    leak(format!(
        "{}{}{}",
        &SPEC_EXAMPLE[..start],
        jws,
        &SPEC_EXAMPLE[end..]
    ))
}

fn assert_starts_and_serves(bytes: &'static [u8]) {
    let served = match ServedCatalog::load(bytes) {
        Ok(s) => s,
        Err(e) => panic!("must start: {e}"),
    };
    let r = handle(&served, &request("GET", WELL_KNOWN_PATH));
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(body_bytes(&r), bytes);
}

fn member<'a>(json: &'a Json, name: &str) -> &'a Json {
    match json {
        Json::Object(m) => m.get(name).unwrap_or_else(|| panic!("member {name}")),
        other => panic!("not an object: {other:?}"),
    }
}

fn entry_triples(json: &Json) -> Vec<(Json, Json, Json)> {
    match member(json, "entries") {
        Json::Array(items) => items
            .iter()
            .map(|e| {
                (
                    member(e, "identifier").clone(),
                    member(e, "type").clone(),
                    member(e, "url").clone(),
                )
            })
            .collect(),
        other => panic!("entries not an array: {other:?}"),
    }
}

// #level-1-minimal-catalog, #catalog-entry: an entry with an empty identifier is an error.
#[test]
fn bundle_with_an_error_diagnostic_fails_cold_start() {
    let bytes = doc(
        r#"{"identifier":"","type":"application/a2a-agent-card+json","url":"https://example.com/a.json"}"#,
    );
    match ServedCatalog::load(bytes) {
        Err(ColdStartError::Invalid(report)) => assert!(report.has_errors()),
        Err(e) => panic!("wrong error: {e}"),
        Ok(_) => panic!("an error diagnostic must fail cold start"),
    }
}

// #media-type-registration-application-ai-catalog-json: the document MUST contain `specVersion` and `entries`.
#[test]
fn unparseable_bundle_fails_cold_start() {
    let cases: [Case; 3] = [
        (b"{", |k| matches!(k, ParseErrorKind::Syntax), ""),
        (
            br#"{"specVersion":"1.0"}"#,
            |k| matches!(k, ParseErrorKind::MissingMember),
            "/entries",
        ),
        (
            br#"{"specVersion":"2.0","entries":[]}"#,
            |k| matches!(k, ParseErrorKind::UnsupportedMajor { .. }),
            "/specVersion",
        ),
    ];
    for (bytes, is_kind, pointer) in cases {
        match ServedCatalog::load(bytes) {
            Err(ColdStartError::Parse(e)) => {
                assert!(is_kind(&e.kind), "{bytes:?}: {:?}", e.kind);
                if !pointer.is_empty() {
                    assert_eq!(e.pointer.to_string(), pointer, "{bytes:?}");
                }
            }
            Err(e) => panic!("wrong error: {e}"),
            Ok(_) => panic!("{bytes:?} must fail cold start"),
        }
    }
}

// #cddl-schema "digest": an unknown algorithm is a warning; #catalog-entry nesting limit gives Undetermined.
#[test]
fn warnings_do_not_fail_cold_start() {
    let unknown_digest = doc(&format!(
        r#"{{"identifier":"urn:example:a","type":"application/a2a-agent-card+json","url":"https://example.com/a.json","digest":"sha3-256:{}"}}"#,
        "a".repeat(64)
    ));
    let report = validate(&parse(unknown_digest).expect("parses"));
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|d| d.code.as_str() == "digest-algorithm-unknown"
                && d.severity == Severity::Warning)
    );
    assert_starts_and_serves(unknown_digest);

    // Root plus four nested inline catalogs: the fifth level is not descended.
    let mut nested = format!(r#"{{"specVersion":"1.0","entries":[{ENTRY}]}}"#);
    for n in 0..4 {
        nested = format!(
            r#"{{"specVersion":"1.0","entries":[{{"identifier":"urn:example:n{n}","type":"application/ai-catalog+json","data":{nested}}}]}}"#
        );
    }
    let bytes = leak(nested);
    let report = validate(&parse(bytes).expect("parses"));
    assert_eq!(report.conformance(), Conformance::Undetermined);
    assert!(!report.has_errors());
    assert_starts_and_serves(bytes);
}

// The committed bundle passes the one gate.
#[test]
fn bundled_catalog_loads() {
    if let Err(e) = ServedCatalog::load(BUNDLE) {
        panic!("the shipped catalog.json must load: {e}");
    }
}

// #level-2-discoverable-catalog: default content is the spec example's host and three entries.
#[test]
fn bundled_catalog_is_discoverable() {
    let report = validate(&parse(BUNDLE).expect("parses"));
    assert!(!report.has_errors(), "{:?}", report.diagnostics());
    assert_eq!(
        report.conformance(),
        Conformance::Met(ConformanceLevel::Discoverable)
    );
    let ours = read(BUNDLE).expect("bundle reads");
    let spec = read(SPEC_EXAMPLE.as_bytes()).expect("example reads");
    assert_eq!(entry_triples(&ours), entry_triples(&spec));
    assert_eq!(member(&ours, "host"), member(&spec, "host"));
}

#[test]
fn cold_start_error_names_each_error_code_and_pointer() {
    let bytes = doc(r#"{"identifier":"","type":"","url":"https://example.com/a.json"}"#);
    let Err(err) = ServedCatalog::load(bytes) else {
        panic!("must fail cold start");
    };
    let ColdStartError::Invalid(report) = &err else {
        panic!("wrong error: {err}");
    };
    let text = err.to_string();
    let errors: Vec<_> = report
        .diagnostics()
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert!(errors.len() >= 2, "{errors:?}");
    for d in errors {
        assert!(text.contains(d.code.as_str()), "{text}");
        assert!(text.contains(&d.path.to_string()), "{text}");
    }
    assert!(text.contains("identifier-empty") && text.contains("/entries/0/identifier"));
    assert!(text.contains("type-empty") && text.contains("/entries/0/type"));
}

// #signed-payload-and-jws-construction, #did-web: header and signer shape are never checked by the format crate.
#[test]
fn signature_defects_do_not_fail_cold_start() {
    // base64url of `{}` and of `{"alg":"none"}`
    assert_starts_and_serves(with_jws("e30..detached-jws-signature"));
    assert_starts_and_serves(with_jws("eyJhbGciOiJub25lIn0..detached-jws-signature"));
    // `contributor` changes with `signer`: #contributor-identity "MUST exactly equal" stays met, so
    // the only defect is the `did:web` signer shape.
    let signer = SPEC_EXAMPLE
        .replacen(
            r#""contributor": "did:web:acme-corp.com""#,
            r#""contributor": "did:web:Example.com""#,
            1,
        )
        .replacen(
            r#""signer": "did:web:acme-corp.com""#,
            r#""signer": "did:web:Example.com""#,
            1,
        );
    assert_eq!(signer.matches("did:web:Example.com").count(), 2);
    assert_starts_and_serves(leak(signer));
}

// #cddl-schema: a Signature requires `signer`, `profile`, `issuedAt` and `jws`.
#[test]
fn signature_missing_a_required_member_fails_cold_start() {
    let missing = SPEC_EXAMPLE.replacen(r#""profile": "did-web-v1","#, "", 1);
    assert_ne!(missing, SPEC_EXAMPLE);
    match ServedCatalog::load(leak(missing)) {
        Err(ColdStartError::Parse(e)) => {
            assert_eq!(e.kind, ParseErrorKind::MissingMember);
            assert_eq!(
                e.pointer.to_string(),
                "/entries/0/trustManifests/0/signature/profile"
            );
        }
        Err(e) => panic!("wrong error: {e}"),
        Ok(_) => panic!("a missing signature member must fail cold start"),
    }
}

// #nested-catalog-entries: a `url`-nested catalog is a separate document and is not fetched.
#[test]
fn url_nested_children_are_not_fetched_at_cold_start() {
    assert_starts_and_serves(doc(
        r#"{"identifier":"urn:example:child","type":"application/ai-catalog+json","url":"https://127.0.0.1/x.json"}"#,
    ));
}

// #signed-payload-and-jws-construction: I-JSON is required of signed objects only.
#[test]
fn non_i_json_numbers_outside_signed_objects_do_not_fail_cold_start() {
    assert_starts_and_serves(doc(
        r#"{"identifier":"urn:example:a","type":"application/vnd.example+json","data":{"n":1e400}}"#,
    ));
}
