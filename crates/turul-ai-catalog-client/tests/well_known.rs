//! Well-known lookup contract (client half).

mod support;

use support::loopback::{Reply, Server};
use turul_ai_catalog_client::{FetchPolicy, Source, WellKnownHost, load};

const CATALOG: &str = "application/ai-catalog+json";
const FIXTURE: &[u8] = include_bytes!("fixtures/ai-catalog.json");

// #well-known-uri: https://host + /.well-known/ai-catalog.json
#[test]
fn well_known_source_requests_the_well_known_path() {
    let host = WellKnownHost::new("h").unwrap();
    assert_eq!(
        Source::WellKnown(host).request_url().as_deref(),
        Some("https://h/.well-known/ai-catalog.json")
    );
    let host = WellKnownHost::new("h:8443").unwrap();
    assert_eq!(
        Source::WellKnown(host).request_url().as_deref(),
        Some("https://h:8443/.well-known/ai-catalog.json")
    );
    assert_eq!(Source::File("x.json".into()).request_url(), None);
    assert_eq!(
        Source::Url("https://h/c.json".into())
            .request_url()
            .as_deref(),
        Some("https://h/c.json")
    );
}

// #well-known-uri: a host carries no path, query, fragment or userinfo (RFC 8615: root of an origin).
#[test]
fn invalid_well_known_hosts_are_refused() {
    for h in ["h/x", "h?q", "h#f", "u@h", "https://h", ""] {
        assert!(WellKnownHost::new(h).is_err(), "{h:?} must be refused");
    }
}

// #well-known-uri: only a well-known source asks for the well-known URI.
#[test]
fn well_known_is_requested_only_for_a_well_known_source() {
    let mut p = FetchPolicy::default();
    p.permit_loopback_http = true;
    let s = Server::start(vec![
        Reply::redirect(302, "/elsewhere/catalog.json"),
        Reply::ok(CATALOG, FIXTURE),
    ]);
    load(&Source::Url(s.url("/catalog.json")), &p).expect("loads");
    let log = s.log();
    assert_eq!(log.len(), 2);
    assert!(log.iter().all(|l| !l.path().contains(".well-known")));
}
