//! The digest a producer computes is the one a verifier recomputes.

mod support;

#[path = "../../turul-ai-catalog-client/tests/support/loopback.rs"]
mod loopback;

use loopback::{Reply, Server};
use support::world::{entry, loopback as policy, url_entry};
use turul_ai_catalog_trust::{IntegrityError, artifact_digest, compute, verify_artifact};

#[test]
fn data_digest_is_sha256_of_jcs() {
    let entry = entry();
    assert_eq!(
        artifact_digest(&entry, None, &policy()).unwrap(),
        compute("sha256", br#"{"a":1}"#).unwrap()
    );
}

#[test]
fn url_digest_matches_verification() {
    let body = br#"{"name":"served"}"#;
    let server = Server::start(vec![
        Reply::ok("application/json", body),
        Reply::ok("application/json", body),
    ]);
    let mut entry = url_entry(
        "urn:air:example.com:fixture:u",
        &server.url("/card.json"),
        b"",
    );
    let digest = artifact_digest(&entry, None, &policy()).unwrap();
    assert_eq!(digest, compute("sha256", body).unwrap());

    entry.digest = Some(digest);
    verify_artifact(&entry, None, &policy()).expect("verification accepts it");
    let log = server.log();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].header("accept"), log[1].header("accept"));
}

#[test]
fn non_200_is_refused() {
    let server = Server::start(vec![Reply::new(404)]);
    let entry = url_entry(
        "urn:air:example.com:fixture:u",
        &server.url("/missing"),
        b"",
    );
    assert!(matches!(
        artifact_digest(&entry, None, &policy()),
        Err(IntegrityError::Status(404))
    ));
}
