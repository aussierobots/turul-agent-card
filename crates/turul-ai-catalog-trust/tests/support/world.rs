//! Signed catalogs, DID documents and resolved trees for acceptance tests. Every signature is
//! made with the RFC 7515 A.3 key, which every DID document published here holds.
#![allow(dead_code)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use turul_ai_catalog::model::{AiCatalog, CatalogEntry, TrustManifest};
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_client::{FetchPolicy, Fetched, ResolvedCatalog, Source, load};
use turul_ai_catalog_trust::{
    DidDocument, DidDocuments, DidWebRoot, SignerParams, SigningKey, TrustPolicy, accept_document,
    compute, sign_catalog, sign_manifest,
};

/// `issuedAt` of every signature unless a test says otherwise.
pub const ISSUED: &str = "2026-09-12T10:00:00Z";
const ISSUED_UNIX: u64 = 1_789_207_200;
pub const SIGNER: &str = "did:web:example.com";
pub const ID: &str = "urn:air:example.com:fixture:a";

/// The instant `offset` seconds after [`ISSUED`].
pub fn at(offset: i64) -> SystemTime {
    let base = UNIX_EPOCH + Duration::from_secs(ISSUED_UNIX);
    if offset >= 0 {
        base + Duration::from_secs(offset as u64)
    } else {
        base - Duration::from_secs(offset.unsigned_abs())
    }
}

pub fn key() -> SigningKey {
    SigningKey::from_pkcs8_der(&super::fixture("keys/rfc7515-a3.pk8"))
}

pub fn root(signer: &str) -> DidWebRoot {
    DidWebRoot::parse(signer).unwrap_or_else(|| panic!("{signer}"))
}

/// A DID document for `signer` whose `kid` fragment is `fragment`.
pub fn doc_with_fragment(signer: &str, fragment: &str) -> DidDocument {
    let jwk = String::from_utf8(super::fixture("keys/rfc7515-a3.public.jwk.json")).unwrap();
    let body = format!(
        r#"{{"id":"{signer}","verificationMethod":[{{"id":"{signer}#{fragment}","type":"JsonWebKey2020","controller":"{signer}","publicKeyJwk":{jwk}}}],"assertionMethod":["{signer}#{fragment}"]}}"#
    );
    doc_from(&body, signer)
}

pub fn doc_from(body: &str, signer: &str) -> DidDocument {
    let fetched = Fetched::new(
        200,
        Some("application/did+json".to_string()),
        body.as_bytes().to_vec(),
    );
    accept_document(&fetched, &root(signer)).unwrap()
}

pub fn doc(signer: &str) -> DidDocument {
    doc_with_fragment(signer, "key-1")
}

pub fn docs(signers: &[&str]) -> DidDocuments {
    signers
        .iter()
        .map(|s| (root(s), Ok(doc(s))))
        .collect::<DidDocuments>()
}

pub fn loopback() -> FetchPolicy {
    let mut p = FetchPolicy::default();
    p.permit_loopback_http = true;
    p
}

pub fn policy() -> TrustPolicy {
    let mut p = TrustPolicy::default();
    p.fetch = loopback();
    p
}

pub fn params(signer: &str, issued: &str, expires: Option<&str>) -> SignerParams {
    SignerParams {
        signer: root(signer),
        key_fragment: "key-1".to_string(),
        issued_at: issued.to_string(),
        expires_at: expires.map(str::to_string),
    }
}

fn parse_entries(entries: &str) -> AiCatalog {
    let doc = format!(r#"{{"specVersion":"1.0","entries":[{entries}]}}"#);
    parse(doc.as_bytes()).unwrap_or_else(|e| panic!("{e:?} for {doc}"))
}

/// An inline-`data` entry whose `digest` is the digest of its JCS `data`.
pub fn data_entry(identifier: &str, data: &str) -> CatalogEntry {
    let mut entry = parse_entries(&format!(
        r#"{{"identifier":"{identifier}","type":"application/json","data":{data}}}"#
    ))
    .entries
    .remove(0);
    let bytes = match &entry.source {
        turul_ai_catalog::model::ArtifactSource::Data(j) => j.to_canonical_vec().unwrap(),
        _ => unreachable!(),
    };
    entry.digest = Some(compute("sha256", &bytes).unwrap());
    entry
}

/// A `url` entry whose `digest` is the digest of `body`.
pub fn url_entry(identifier: &str, url: &str, body: &[u8]) -> CatalogEntry {
    let mut entry = parse_entries(&format!(
        r#"{{"identifier":"{identifier}","type":"application/json","url":"{url}"}}"#
    ))
    .entries
    .remove(0);
    entry.digest = Some(compute("sha256", body).unwrap());
    entry
}

pub fn entry() -> CatalogEntry {
    data_entry(ID, r#"{"a":1}"#)
}

/// `entry` signed by `contributor` with the default window.
pub fn signed(entry: &CatalogEntry, contributor: &str) -> TrustManifest {
    signed_at(entry, contributor, ISSUED, None)
}

pub fn signed_at(
    entry: &CatalogEntry,
    contributor: &str,
    issued: &str,
    expires: Option<&str>,
) -> TrustManifest {
    sign_manifest(
        entry,
        TrustManifest::new(contributor.to_string()),
        &params(contributor, issued, expires),
        &key(),
    )
    .unwrap()
}

pub fn with_manifests(mut entry: CatalogEntry, manifests: Vec<TrustManifest>) -> CatalogEntry {
    entry.trust_manifests = Some(manifests);
    entry
}

pub fn catalog(entries: Vec<CatalogEntry>) -> AiCatalog {
    AiCatalog::new(entries)
}

pub fn sign_root(catalog: AiCatalog, signer: &str) -> AiCatalog {
    sign_catalog(catalog, &params(signer, ISSUED, None), &key()).unwrap()
}

/// `catalog` as the tree `load` builds from a file holding it.
pub fn tree(catalog: &AiCatalog) -> ResolvedCatalog {
    static N: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "trust-fixture-{}-{}.json",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, catalog.to_json().to_canonical_vec().unwrap()).unwrap();
    let tree = load(&Source::File(path.clone()), &loopback()).unwrap();
    let _ = std::fs::remove_file(path);
    tree
}
