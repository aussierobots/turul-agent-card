//! Signing: header, verification by the validator, snapshot coverage and producer refusals.

mod support;

use turul_ai_catalog::model::{AiCatalog, CatalogEntry, TrustManifest};
use turul_ai_catalog::validate::Code;
use turul_ai_catalog_trust::{
    DidWebRoot, SignError, SignerParams, SigningKey, catalog_payload, manifest_payload,
    sign_catalog, sign_manifest,
};
use turul_jwt_validator::detached::{
    DetachedJwsPolicy, JwsAlg, check_public_jwk, inspect_detached, verify_detached,
};

const TYP_MANIFEST: &str = "ai-catalog-trust-manifest+jws";
const TYP_CATALOG: &str = "ai-catalog+jws";
const DIGEST: &str = "sha256:56bbd2ff730d81a52951fc2f58809dfb086b4be7a82766dcc18cfe32ff6acbbd";
const SIGNER: &str = "did:web:acme.com";
const OTHER_ENTRY: &str = r#"{"identifier":"urn:air:acme.com:agent:other","type":"application/a2a-agent-card+json","url":"https://acme.com/o.json"}"#;

fn key() -> SigningKey {
    SigningKey::from_pkcs8_der(&support::fixture("keys/rfc7515-a3.pk8"))
}

fn params() -> SignerParams {
    SignerParams {
        signer: DidWebRoot::parse(SIGNER).unwrap(),
        key_fragment: "key-1".to_string(),
        issued_at: "2026-09-12T10:00:00Z".to_string(),
        expires_at: None,
    }
}

fn entry(extra: &str) -> CatalogEntry {
    let doc = format!(
        r#"{{"specVersion":"1.0","entries":[{{"identifier":"urn:air:acme.com:agent:finance","type":"application/a2a-agent-card+json","url":"https://acme.com/finance.json"{extra}}}]}}"#
    );
    support::catalog(&doc).entries.remove(0)
}

