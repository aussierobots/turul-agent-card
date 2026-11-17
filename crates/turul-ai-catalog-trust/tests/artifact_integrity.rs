//! Artifact digests and integrity checks.

mod support;

#[path = "../../turul-ai-catalog-client/tests/support/loopback.rs"]
mod loopback;

use loopback::{Reply, Server};
use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
use turul_ai_catalog::model::{ArtifactSource, CatalogEntry};
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_client::{FetchError, FetchPolicy, Source, load};
use turul_ai_catalog_trust::{IntegrityError, Unsupported, compute, verify_artifact};

const EMPTY_SHA256: &str =
    "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn policy() -> FetchPolicy {
    let mut p = FetchPolicy::default();
    p.permit_loopback_http = true;
    p
}

fn entry(source: ArtifactSource, digest: Option<&str>) -> CatalogEntry {
    let mut e = CatalogEntry::new(
        EntryIdentifier::new("urn:air:example.com:fixture:a"),
        ArtifactType::new("application/json"),
        source,
    );
    e.digest = digest.map(str::to_string);
    e
}

fn url_entry(url: &str, digest: &str) -> CatalogEntry {
    entry(ArtifactSource::Url(url.to_string()), Some(digest))
}

fn data(json: &str) -> ArtifactSource {
    let doc = format!(
        r#"{{"specVersion":"1.0","entries":[{{"identifier":"urn:air:example.com:fixture:d","type":"application/json","data":{json}}}]}}"#
    );
    parse(doc.as_bytes()).unwrap().entries.remove(0).source
}

fn is_mismatch(r: Result<(), IntegrityError>) -> bool {
    matches!(r, Err(IntegrityError::Mismatch { .. }))
}

// #digest-format
#[test]
fn sha256_of_empty_input_is_the_known_value() {
    assert_eq!(compute("sha256", b"").unwrap(), EMPTY_SHA256);
}

#[test]
fn sha384_and_sha512_are_computed() {
    assert_eq!(
        compute("sha384", b"").unwrap(),
        "sha384:38b060a751ac96384cd9327eb1b1e36a21fdb71114be07434c0cc7bf63f6e1da274edebfe76f65fbd51ad2f14898b95b"
    );
    assert_eq!(
        compute("sha512", b"").unwrap(),
        "sha512:cf83e1357eefb8bdf1542850d66d8007d620e4050b5715dc83f4a921d36ce9ce47d0d13c5d85f2b0ff8318d2877eec2f63b931bd47417a81a538327af927da3e"
    );
}

#[test]
fn algorithm_name_is_case_insensitive() {
    assert_eq!(compute("SHA256", b"").unwrap(), EMPTY_SHA256);
    assert_eq!(compute("Sha256", b"").unwrap(), EMPTY_SHA256);
}

