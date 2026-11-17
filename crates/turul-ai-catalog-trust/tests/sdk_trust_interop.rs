//! Interop with `ai-catalog-trust` (payload and digest half): our payloads and digest verdicts
//! against its, over the original bytes of the format crate's corpus.

mod support;

use std::fs;
use std::path::PathBuf;

use ai_catalog_trust::{
    canonicalize_for_signature, canonicalize_value_for_signature, parse_json, verify_digest,
};
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::{AiCatalog, ArtifactSource, TrustManifest};
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_client::FetchPolicy;
use turul_ai_catalog_trust::{
    IntegrityError, PayloadError, catalog_payload, compute, manifest_payload, verify_artifact,
};

fn format_fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../turul-ai-catalog/tests/fixtures")
}

/// Every catalog document of the corpus as (key, bytes).
fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let dirs = [
        (format_fixtures().join("upstream"), "upstream"),
        (format_fixtures().join("interop"), "interop"),
        (format_fixtures().join("spec/examples"), "spec/examples"),
        (
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signed_payload"),
            "signed_payload",
        ),
    ];
    for (dir, label) in dirs {
        let mut names: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|f| f.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.ends_with(".json"))
            .collect();
        names.sort();
        for n in names {
            out.push((format!("{label}/{n}"), fs::read(dir.join(&n)).unwrap()));
        }
    }
    out
}

/// Every manifest of `catalog` with its JSON Pointer, nested inline catalogs included.
fn manifests(catalog: &AiCatalog, prefix: &str, out: &mut Vec<(String, TrustManifest)>) {
    for (i, entry) in catalog.entries.iter().enumerate() {
        let at = format!("{prefix}/entries/{i}");
        for (j, m) in entry.trust_manifests.iter().flatten().enumerate() {
            out.push((format!("{at}/trustManifests/{j}"), m.clone()));
        }
        if let ArtifactSource::Data(data) = &entry.source {
            // A nested inline catalog is itself a catalog document; anything else does not parse.
            if let Some(nested) = data.to_canonical_vec().ok().and_then(|b| parse(&b).ok()) {
                manifests(&nested, &format!("{at}/data"), out);
            }
        }
    }
}

// Payload law (documents ours refuses as non-I-JSON are excluded and counted).
#[test]
fn payloads_agree_for_every_i_json_corpus_object() {
    let (mut catalogs, mut objects, mut excluded) = (0, 0, Vec::new());
    for (key, bytes) in corpus() {
        let (Ok(ours), Ok(value)) = (parse(&bytes), parse_json(&bytes)) else {
            continue;
        };
        match catalog_payload(&ours) {
            Ok(p) => {
                assert_eq!(
                    p,
                    canonicalize_for_signature(&bytes).unwrap().into_bytes(),
                    "{key}"
                );
                catalogs += 1;
            }
            Err(PayloadError::NotIJson { .. }) => excluded.push(key.clone()),
            Err(e) => panic!("{key}: {e}"),
        }
        let mut found = Vec::new();
        manifests(&ours, "", &mut found);
        for (pointer, m) in found {
            match manifest_payload(&m) {
                Ok(p) => {
                    let v = value
                        .pointer(&pointer)
                        .unwrap_or_else(|| panic!("{key}{pointer}"));
                    assert_eq!(
                        p,
                        canonicalize_value_for_signature(v).unwrap().into_bytes(),
                        "{key}{pointer}"
                    );
                    objects += 1;
                }
                Err(PayloadError::NotIJson { .. }) => excluded.push(format!("{key}{pointer}")),
                Err(e) => panic!("{key}{pointer}: {e}"),
            }
        }
    }
    // The corpus includes unknown members on every kind and a nested signed manifest.
    assert!(
        catalogs >= 10 && objects >= 3,
        "{catalogs} catalogs, {objects} manifests"
    );
    eprintln!("excluded (not a finite double): {excluded:?}");
}

// Digest verdicts for `data` entries.
#[test]
fn digest_verdicts_agree_for_every_data_entry() {
    let jcs = br#"{"a":2,"b":1}"#;
    let good = compute("sha256", jcs).unwrap();
    let hex = good.split_once(':').unwrap().1.to_string();
    let cases = [
        good.clone(),
        compute("sha384", jcs).unwrap(),
        compute("sha512", jcs).unwrap(),
        format!("sha256:{}", "0".repeat(64)),
        format!("SHA256:{hex}"),
        format!("sha3-256:{}", "a".repeat(64)),
        "sha256:zz".to_string(),
        format!("md5:{}", "a".repeat(32)),
    ];
    for digest in cases {
        let doc = format!(
            r#"{{"specVersion":"1.0","entries":[{{"identifier":"urn:air:example.com:fixture:d","type":"application/json","data":{{"b":1,"a":2}},"digest":"{digest}"}}]}}"#
        );
        let entry = parse(doc.as_bytes()).unwrap().entries.remove(0);
        let ours = verify_artifact(&entry, None, &FetchPolicy::default());
        let theirs = verify_digest(&digest, jcs);
        match (&ours, &theirs) {
            (Ok(()), Ok(true)) => {}
            (Err(IntegrityError::Mismatch { .. }), Ok(false)) => {}
            (Err(IntegrityError::Unsupported(_) | IntegrityError::DigestInvalid), Err(_)) => {}
            _ => panic!("{digest}: ours {ours:?}, theirs {theirs:?}"),
        }
    }
    // Corpus entries with inline data and a digest agree too.
    for (key, bytes) in corpus() {
        let Ok(c) = parse(&bytes) else { continue };
        for entry in c.entries.iter().filter(|e| e.digest.is_some()) {
            let ArtifactSource::Data(data @ Json::Object(_)) = &entry.source else {
                continue;
            };
            let Ok(b) = data.to_canonical_vec() else {
                continue;
            };
            let ours = verify_artifact(entry, None, &FetchPolicy::default());
            let theirs = verify_digest(entry.digest.as_deref().unwrap(), &b);
            assert_eq!(ours.is_ok(), matches!(theirs, Ok(true)), "{key}");
        }
    }
}

