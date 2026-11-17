//! The protected header of a `did-web-v1` signature: policy, validator refusals and `kid`.

mod support;

use turul_ai_catalog::model::{AiCatalog, TrustManifest};
use turul_ai_catalog::validate::{Conformance, validate};
use turul_ai_catalog_trust::{
    DID_WEB_V1_FORBIDDEN_HEADERS, DidWebRoot, ES256, HeaderError, SignedObject, SignerParams,
    SigningKey, TYP_CATALOG, TYP_TRUST_MANIFEST, inspect, kid_matches_signer, policy, sign_catalog,
    sign_manifest,
};
use turul_jwt_validator::detached::DetachedJwsError;

const SIGNER: &str = "did:web:acme.com";
const KID: &str = "did:web:acme.com#k";

fn manifest() -> TrustManifest {
    support::manifest(&format!(r#"{{"contributor":"{SIGNER}"}}"#))
}

fn catalog() -> AiCatalog {
    AiCatalog::new(vec![])
}

fn check_manifest(header: &str) -> Result<String, HeaderError> {
    let m = manifest();
    inspect(
        SignedObject::Manifest(&m),
        &support::signature(SIGNER, &support::jws_with(header)),
    )
    .map(|k| k.as_str().to_string())
}

fn check_catalog(header: &str) -> Result<String, HeaderError> {
    let c = catalog();
    inspect(
        SignedObject::Catalog(&c),
        &support::signature(SIGNER, &support::jws_with(header)),
    )
    .map(|k| k.as_str().to_string())
}

fn manifest_header(extra: &str) -> String {
    format!(r#"{{"alg":"ES256","typ":"{TYP_TRUST_MANIFEST}","kid":"{KID}"{extra}}}"#)
}

fn jws_err(e: Result<String, HeaderError>) -> DetachedJwsError {
    match e {
        Err(HeaderError::Jws(e)) => e,
        other => panic!("expected a validator refusal, got {other:?}"),
    }
}

fn kid_check(kid: &str) -> Result<String, HeaderError> {
    check_manifest(&format!(
        r#"{{"alg":"ES256","typ":"{TYP_TRUST_MANIFEST}","kid":"{kid}"}}"#
    ))
}

#[test]
fn policy_is_es256_with_kind_typ_and_forbidden_headers() {
    let m = manifest();
    let c = catalog();
    for (object, typ) in [
        (SignedObject::Manifest(&m), TYP_TRUST_MANIFEST),
        (SignedObject::Catalog(&c), TYP_CATALOG),
    ] {
        let p = policy(object);
        assert_eq!(p.expected_typ, typ);
        assert_eq!(p.allowed_algs, [ES256]);
        assert_eq!(p.forbidden_params, DID_WEB_V1_FORBIDDEN_HEADERS);
    }
    assert_eq!(TYP_TRUST_MANIFEST, "ai-catalog-trust-manifest+jws");
    assert_eq!(TYP_CATALOG, "ai-catalog+jws");
    assert_eq!(DID_WEB_V1_FORBIDDEN_HEADERS, ["jku", "jwk", "x5u", "x5c"]);
}

#[test]
fn header_without_alg_is_invalid_header() {
    let h = format!(r#"{{"typ":"{TYP_TRUST_MANIFEST}","kid":"{KID}"}}"#);
    assert_eq!(jws_err(check_manifest(&h)), DetachedJwsError::InvalidHeader);
}

#[test]
fn header_without_typ_is_typ_mismatch() {
    let h = format!(r#"{{"alg":"ES256","kid":"{KID}"}}"#);
    assert_eq!(jws_err(check_manifest(&h)), DetachedJwsError::TypMismatch);
}

#[test]
fn manifest_typ_on_catalog_root_is_typ_mismatch() {
    assert_eq!(
        jws_err(check_catalog(&manifest_header(""))),
        DetachedJwsError::TypMismatch
    );
}

#[test]
fn catalog_typ_on_manifest_is_typ_mismatch() {
    let h = format!(r#"{{"alg":"ES256","typ":"{TYP_CATALOG}","kid":"{KID}"}}"#);
    assert_eq!(jws_err(check_manifest(&h)), DetachedJwsError::TypMismatch);
    assert_eq!(check_catalog(&h).as_deref(), Ok(KID));
}

#[test]
fn typ_is_compared_case_sensitively() {
    let h = format!(r#"{{"alg":"ES256","typ":"AI-CATALOG+JWS","kid":"{KID}"}}"#);
    assert_eq!(jws_err(check_catalog(&h)), DetachedJwsError::TypMismatch);
}

#[test]
fn alg_none_is_refused() {
    let h = format!(r#"{{"alg":"none","typ":"{TYP_TRUST_MANIFEST}","kid":"{KID}"}}"#);
    assert_eq!(jws_err(check_manifest(&h)), DetachedJwsError::AlgNone);
}

#[test]
fn alg_outside_allowlist_is_refused() {
    let h = format!(r#"{{"alg":"ES384","typ":"{TYP_TRUST_MANIFEST}","kid":"{KID}"}}"#);
    assert_eq!(jws_err(check_manifest(&h)), DetachedJwsError::AlgNotAllowed);
}

#[test]
fn b64_is_refused_whatever_its_value() {
    for v in ["false", "true"] {
        let h = manifest_header(&format!(r#","b64":{v}"#));
        assert_eq!(jws_err(check_manifest(&h)), DetachedJwsError::B64Present);
    }
}

#[test]
fn any_crit_is_refused() {
    for v in [r#"["exp"]"#, "[]", r#""x""#] {
        let h = manifest_header(&format!(r#","crit":{v}"#));
        assert_eq!(jws_err(check_manifest(&h)), DetachedJwsError::CritPresent);
    }
}

fn forbidden(name: &str) {
    let h = manifest_header(&format!(r#","{name}":"x""#));
    assert_eq!(
        jws_err(check_manifest(&h)),
        DetachedJwsError::ForbiddenParam(name.to_string())
    );
}

#[test]
fn jku_is_forbidden() {
    forbidden("jku");
}

#[test]
fn jwk_is_forbidden() {
    forbidden("jwk");
}

#[test]
fn x5u_is_forbidden() {
    forbidden("x5u");
}

#[test]
fn x5c_is_forbidden() {
    forbidden("x5c");
}

#[test]
fn non_detached_forms_are_malformed() {
    let m = manifest();
    let head = support::b64url(manifest_header("").as_bytes());
    for jws in [
        format!("{head}.p.s"),
        format!("{head}..s.."),
        head.clone(),
        String::new(),
    ] {
        let r = inspect(
            SignedObject::Manifest(&m),
            &support::signature(SIGNER, &jws),
        );
        assert_eq!(
            r,
            Err(HeaderError::Jws(DetachedJwsError::Malformed)),
            "{jws}"
        );
    }
}

#[test]
fn padded_or_non_alphabet_header_is_malformed() {
    let m = manifest();
    let head = support::b64url(manifest_header("").as_bytes());
    for jws in [
        format!("{head}=..s"),
        format!("{}+..s", &head[1..]),
        format!("!{head}..s"),
    ] {
        let r = inspect(
            SignedObject::Manifest(&m),
            &support::signature(SIGNER, &jws),
        );
        assert_eq!(
            r,
            Err(HeaderError::Jws(DetachedJwsError::Malformed)),
            "{jws}"
        );
    }
}

#[test]
fn duplicate_header_member_is_invalid_header() {
    let h = format!(
        r#"{{"alg":"ES256","typ":"{TYP_CATALOG}","kid":"{KID}","typ":"{TYP_TRUST_MANIFEST}"}}"#
    );
    assert_eq!(jws_err(check_manifest(&h)), DetachedJwsError::InvalidHeader);
}

#[test]
fn non_object_header_is_invalid_header() {
    for h in ["[]", r#""ES256""#, "null", "1"] {
        assert_eq!(
            jws_err(check_manifest(h)),
            DetachedJwsError::InvalidHeader,
            "{h}"
        );
    }
}

#[test]
fn missing_kid_is_kid_mismatch() {
    let h = format!(r#"{{"alg":"ES256","typ":"{TYP_TRUST_MANIFEST}"}}"#);
    assert_eq!(check_manifest(&h), Err(HeaderError::KidMismatch));
}

#[test]
fn non_string_kid_is_kid_mismatch() {
    for v in ["1", "null", r#"{"a":1}"#, r#"["did:web:acme.com#k"]"#] {
        let h = format!(r#"{{"alg":"ES256","typ":"{TYP_TRUST_MANIFEST}","kid":{v}}}"#);
        assert_eq!(check_manifest(&h), Err(HeaderError::KidMismatch), "{v}");
    }
}

#[test]
fn kid_of_another_did_is_kid_mismatch() {
    assert_eq!(
        kid_check("did:web:other.example#k"),
        Err(HeaderError::KidMismatch)
    );
}

#[test]
fn kid_with_empty_fragment_is_kid_mismatch() {
    assert_eq!(
        kid_check("did:web:acme.com#"),
        Err(HeaderError::KidMismatch)
    );
    assert_eq!(kid_check("did:web:acme.com"), Err(HeaderError::KidMismatch));
}

#[test]
fn kid_with_path_is_kid_mismatch() {
    assert_eq!(
        kid_check("did:web:acme.com/p#k"),
        Err(HeaderError::KidMismatch)
    );
    assert_eq!(
        kid_check("did:web:acme.com:p#k"),
        Err(HeaderError::KidMismatch)
    );
}

#[test]
fn kid_fragment_follows_rfc_3986() {
    for f in ["k/1?x", "%41", "k", "a:b@c"] {
        let kid = format!("{SIGNER}#{f}");
        assert_eq!(kid_check(&kid).as_deref(), Ok(kid.as_str()), "{kid}");
    }
    for f in ["k#2", "k k", "%zz", "%4"] {
        assert_eq!(
            kid_check(&format!("{SIGNER}#{f}")),
            Err(HeaderError::KidMismatch),
            "{f}"
        );
    }
    let signer = DidWebRoot::parse(SIGNER).unwrap();
    assert!(kid_matches_signer("did:web:acme.com#k/1?x", &signer));
    assert!(!kid_matches_signer("did:web:acme.com#k#2", &signer));
}

#[test]
fn inspect_returns_a_checked_kid() {
    let kid = check_manifest(&manifest_header("")).unwrap();
    assert_eq!(kid, KID);
    let m = manifest();
    let k = inspect(
        SignedObject::Manifest(&m),
        &support::signature(SIGNER, &support::jws_with(&manifest_header(""))),
    )
    .unwrap();
    assert_eq!((k.as_str(), k.fragment()), (KID, "k"));
}

#[test]
fn unparsable_signer_is_kid_mismatch() {
    let m = manifest();
    let jws = support::jws_with(&manifest_header(""));
    let r = inspect(
        SignedObject::Manifest(&m),
        &support::signature("did:web:Acme.com", &jws),
    );
    assert_eq!(r, Err(HeaderError::KidMismatch));
}

#[test]
fn spec_example_jws_values_pass_inspect() {
    let m = manifest();
    for (signer, jws, kid) in [
        (
            "did:web:acme.com",
            "eyJhbGciOiJFUzI1NiIsImtpZCI6ImRpZDp3ZWI6YWNtZS5jb20jcmVsZWFzZS1zaWduaW5nLWtleSIsInR5cCI6ImFpLWNhdGFsb2ctdHJ1c3QtbWFuaWZlc3QrandzIn0..detached-jws-signature",
            "did:web:acme.com#release-signing-key",
        ),
        (
            "did:web:acme-corp.com",
            "eyJhbGciOiJFUzI1NiIsImtpZCI6ImRpZDp3ZWI6YWNtZS1jb3JwLmNvbSNyZWxlYXNlLXNpZ25pbmcta2V5IiwidHlwIjoiYWktY2F0YWxvZy10cnVzdC1tYW5pZmVzdCtqd3MifQ..detached-jws-signature",
            "did:web:acme-corp.com#release-signing-key",
        ),
    ] {
        let k = inspect(SignedObject::Manifest(&m), &support::signature(signer, jws)).unwrap();
        assert_eq!(k.as_str(), kid);
    }
}

#[test]
fn signer_output_passes_inspect() {
    let key = SigningKey::from_pkcs8_der(&support::fixture("keys/rfc7515-a3.pk8"));
    let params = SignerParams {
        signer: DidWebRoot::parse(SIGNER).unwrap(),
        key_fragment: "key-1".to_string(),
        issued_at: "2026-09-12T10:00:00Z".to_string(),
        expires_at: None,
    };
    let text = support::fixture("signed_payload/catalog-snapshot.input.json");
    let catalog = turul_ai_catalog::parse::parse(&text).unwrap();
    let entry = &catalog.entries[0];
    let manifest = entry.trust_manifests.clone().unwrap().remove(0);
    let signed_manifest = sign_manifest(entry, manifest, &params, &key).unwrap();
    let signed_catalog = sign_catalog(catalog, &params, &key).unwrap();
    for (object, signature) in [
        (
            SignedObject::Manifest(&signed_manifest),
            signed_manifest.signature.as_ref().unwrap(),
        ),
        (
            SignedObject::Catalog(&signed_catalog),
            signed_catalog.signature.as_ref().unwrap(),
        ),
    ] {
        assert_eq!(object.signature(), Some(signature));
        assert_eq!(
            inspect(object, signature).unwrap().as_str(),
            "did:web:acme.com#key-1"
        );
    }
}

#[test]
fn header_defects_are_not_document_diagnostics() {
    let doc = |jws: &str| {
        format!(
            r#"{{"specVersion":"1.0","signature":{{"signer":"{SIGNER}","profile":"did-web-v1","issuedAt":"2026-09-12T10:00:00Z","jws":"{jws}"}},"entries":[]}}"#
        )
    };
    let good = support::jws_with(&format!(
        r#"{{"alg":"ES256","typ":"{TYP_CATALOG}","kid":"{KID}"}}"#
    ));
    let baseline = validate(&support::catalog(&doc(&good)));
    assert!(matches!(baseline.conformance(), Conformance::Met(_)));
    for header in [
        r#"{"typ":"ai-catalog+jws","kid":"did:web:acme.com#k"}"#.to_string(),
        manifest_header(""),
        r#"{"alg":"none","typ":"ai-catalog+jws","kid":"did:web:acme.com#k"}"#.to_string(),
        r#"{"alg":"ES256","typ":"ai-catalog+jws","kid":"did:web:acme.com#k","b64":false}"#
            .to_string(),
        r#"{"alg":"ES256","typ":"ai-catalog+jws","kid":"did:web:acme.com#k","crit":["x"]}"#
            .to_string(),
        r#"{"alg":"ES256","typ":"ai-catalog+jws","kid":"did:web:acme.com#k","jku":"x"}"#
            .to_string(),
        r#"{"alg":"ES256","typ":"ai-catalog+jws","kid":"did:web:other.example#k"}"#.to_string(),
    ] {
        let report = validate(&support::catalog(&doc(&support::jws_with(&header))));
        assert_eq!(report, baseline, "{header}");
    }
}