#[test]
fn unknown_algorithm_is_unsupported() {
    let digest = format!("sha3-256:{}", "a".repeat(64));
    let e = entry(data(r#"{"a":1}"#), Some(&digest));
    match verify_artifact(&e, None, &policy()) {
        Err(IntegrityError::Unsupported(Unsupported::Algorithm(name))) => {
            assert_eq!(name, "sha3-256");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn missing_and_invalid_digests_are_distinct() {
    let src = data("{}");
    assert!(matches!(
        verify_artifact(&entry(src.clone(), None), None, &policy()),
        Err(IntegrityError::DigestMissing)
    ));
    assert!(matches!(
        verify_artifact(&entry(src, Some("sha256:zz")), None, &policy()),
        Err(IntegrityError::DigestInvalid)
    ));
}

// verifying-artifact-integrity-1
#[test]
fn data_entry_hashes_its_jcs_bytes() {
    // Members out of order and spaced: only the JCS bytes `{"a":2,"b":1}` match.
    let src = data(r#"{ "b": 1, "a": 2 }"#);
    let digest = compute("sha256", br#"{"a":2,"b":1}"#).unwrap();
    verify_artifact(&entry(src, Some(&digest)), None, &policy()).unwrap();
}

// verifying-artifact-integrity-1
#[test]
fn url_entry_hashes_the_retrieved_body() {
    let body = b"artifact bytes, not JSON \xff";
    let server = Server::start(vec![Reply::ok("application/octet-stream", body)]);
    let digest = compute("sha256", body).unwrap();
    verify_artifact(&url_entry(&server.url("/a"), &digest), None, &policy()).unwrap();
    assert_eq!(server.log().len(), 1);
    assert_eq!(server.log()[0].header("accept-encoding"), Some("identity"));
}

// verifying-artifact-integrity-1
#[test]
fn mismatch_fails() {
    let wrong = compute("sha256", b"other").unwrap();
    assert!(is_mismatch(verify_artifact(
        &entry(data(r#"{"a":1}"#), Some(&wrong)),
        None,
        &policy()
    )));
    let server = Server::start(vec![Reply::ok("application/json", b"{}")]);
    assert!(is_mismatch(verify_artifact(
        &url_entry(&server.url("/a"), &wrong),
        None,
        &policy()
    )));
}

// verifying-artifact-integrity-2
#[test]
fn source_digest_is_never_substituted() {
    let body = b"{}";
    let actual = compute("sha256", body).unwrap();
    let wrong = compute("sha256", b"other").unwrap();
    let doc = format!(
        r#"{{"specVersion":"1.0","entries":[{{"identifier":"urn:air:example.com:fixture:a","type":"application/json","url":"https://example.com/a.json","digest":"{wrong}","trustManifests":[{{"contributor":"did:web:acme.com","provenance":[{{"relation":"publishedFrom","sourceId":"s","sourceDigest":"{actual}"}}],"extensions":{{"https://example.com/x":true}}}}]}}]}}"#
    );
    let mut e = parse(doc.as_bytes()).unwrap().entries.remove(0);
    let server = Server::start(vec![Reply::ok("application/json", body)]);
    e.source = ArtifactSource::Url(server.url("/a"));
    assert!(is_mismatch(verify_artifact(&e, None, &policy())));
}

#[test]
fn verify_artifact_compares_entry_digest_only() {
    let body = b"{}";
    let actual = compute("sha256", body).unwrap();
    let wrong = compute("sha256", b"other").unwrap();
    let doc = format!(
        r#"{{"specVersion":"1.0","entries":[{{"identifier":"urn:air:example.com:fixture:a","type":"application/json","url":"https://example.com/a.json","digest":"{wrong}","trustManifests":[{{"contributor":"did:web:acme.com","subject":{{"identifier":"urn:air:example.com:fixture:a","type":"application/json","digest":"{actual}"}},"attestations":[{{"type":"t","uri":"https://example.com/t","digest":"{actual}"}}]}}]}}]}}"#
    );
    let mut e = parse(doc.as_bytes()).unwrap().entries.remove(0);
    let server = Server::start(vec![Reply::ok("application/json", body)]);
    e.source = ArtifactSource::Url(server.url("/a"));
    assert!(is_mismatch(verify_artifact(&e, None, &policy())));
}

#[test]
fn non_200_status_is_refused() {
    let body = b"{}";
    let digest = compute("sha256", body).unwrap();
    let server = Server::start(vec![Reply::new(404).body(body)]);
    assert!(matches!(
        verify_artifact(&url_entry(&server.url("/a"), &digest), None, &policy()),
        Err(IntegrityError::Status(404))
    ));
}

// well-known-uri-3
#[test]
fn relative_url_resolves_against_the_catalog_base() {
    let body = b"the artifact";
    let digest = compute("sha256", body).unwrap();
    let catalog = format!(
        r#"{{"specVersion":"1.0","entries":[{{"identifier":"urn:air:example.com:fixture:a","type":"application/json","url":"a.json","digest":"{digest}"}}]}}"#
    );
    let server = Server::start(vec![
        Reply::ok("application/ai-catalog+json", catalog.as_bytes()),
        Reply::ok("application/json", body),
    ]);
    let loaded = load(&Source::Url(server.url("/dir/catalog.json")), &policy()).unwrap();
    verify_artifact(
        &loaded.document.entries[0],
        loaded.base.as_deref(),
        &policy(),
    )
    .unwrap();
    assert_eq!(server.log().last().unwrap().path(), "/dir/a.json");
}

// well-known-uri-3
#[test]
fn relative_url_from_a_file_root_is_unresolvable() {
    let e = url_entry("a.json", EMPTY_SHA256);
    assert!(matches!(
        verify_artifact(&e, None, &policy()),
        Err(IntegrityError::Unresolvable)
    ));
}

#[test]
fn data_uri_url_is_unsupported_without_a_request() {
    let server = Server::silent();
    for url in ["data:text/plain,hi", "DATA:text/plain,hi"] {
        let e = url_entry(url, EMPTY_SHA256);
        assert!(
            matches!(
                verify_artifact(&e, None, &policy()),
                Err(IntegrityError::Unsupported(Unsupported::DataUri))
            ),
            "{url}"
        );
    }
    assert!(server.log().is_empty());
}

// Fetch ceiling
#[test]
fn body_over_the_ceiling_is_a_fetch_error() {
    let body = vec![0u8; 4 * 1024 * 1024 + 1];
    let server = Server::start(vec![Reply::ok("application/octet-stream", &body)]);
    assert!(matches!(
        verify_artifact(&url_entry(&server.url("/a"), EMPTY_SHA256), None, &policy()),
        Err(IntegrityError::Fetch(FetchError::TooLarge))
    ));
}

// a number inside `data` that is not a finite double
#[test]
fn data_that_is_not_i_json_is_refused() {
    let e = entry(data(r#"{"n":1e999}"#), Some(EMPTY_SHA256));
    match verify_artifact(&e, None, &policy()) {
        Err(IntegrityError::NotIJson { pointer }) => assert_eq!(pointer.to_string(), "/n"),
        other => panic!("{other:?}"),
    }
}