// `ai-catalog-trust` verifies what we sign, against the published A.3 public key.
#[test]
fn ai_catalog_trust_verifies_our_signature() {
    use ai_catalog_trust::{P256Key, verify_es256};
    use turul_ai_catalog_trust::{
        DidWebRoot, SignerParams, SigningKey, sign_catalog, sign_manifest,
    };

    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let key = SigningKey::from_pkcs8_der(&fs::read(dir.join("keys/rfc7515-a3.pk8")).unwrap());
    let jwk = parse_json(fs::read(dir.join("keys/rfc7515-a3.public.jwk.json")).unwrap()).unwrap();
    let public = P256Key::from_jwk(&jwk).unwrap();
    let params = SignerParams {
        signer: DidWebRoot::parse("did:web:acme.com").unwrap(),
        key_fragment: "key-1".to_string(),
        issued_at: "2026-09-12T10:00:00Z".to_string(),
        expires_at: None,
    };

    let text = fs::read(dir.join("signed_payload/catalog-snapshot.input.json")).unwrap();
    let catalog = parse(&text).unwrap();
    let signed = sign_catalog(catalog.clone(), &params, &key).unwrap();
    let payload = catalog_payload(&signed).unwrap();
    let jws = &signed.signature.as_ref().unwrap().jws;
    verify_es256(jws, &payload, &public).unwrap();
    assert!(verify_es256(jws, b"{}", &public).is_err());

    let entry = &catalog.entries[0];
    let manifest = entry.trust_manifests.clone().unwrap().remove(0);
    let signed = sign_manifest(entry, manifest, &params, &key).unwrap();
    let payload = manifest_payload(&signed).unwrap();
    verify_es256(&signed.signature.as_ref().unwrap().jws, &payload, &public).unwrap();
}

fn validate_warning_codes(signer: &str, kid: &str) -> Vec<String> {
    let jws = format!(
        "{}..c2ln",
        support::b64url(
            format!(r#"{{"alg":"ES256","typ":"ai-catalog-trust-manifest+jws","kid":"{kid}"}}"#)
                .as_bytes()
        )
    );
    let doc = format!(
        r#"{{"specVersion":"1.0","entries":[{{{}, "trustManifests":[{{"contributor":"{signer}","signature":{{"signer":"{signer}","profile":"did-web-v1","issuedAt":"2026-09-12T10:00:00Z","jws":"{jws}"}}}}]}}]}}"#,
        support::ENTRY
    );
    let catalog = ai_catalog::parse_str(&doc).unwrap();
    ai_catalog_validate::validate(&catalog)
        .warnings
        .iter()
        .map(|d| d.code.to_string())
        .collect()
}

// ai-catalog-validate accepts IP spellings beyond dotted quads as `did:web` domains.
#[test]
fn did_web_ip_spellings_are_refused_here_only() {
    use turul_ai_catalog_trust::DidWebRoot;
    let control = validate_warning_codes("did:web:Example.com", "did:web:Example.com#k");
    assert!(!control.is_empty());
    for host in ["2130706433", "127.1", "0x7f.0.0.1"] {
        let signer = format!("did:web:{host}");
        let warnings = validate_warning_codes(&signer, &format!("{signer}#k"));
        assert!(warnings.is_empty(), "{host}: {warnings:?}");
        assert_eq!(DidWebRoot::parse(&signer), None, "{host}");
    }
}

// ai-catalog-validate refuses a `kid` fragment with `/` or `?`; RFC 3986 allows both.
#[test]
fn kid_fragment_with_slash_is_accepted_here_only() {
    use turul_ai_catalog_trust::{DidWebRoot, kid_matches_signer};
    let warnings = validate_warning_codes("did:web:acme.com", "did:web:acme.com#k/1?x");
    assert!(!warnings.is_empty());
    let signer = DidWebRoot::parse("did:web:acme.com").unwrap();
    assert!(kid_matches_signer("did:web:acme.com#k/1?x", &signer));
}
