//! Generating signing keys and reading their public half.

mod support;

use turul_ai_catalog::json::{Json, read};
use turul_ai_catalog_trust::{SignError, SigningKey};
use turul_jwt_validator::detached::check_public_jwk;

fn members(json: Json) -> Vec<(String, Json)> {
    match json {
        Json::Object(m) => m.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        other => panic!("not an object: {other:?}"),
    }
}

#[test]
fn generated_keys_are_distinct_p256_keys() {
    let (a, b) = (SigningKey::generate(), SigningKey::generate());
    let (ja, jb) = (a.public_jwk().unwrap(), b.public_jwk().unwrap());
    assert_ne!(ja, jb);
    for jwk in [&ja, &jb] {
        check_public_jwk(&jwk.to_canonical_vec().unwrap()).expect("a P-256 public key");
    }
}

#[test]
fn public_jwk_has_no_private_member() {
    let jwk = members(SigningKey::generate().public_jwk().unwrap());
    let names: Vec<&str> = jwk.iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(names, ["crv", "kty", "x", "y"]);
    assert!(jwk.contains(&("kty".into(), Json::String("EC".into()))));
    assert!(jwk.contains(&("crv".into(), Json::String("P-256".into()))));

    let fixture = SigningKey::from_pkcs8_der(&support::fixture("keys/rfc7515-a3.pk8"));
    let expected = read(&support::fixture("keys/rfc7515-a3.public.jwk.json")).unwrap();
    assert_eq!(fixture.public_jwk().unwrap(), expected);

    let refused = SigningKey::from_pkcs8_der(b"not a key").public_jwk();
    assert_eq!(refused, Err(SignError::Key));
}

#[test]
fn debug_never_shows_the_key() {
    let der = support::fixture("keys/rfc7515-a3.pk8");
    let key = SigningKey::from_pkcs8_der(&der);
    assert_eq!(format!("{key:?}"), "SigningKey(..)");
    assert_eq!(format!("{key:#?}"), "SigningKey(..)");
}
