//! The root `did:web` signer shape.

mod support;

use turul_ai_catalog::validate::{Conformance, validate};
use turul_ai_catalog_trust::DidWebRoot;

fn refused(values: &[&str]) {
    for v in values {
        assert_eq!(DidWebRoot::parse(v), None, "{v}");
    }
}

fn accepted(values: &[&str]) {
    for v in values {
        let d = DidWebRoot::parse(v).unwrap_or_else(|| panic!("{v}"));
        assert_eq!(d.as_str(), *v);
        assert_eq!(d.domain(), &v["did:web:".len()..]);
    }
}

#[test]
fn path_based_did_web_is_refused() {
    refused(&[
        "did:web:example.com:user",
        "did:web:example.com/user",
        "did:web:example.com?x",
        "did:web:example.com#k",
    ]);
}

#[test]
fn other_methods_and_prefix_spellings_are_refused() {
    refused(&[
        "did:key:z6Mk",
        "DID:web:example.com",
        "did:WEB:example.com",
        "did:web:",
        "",
        "example.com",
    ]);
}

#[test]
fn uppercase_domain_is_refused() {
    refused(&["did:web:Example.com", "did:web:example.COM"]);
}

#[test]
fn port_is_refused() {
    refused(&["did:web:example.com%3A8443", "did:web:example.com:8443"]);
}

#[test]
fn ip_address_spellings_are_refused() {
    refused(&[
        "did:web:192.0.2.1",
        "did:web:2130706433",
        "did:web:127.1",
        "did:web:0x7f.0.0.1",
        "did:web:example.0x7f",
        "did:web:example.0x",
        "did:web:[::1]",
    ]);
}

#[test]
fn trailing_dot_is_refused() {
    refused(&[
        "did:web:example.com.",
        "did:web:.example.com",
        "did:web:a..b",
    ]);
}

#[test]
fn u_label_is_refused() {
    refused(&["did:web:bücher.example", "did:web:例え.jp"]);
}

#[test]
fn a_label_and_digit_led_label_are_accepted() {
    accepted(&[
        "did:web:xn--bcher-kva.example",
        "did:web:a1.example",
        "did:web:1a.example",
        "did:web:example",
        "did:web:a-b.example",
        "did:web:0xg.example",
    ]);
    refused(&[
        "did:web:-a.example",
        "did:web:a-.example",
        "did:web:a_b.example",
    ]);
}

#[test]
fn label_and_domain_length_limits() {
    let l63 = "a".repeat(63);
    let l64 = "a".repeat(64);
    accepted(&[&format!("did:web:{l63}.example")]);
    refused(&[&format!("did:web:{l64}.example")]);
    // 253 bytes: 63 + 1 + 63 + 1 + 63 + 1 + 61.
    let d253 = format!("{l63}.{l63}.{l63}.{}", "a".repeat(61));
    assert_eq!(d253.len(), 253);
    accepted(&[&format!("did:web:{d253}")]);
    refused(&[&format!("did:web:{d253}a")]);
}

#[test]
fn signer_shape_defects_are_not_document_diagnostics() {
    let doc = |signer: &str| {
        format!(
            r#"{{"specVersion":"1.0","signature":{{"signer":"{signer}","profile":"did-web-v1","issuedAt":"2026-09-12T10:00:00Z","jws":"a..b"}},"entries":[]}}"#
        )
    };
    let baseline = validate(&support::catalog(&doc("did:web:example.com")));
    assert!(matches!(baseline.conformance(), Conformance::Met(_)));
    for signer in [
        "did:web:Example.com",
        "did:web:example.com%3A8443",
        "did:web:2130706433",
        "did:web:example.com:user",
    ] {
        let report = validate(&support::catalog(&doc(signer)));
        assert_eq!(report, baseline, "{signer}");
    }
}
