//! Verification-key selection from an accepted DID document.

use turul_ai_catalog_client::Fetched;
use turul_ai_catalog_trust::{
    DidDocument, DidWebRoot, KeySelectionError, Kid, accept_document, select_key,
};
use turul_jwt_validator::detached::DetachedJwsError;

const SIGNER: &str = "did:web:example.com";
const X: &str = "f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU";
const Y: &str = "x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0";

fn jwk() -> String {
    format!(r#"{{"kty":"EC","crv":"P-256","x":"{X}","y":"{Y}"}}"#)
}

fn method(id: &str, controller: &str, jwk: &str) -> String {
    format!(
        r#"{{"id":"{id}","type":"JsonWebKey2020","controller":"{controller}","publicKeyJwk":{jwk}}}"#
    )
}

fn doc(body_members: &str) -> DidDocument {
    let body = format!(r#"{{"id":"{SIGNER}",{body_members}}}"#);
    let f = Fetched::new(200, Some("application/did+json".into()), body.into_bytes());
    accept_document(&f, &DidWebRoot::parse(SIGNER).unwrap()).unwrap_or_else(|e| panic!("{e:?}"))
}

fn kid(fragment: &str) -> Kid {
    Kid::parse(
        &format!("{SIGNER}#{fragment}"),
        &DidWebRoot::parse(SIGNER).unwrap(),
    )
    .unwrap()
}

fn listed(m: &str, assertion: &str) -> DidDocument {
    doc(&format!(
        r#""verificationMethod":[{m}],"assertionMethod":{assertion}"#
    ))
}

fn k_method(jwk: &str) -> String {
    method("did:web:example.com#k", SIGNER, jwk)
}

fn err(d: &DidDocument) -> KeySelectionError {
    select_key(d, &kid("k")).unwrap_err()
}

fn jwk_err(jwk: &str) -> KeySelectionError {
    err(&listed(&k_method(jwk), r##"["#k"]"##))
}

#[test]
fn referenced_assertion_method_key_is_selected() {
    let d = listed(&k_method(&jwk()), r##"["did:web:example.com#k"]"##);
    assert!(select_key(&d, &kid("k")).is_ok());
}

#[test]
fn embedded_assertion_method_key_is_selected() {
    let d = doc(&format!(r#""assertionMethod":[{}]"#, k_method(&jwk())));
    assert!(select_key(&d, &kid("k")).is_ok());
}

#[test]
fn fragment_reference_resolves_against_the_document_id() {
    let m = method("#k", SIGNER, &jwk());
    let d = listed(&m, r##"["#k"]"##);
    assert!(select_key(&d, &kid("k")).is_ok());
}

#[test]
fn non_fragment_relative_reference_is_compared_verbatim() {
    for reference in ["key-1", "./web:example.com#k"] {
        let m = method("did:web:example.com#key-1", SIGNER, &jwk());
        let d = listed(&m, &format!(r#"["{reference}"]"#));
        assert_eq!(
            select_key(&d, &kid("key-1")).unwrap_err(),
            KeySelectionError::NotAuthorized,
            "{reference}: reference matches nothing, method is only listed"
        );
    }
    let m = method("key-1", SIGNER, &jwk());
    let d = listed(&m, r#"["key-1"]"#);
    assert_eq!(
        select_key(&d, &kid("key-1")).unwrap_err(),
        KeySelectionError::NotFound
    );
}

#[test]
fn two_methods_with_the_kid_are_ambiguous() {
    let m = format!("{},{}", k_method(&jwk()), k_method(&jwk()));
    assert_eq!(
        err(&listed(&m, r##"["#k"]"##)),
        KeySelectionError::Ambiguous
    );
}

#[test]
fn reference_and_target_count_once() {
    let d = listed(
        &k_method(&jwk()),
        r##"["#k","did:web:example.com#k","#k"]"##,
    );
    assert!(select_key(&d, &kid("k")).is_ok());
}

#[test]
fn embedded_duplicate_of_a_listed_method_is_ambiguous() {
    let d = doc(&format!(
        r#""verificationMethod":[{m}],"assertionMethod":[{m}]"#,
        m = k_method(&jwk())
    ));
    assert_eq!(err(&d), KeySelectionError::Ambiguous);
}

#[test]
fn other_es256_keys_are_never_counted() {
    let other = method("did:web:example.com#other", SIGNER, &jwk());
    let m = format!("{other},{}", k_method(&jwk()));
    let d = listed(&m, r##"["#other","#k"]"##);
    assert!(select_key(&d, &kid("k")).is_ok());
}

#[test]
fn unknown_kid_is_not_found() {
    let d = listed(&k_method(&jwk()), r##"["#k"]"##);
    assert_eq!(
        select_key(&d, &kid("missing")).unwrap_err(),
        KeySelectionError::NotFound
    );
}

#[test]
fn authentication_only_key_is_not_authorized() {
    let d = doc(&format!(
        r##""verificationMethod":[{}],"authentication":["#k"]"##,
        k_method(&jwk())
    ));
    assert_eq!(err(&d), KeySelectionError::NotAuthorized);
    let d = doc(&format!(r#""authentication":[{}]"#, k_method(&jwk())));
    assert_eq!(err(&d), KeySelectionError::NotFound);
}

#[test]
fn verification_method_only_key_is_not_authorized() {
    let d = doc(&format!(r#""verificationMethod":[{}]"#, k_method(&jwk())));
    assert_eq!(err(&d), KeySelectionError::NotAuthorized);
}

#[test]
fn key_removed_from_assertion_method_is_not_authorized() {
    let m = k_method(&jwk());
    assert!(select_key(&listed(&m, r##"["#k"]"##), &kid("k")).is_ok());
    assert_eq!(err(&listed(&m, "[]")), KeySelectionError::NotAuthorized);
}

#[test]
fn only_an_assertion_method_key_controlled_by_the_signer_is_authorized() {
    let d = listed(
        &method("did:web:example.com#k", "did:web:other.example", &jwk()),
        r##"["#k"]"##,
    );
    assert_eq!(err(&d), KeySelectionError::ControllerMismatch);
    let d = listed(&k_method(&jwk()), "[]");
    assert_eq!(err(&d), KeySelectionError::NotAuthorized);
    let d = listed(&k_method(&jwk()), r##"["#k"]"##);
    assert!(select_key(&d, &kid("k")).is_ok());
}

#[test]
fn controller_other_than_signer_is_refused() {
    for c in ["did:web:other.example", "did:web:Example.com"] {
        let d = listed(&method("did:web:example.com#k", c, &jwk()), r##"["#k"]"##);
        assert_eq!(err(&d), KeySelectionError::ControllerMismatch, "{c}");
    }
}

#[test]
fn method_without_public_key_jwk_is_refused() {
    let m = r#"{"id":"did:web:example.com#k","type":"JsonWebKey2020","controller":"did:web:example.com"}"#;
    assert_eq!(
        err(&listed(m, r##"["#k"]"##)),
        KeySelectionError::NoPublicKeyJwk
    );
}

fn invalid(e: KeySelectionError) {
    assert!(
        matches!(e, KeySelectionError::Jwk(DetachedJwsError::InvalidJwk(_))),
        "{e:?}"
    );
}

#[test]
fn jwk_with_p384_is_refused() {
    invalid(jwk_err(&format!(
        r#"{{"kty":"EC","crv":"P-384","x":"{X}","y":"{Y}"}}"#
    )));
}

#[test]
fn jwk_off_the_curve_is_refused() {
    let bad_y = format!("{}g", &Y[..Y.len() - 1]);
    invalid(jwk_err(&format!(
        r#"{{"kty":"EC","crv":"P-256","x":"{X}","y":"{bad_y}"}}"#
    )));
}

#[test]
fn jwk_with_kty_rsa_is_refused() {
    invalid(jwk_err(&format!(
        r#"{{"kty":"RSA","crv":"P-256","x":"{X}","y":"{Y}"}}"#
    )));
}

#[test]
fn jwk_with_d_is_refused() {
    invalid(jwk_err(&format!(
        r#"{{"kty":"EC","crv":"P-256","x":"{X}","y":"{Y}","d":"{X}"}}"#
    )));
}

fn with(extra: &str) -> String {
    format!(r#"{{"kty":"EC","crv":"P-256","x":"{X}","y":"{Y}",{extra}}}"#)
}

#[test]
fn jwk_with_alg_es384_is_refused() {
    invalid(jwk_err(&with(r#""alg":"ES384""#)));
}

#[test]
fn jwk_with_use_enc_is_refused() {
    invalid(jwk_err(&with(r#""use":"enc""#)));
}

#[test]
fn jwk_with_key_ops_sign_is_refused() {
    invalid(jwk_err(&with(r#""key_ops":["sign"]"#)));
}

#[test]
fn jwk_with_es256_sig_verify_is_accepted() {
    let d = listed(
        &k_method(&with(r#""alg":"ES256","use":"sig","key_ops":["verify"]"#)),
        r##"["#k"]"##,
    );
    assert!(select_key(&d, &kid("k")).is_ok());
}

#[test]
fn jwk_is_checked_from_its_canonical_bytes() {
    let reordered = format!(
        r#"{{ "y" : "{Y}", "x":"{X}",
        "crv":"P-256", "kty":"EC" }}"#
    );
    let d = listed(&k_method(&reordered), r##"["#k"]"##);
    assert!(select_key(&d, &kid("k")).is_ok());
}

#[test]
fn jwk_kid_differing_from_the_method_id_is_ignored() {
    let d = listed(&k_method(&with(r#""kid":"something-else""#)), r##"["#k"]"##);
    assert!(select_key(&d, &kid("k")).is_ok());
    assert_eq!(
        select_key(&d, &kid("something-else")).unwrap_err(),
        KeySelectionError::NotFound
    );
}
