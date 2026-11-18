//! The DID document a publisher serves for its signing key.

mod support;

use support::world::{self, ISSUED, SIGNER, at, entry, params, root};
use turul_ai_catalog::json::{Json, read};
use turul_ai_catalog_client::Fetched;
use turul_ai_catalog_trust::{
    DidDocuments, Endorsement, Kid, SignError, SigningKey, accept_document, did_document,
    endorse_entry, select_key, verify_resolved,
};

fn served(document: &Json) -> Fetched {
    Fetched::new(
        200,
        Some("application/did+json".to_string()),
        document.to_canonical_vec().unwrap(),
    )
}

#[test]
fn document_is_accepted_and_selects_the_key() {
    let signer = root(SIGNER);
    let jwk = SigningKey::generate().public_jwk().unwrap();
    let document = did_document(&signer, "key-1", &jwk).unwrap();

    let accepted = accept_document(&served(&document), &signer).expect("accepted");
    let kid = Kid::parse(&format!("{SIGNER}#key-1"), &signer).unwrap();
    select_key(&accepted, &kid).expect("the key is selected");

    let expected = read(
        format!(
            r#"{{"@context":["https://www.w3.org/ns/did/v1","https://w3id.org/security/suites/jws-2020/v1"],"id":"{SIGNER}","verificationMethod":[{{"id":"{SIGNER}#key-1","type":"JsonWebKey2020","controller":"{SIGNER}","publicKeyJwk":{}}}],"assertionMethod":["{SIGNER}#key-1"]}}"#,
            String::from_utf8(jwk.to_canonical_vec().unwrap()).unwrap()
        )
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(document, expected);
}

#[test]
fn generated_key_endorses_an_entry_end_to_end() {
    let key = SigningKey::generate();
    let signer = root(SIGNER);
    let document = did_document(&signer, "key-1", &key.public_jwk().unwrap()).unwrap();
    let accepted = accept_document(&served(&document), &signer).unwrap();

    let endorsed = endorse_entry(entry(), &params(SIGNER, ISSUED, None), &key).unwrap();
    let tree = world::tree(&world::catalog(vec![endorsed]));
    let mut docs = DidDocuments::new();
    docs.insert(signer, Ok(accepted));
    let report = verify_resolved(&tree, &docs, &world::policy(), at(0));

    let entry = &report.catalogs[0].entries[0];
    assert!(matches!(entry.integrity, Some(Ok(()))));
    assert_eq!(entry.manifests[0].endorsement(), Endorsement::Publisher);
}

#[test]
fn bad_fragment_or_jwk_is_refused() {
    let signer = root(SIGNER);
    let jwk = SigningKey::generate().public_jwk().unwrap();
    for fragment in ["", "a b", "a#b"] {
        assert_eq!(
            did_document(&signer, fragment, &jwk),
            Err(SignError::Kid),
            "{fragment:?}"
        );
    }
    for bad in [
        r#"{"kty":"oct","k":"AAAA"}"#,
        r#"{"kty":"EC","crv":"P-256"}"#,
        r#"[]"#,
    ] {
        assert_eq!(
            did_document(&signer, "key-1", &read(bad.as_bytes()).unwrap()),
            Err(SignError::Key),
            "{bad}"
        );
    }
}