fn entry_with_digest() -> CatalogEntry {
    entry(&format!(r#","digest":"{DIGEST}""#))
}

fn manifest() -> TrustManifest {
    support::manifest(&format!(r#"{{"contributor":"{SIGNER}"}}"#))
}

fn jws_of(m: &TrustManifest) -> String {
    m.signature.as_ref().unwrap().jws.clone()
}

fn policy(typ: &'static str) -> DetachedJwsPolicy<'static> {
    DetachedJwsPolicy {
        expected_typ: typ,
        allowed_algs: &[JwsAlg::Es256],
        forbidden_params: &["jku", "jwk", "x5u", "x5c"],
    }
}

/// Whether the validator accepts `jws` over `payload` with the A.3 public key.
fn verifies(jws: &str, typ: &'static str, payload: &[u8]) -> bool {
    let jwk = support::fixture("keys/rfc7515-a3.public.jwk.json");
    let public = check_public_jwk(&jwk).unwrap();
    verify_detached(jws, payload, &public, &policy(typ)).is_ok()
}

fn catalog_text() -> String {
    String::from_utf8(support::fixture(
        "signed_payload/catalog-snapshot.input.json",
    ))
    .unwrap()
}

fn catalog_text_with_other(other_first: bool) -> String {
    let text = catalog_text();
    if other_first {
        text.replace(r#""entries": ["#, &format!(r#""entries": [{OTHER_ENTRY},"#))
    } else {
        text.replace("\n    }\n  ],", &format!("\n    }},{OTHER_ENTRY}\n  ],"))
    }
}

fn signed_catalog(text: &str) -> AiCatalog {
    sign_catalog(support::catalog(text), &params(), &key()).unwrap()
}

/// Whether `jws` (from `signed`) still covers the catalog parsed from `text`.
fn still_covers(signed: &AiCatalog, text: &str) -> bool {
    let mut other = support::catalog(text);
    other.signature = signed.signature.clone();
    let jws = signed.signature.as_ref().unwrap().jws.clone();
    verifies(&jws, TYP_CATALOG, &catalog_payload(&other).unwrap())
}

#[test]
fn output_is_detached_with_header_alg_typ_kid() {
    let signed = sign_manifest(&entry_with_digest(), manifest(), &params(), &key()).unwrap();
    let jws = jws_of(&signed);
    let (head, rest) = jws.split_once("..").unwrap();
    assert!(!head.is_empty() && !rest.is_empty() && !rest.contains('.'));
    let header = inspect_detached(&jws, &policy(TYP_MANIFEST)).unwrap();
    assert_eq!(header.len(), 3);
    assert_eq!(header.get("alg").and_then(|v| v.as_str()), Some("ES256"));
    assert_eq!(
        header.get("typ").and_then(|v| v.as_str()),
        Some(TYP_MANIFEST)
    );
    assert_eq!(
        header.get("kid").and_then(|v| v.as_str()),
        Some("did:web:acme.com#key-1")
    );
    let catalog = signed_catalog(&catalog_text());
    let header = inspect_detached(&catalog.signature.unwrap().jws, &policy(TYP_CATALOG)).unwrap();
    assert_eq!(
        header.get("typ").and_then(|v| v.as_str()),
        Some(TYP_CATALOG)
    );
}

#[test]
fn validator_verifies_the_output() {
    let signed = sign_manifest(&entry_with_digest(), manifest(), &params(), &key()).unwrap();
    assert_eq!(signed.signature.as_ref().unwrap().profile, "did-web-v1");
    assert!(verifies(
        &jws_of(&signed),
        TYP_MANIFEST,
        &manifest_payload(&signed).unwrap()
    ));
    let catalog = signed_catalog(&catalog_text());
    assert!(still_covers(&catalog, &catalog_text()));
    // A manifest signature is not a catalog signature.
    assert!(!verifies(
        &jws_of(&signed),
        TYP_CATALOG,
        &manifest_payload(&signed).unwrap()
    ));
}

#[test]
fn any_payload_change_breaks_the_signature() {
    let signed = sign_manifest(&entry_with_digest(), manifest(), &params(), &key()).unwrap();
    let jws = jws_of(&signed);
    let mut unknown = signed.clone();
    unknown.unrecognized =
        support::manifest(r#"{"contributor":"did:web:acme.com","x-new":1}"#).unrecognized;
    assert!(!verifies(
        &jws,
        TYP_MANIFEST,
        &manifest_payload(&unknown).unwrap()
    ));
    let mut later = signed.clone();
    later.signature.as_mut().unwrap().issued_at = "2026-09-12T10:00:01Z".to_string();
    assert!(!verifies(
        &jws,
        TYP_MANIFEST,
        &manifest_payload(&later).unwrap()
    ));
}

#[test]
fn subject_version_is_present_exactly_when_the_entry_has_one() {
    let without = sign_manifest(&entry_with_digest(), manifest(), &params(), &key()).unwrap();
    assert_eq!(without.subject.unwrap().version, None);
    let with = entry(&format!(r#","version":"2.1.0","digest":"{DIGEST}""#));
    let signed = sign_manifest(&with, manifest(), &params(), &key()).unwrap();
    assert_eq!(signed.subject.unwrap().version.as_deref(), Some("2.1.0"));
}

#[test]
fn sign_catalog_covers_the_whole_snapshot() {
    let text = catalog_text();
    let signed = signed_catalog(&text);
    assert!(still_covers(&signed, &text));
    for (from, to) in [
        (r#""specVersion": "1.0""#, r#""specVersion": "1.1""#),
        ("Acme", "Acme2"),
        ("kept", "changed"),
        (r#""x-unknown": {"#, r#""x-unknown": {"extra": 1,"#),
        ("2026-09-12T10:00:00Z", "2026-09-12T10:00:01Z"),
        ("aGVhZGVy..c2ln", "aGVhZGVy..c2lk"),
        (
            r#""contributor": "did:web:acme.com","#,
            r#""contributor": "did:web:acme.com","x-m": 1,"#,
        ),
    ] {
        assert!(text.contains(from), "{from}");
        assert!(!still_covers(&signed, &text.replace(from, to)), "{from}");
    }
    let extra = text.replace(
        r#""additionalSignatures": [{"jws": "b3RoZXI..c2ln"}]"#,
        r#""additionalSignatures": [{"jws": "b3RoZXI..c2ln"},{"jws": "bW9yZQ..c2ln"}]"#,
    );
    assert_ne!(extra, text);
    assert!(still_covers(&signed, &extra));
}

#[test]
fn reordering_entries_breaks_the_catalog_signature() {
    let first = catalog_text_with_other(true);
    let signed = signed_catalog(&first);
    assert!(still_covers(&signed, &first));
    assert!(!still_covers(&signed, &catalog_text_with_other(false)));
}

#[test]
fn adding_or_removing_an_entry_breaks_the_catalog_signature() {
    let base = catalog_text();
    let signed = signed_catalog(&base);
    assert!(!still_covers(&signed, &catalog_text_with_other(true)));
    let with_other = catalog_text_with_other(true);
    let signed = signed_catalog(&with_other);
    assert!(!still_covers(&signed, &base));
}

#[test]
fn truncated_der_is_a_key_error_at_sign_time() {
    let der = support::fixture("keys/rfc7515-a3.pk8");
    let bad = SigningKey::from_pkcs8_der(&der[..40]);
    assert_eq!(
        sign_manifest(&entry_with_digest(), manifest(), &params(), &bad),
        Err(SignError::Key)
    );
}

#[test]
fn entry_without_digest_is_digest_missing() {
    assert_eq!(
        sign_manifest(&entry(""), manifest(), &params(), &key()),
        Err(SignError::DigestMissing)
    );
}

#[test]
fn contributor_other_than_signer_is_refused() {
    let m = support::manifest(r#"{"contributor":"did:web:other.example"}"#);
    assert_eq!(
        sign_manifest(&entry_with_digest(), m, &params(), &key()),
        Err(SignError::ContributorMismatch)
    );
}

#[test]
fn differing_subject_is_refused() {
    let m = support::manifest(&format!(
        r#"{{"contributor":"{SIGNER}","subject":{{"identifier":"urn:air:acme.com:agent:other","type":"application/a2a-agent-card+json","digest":"{DIGEST}"}}}}"#
    ));
    assert_eq!(
        sign_manifest(&entry_with_digest(), m, &params(), &key()),
        Err(SignError::SubjectMismatch)
    );
}

#[test]
fn invalid_manifest_is_not_signed() {
    let mut p = params();
    p.issued_at = "not a time".to_string();
    let err = sign_manifest(&entry_with_digest(), manifest(), &p, &key()).unwrap_err();
    assert!(matches!(err, SignError::Invalid(_)), "{err:?}");
    let mut p = params();
    p.issued_at = "2026-09-12T10:00:00Z".to_string();
    p.expires_at = Some("2026-09-12T09:00:00Z".to_string());
    assert_eq!(
        sign_manifest(&entry_with_digest(), manifest(), &p, &key()),
        Err(SignError::Invalid(Code::SignatureExpiryOrder))
    );
}

// #the-did-web-signer-profile: "The Signature object's `signer` MUST be a root `did:web` DID".
// `SignerParams.signer` is a `DidWebRoot`, so no other shape reaches a `did-web-v1` signature.
#[test]
fn signer_outside_the_did_web_profile_cannot_sign() {
    for bad in [
        "https://example.com/x",
        "did:web:Example.COM",
        "did:web:example.com%3A8443",
        "did:web:example.com:8443",
        "did:web:example.com:user:alice",
        "did:web:127.0.0.1",
        "did:example:123",
    ] {
        assert_eq!(DidWebRoot::parse(bad), None, "{bad}");
    }
    let signed = sign_manifest(&entry_with_digest(), manifest(), &params(), &key()).unwrap();
    assert_eq!(signed.signature.unwrap().signer, SIGNER);
}

#[test]
fn kid_fragment_outside_rfc_3986_is_refused() {
    for fragment in ["", "a b", "a#b", "a%zz", "a%2", "é"] {
        let mut p = params();
        p.key_fragment = fragment.to_string();
        assert_eq!(
            sign_manifest(&entry_with_digest(), manifest(), &p, &key()),
            Err(SignError::Kid),
            "{fragment:?}"
        );
    }
    let mut p = params();
    p.key_fragment = "key-1%2F_~".to_string();
    assert!(sign_manifest(&entry_with_digest(), manifest(), &p, &key()).is_ok());
}
