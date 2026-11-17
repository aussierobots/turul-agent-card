//! Opt-in probe of a deployed handler. Never runs in CI.

use std::process::Command;
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_client::{FetchPolicy, Source, load};
use turul_ai_catalog_lambda::BUNDLE;

const URL_VAR: &str = "TURUL_AI_CATALOG_LIVE_URL";
const CLI_VAR: &str = "AI_CATALOG_CLI";

fn need(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| panic!("{name} is not set"))
}

// #well-known-uri
#[test]
#[ignore]
fn deployed_handler_serves_the_bundle_to_both_clients() {
    let url = need(URL_VAR);
    let cli = need(CLI_VAR);

    let loaded = load(&Source::Url(url.clone()), &FetchPolicy::default()).expect("client loads");
    assert_eq!(loaded.document, parse(BUNDLE).expect("bundle parses"));

    // Arguments for "fetch and validate a URL" are recorded in the ADR revision log.
    let status = Command::new(cli)
        .args(["validate", &url])
        .status()
        .expect("official CLI runs");
    assert!(status.success(), "official CLI exited {status}");
}

#[test]
#[ignore]
fn live_probe_requires_its_environment() {
    need(URL_VAR);
    need(CLI_VAR);
}
