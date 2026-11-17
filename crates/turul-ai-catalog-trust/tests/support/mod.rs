#![allow(dead_code)]

pub mod world;

use turul_ai_catalog::model::{AiCatalog, TrustManifest};
use turul_ai_catalog::parse::parse;

pub const ENTRY: &str = r#""identifier":"urn:air:example.com:fixture:a","type":"application/mcp-server-card+json","url":"https://example.com/a.json""#;

/// A catalog holding one entry that carries `manifest` (a JSON object).
pub fn catalog_with(manifest: &str) -> String {
    format!(r#"{{"specVersion":"1.0","entries":[{{{ENTRY},"trustManifests":[{manifest}]}}]}}"#)
}

pub fn catalog(json: &str) -> AiCatalog {
    parse(json.as_bytes()).unwrap_or_else(|e| panic!("{e:?} for {json}"))
}

/// The one manifest of `catalog_with(manifest)`.
pub fn manifest(json: &str) -> TrustManifest {
    catalog(&catalog_with(json)).entries[0]
        .trust_manifests
        .clone()
        .unwrap()
        .remove(0)
}

pub fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!(
        "{}/tests/fixtures/{name}",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap_or_else(|e| panic!("{name}: {e}"))
}

pub fn spec_golden() -> Vec<u8> {
    std::fs::read(format!(
        "{}/../turul-ai-catalog/tests/fixtures/canonical/spec-signed-payload.jcs",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap()
}

/// Unpadded base64url, for building protected headers by hand.
pub fn b64url(bytes: &[u8]) -> String {
    const A: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(A[(n >> (18 - 6 * i) & 63) as usize] as char);
        }
    }
    out
}

/// A detached JWS with `header_json` as protected header and a placeholder signature segment.
pub fn jws_with(header_json: &str) -> String {
    format!("{}..c2ln", b64url(header_json.as_bytes()))
}

pub fn signature(signer: &str, jws: &str) -> turul_ai_catalog::model::Signature {
    turul_ai_catalog::model::Signature::new(
        signer.to_string(),
        "did-web-v1".to_string(),
        "2026-09-12T10:00:00Z".to_string(),
        jws.to_string(),
    )
}
