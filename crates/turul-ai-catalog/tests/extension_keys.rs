mod common;

use common::*;
use std::fs;
use std::path::PathBuf;
use turul_ai_catalog::extensions::{POLICY_URLS, PolicyUrlsError};
use turul_ai_catalog::validate::Severity::Error;

fn root_ext(key: &str, value: &str) -> String {
    format!(r#"{{"specVersion":"1.0","entries":[],"extensions":{{"{key}":{value}}}}}"#)
}

fn entry_ext(key: &str, value: &str) -> String {
    doc(&[&entry(
        "a",
        &format!(r#","extensions":{{"{key}":{value}}}"#),
    )])
}

fn policy_doc(value: &str) -> String {
    entry_ext(POLICY_URLS, value)
}

fn policy_ptr() -> String {
    format!("/entries/0/extensions/{}", esc(POLICY_URLS))
}

const VALID: &[&str] = &[
    "https://openknowledgeformat.org/ns#",
    "https://ai-catalog.org/extensions/metadata",
    "https://example.com/a?b=1#c",
    "com.example.confidenceScore",
    "io.acme.deploymentRegion",
    "a.b",
    "com.example-x_y.z9",
];

const INVALID: &[&str] = &[
    "https://",
    "https://a b/",
    "example",
    ".com.x",
    "com..x",
    "1com.x",
    "urn:x",
    "",
    "com.x.",
    "com.ex ample",
];

// #format-and-key-naming: "the keys MUST be a valid URL or a reverse-DNS string"
#[test]
fn url_and_reverse_dns_keys_are_valid() {
    for k in VALID {
        assert_clean(&report(&entry_ext(k, "1")));
        assert_clean(&report(&root_ext(k, "1")));
    }
}

// #format-and-key-naming: a key that is neither is reported at the member, escaped per RFC 6901
#[test]
fn malformed_keys_are_extension_key_invalid() {
    for k in INVALID {
        assert_only(
            &report(&entry_ext(k, "1")),
            "extension-key-invalid",
            Error,
            &format!("/entries/0/extensions/{}", esc(k)),
        );
        assert_only(
            &report(&root_ext(k, "1")),
            "extension-key-invalid",
            Error,
            &format!("/extensions/{}", esc(k)),
        );
    }
}

// #format-and-key-naming: parsing never fails on a key
#[test]
fn invalid_key_never_fails_parse() {
    for k in INVALID {
        let c = ok(&entry_ext(k, "1"));
        assert!(c.entries[0].extensions.as_ref().unwrap().get(k).is_some());
    }
}

// #format-and-key-naming: "Consumers that do not recognize an extension key MUST ignore it without throwing an error"
#[test]
fn unknown_valid_keys_give_no_diagnostic() {
    for v in ["1", r#""x""#, "null", r#"{"a":[1,"x",null]}"#, "[]"] {
        assert_clean(&report(&entry_ext("com.example.unknown", v)));
        assert_clean(&report(&root_ext("https://example.com/ns#unknown", v)));
    }
}

// #official-extensions: the metadata extension is "schemaless"
#[test]
fn metadata_value_is_not_validated() {
    for v in ["1", r#""s""#, "null", "[]", "true", r#"{"x":{"y":[]}}"#] {
        assert_clean(&report(&entry_ext(
            "https://ai-catalog.org/extensions/metadata",
            v,
        )));
    }
}

// #policy-urls-extension: "Its value MUST be a JSON object"
#[test]
fn non_object_policy_urls_is_policy_urls_invalid() {
    for v in [r#""x""#, "1", "[]", "null", "true"] {
        let r = report(&policy_doc(v));
        assert_only(&r, "policy-urls-invalid", Error, &policy_ptr());
        let e = ok(&policy_doc(v)).entries[0].policy_urls().unwrap_err();
        assert!(matches!(e, PolicyUrlsError::NotObject), "{v}");
    }
}

// #policy-urls-extension: "The following members are OPTIONAL"
#[test]
fn empty_policy_urls_object_is_valid() {
    assert_clean(&report(&policy_doc("{}")));
    let c = ok(&policy_doc("{}"));
    let p = c.entries[0].policy_urls().unwrap().unwrap();
    assert_eq!(p.privacy_policy_url, None);
    assert_eq!(p.terms_of_service_url, None);
}

// #policy-urls-extension: `privacyPolicyUrl` and `termsOfServiceUrl` are strings
#[test]
fn non_string_policy_url_member_is_policy_urls_invalid() {
    for (v, member) in [
        (r#"{"privacyPolicyUrl":1}"#, "privacyPolicyUrl"),
        (r#"{"termsOfServiceUrl":null}"#, "termsOfServiceUrl"),
        (r#"{"privacyPolicyUrl":[]}"#, "privacyPolicyUrl"),
        (r#"{"termsOfServiceUrl":{}}"#, "termsOfServiceUrl"),
    ] {
        assert_only(
            &report(&policy_doc(v)),
            "policy-urls-invalid",
            Error,
            &format!("{}/{member}", policy_ptr()),
        );
        let e = ok(&policy_doc(v)).entries[0].policy_urls().unwrap_err();
        assert!(
            matches!(e, PolicyUrlsError::MemberNotString(n) if &*n == member),
            "{v}"
        );
    }
}

// #policy-urls-extension: the example's links are returned; URL strings are not scheme-checked; unknown members ignored
#[test]
fn policy_urls_reader_returns_both_links() {
    let frag = fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/spec/blocks/13-policy-urls-extension.json"),
    )
    .unwrap();
    // The fragment is `{"extensions": ...}`; merge its members into an entry.
    let inner = frag
        .trim()
        .strip_prefix('{')
        .unwrap()
        .strip_suffix('}')
        .unwrap();
    let text = doc(&[&entry("a", &format!(",{inner}"))]);
    assert_clean(&report(&text));
    let c = ok(&text);
    let p = c.entries[0].policy_urls().unwrap().unwrap();
    assert_eq!(p.privacy_policy_url, Some("https://example.com/privacy"));
    assert_eq!(p.terms_of_service_url, Some("https://example.com/terms"));

    let c = ok(&policy_doc(
        r#"{"privacyPolicyUrl":"not a url","other":1,"termsOfServiceUrl":"ftp://x"}"#,
    ));
    assert_clean(&validate_of(&c));
    let p = c.entries[0].policy_urls().unwrap().unwrap();
    assert_eq!(p.privacy_policy_url, Some("not a url"));
    assert_eq!(p.terms_of_service_url, Some("ftp://x"));

    assert!(
        ok(&doc(&[&entry("a", "")])).entries[0]
            .policy_urls()
            .unwrap()
            .is_none()
    );
}

fn validate_of(c: &turul_ai_catalog::model::AiCatalog) -> turul_ai_catalog::validate::Report {
    turul_ai_catalog::validate::validate(c)
}

// #policy-urls-extension: "It is used in `entry.extensions`"; elsewhere it is an ordinary valid URL key
#[test]
fn policy_urls_outside_entry_extensions_is_an_ordinary_key() {
    for v in ["\"x\"", "1", "[]"] {
        assert_clean(&report(&root_ext(POLICY_URLS, v)));
    }
}
