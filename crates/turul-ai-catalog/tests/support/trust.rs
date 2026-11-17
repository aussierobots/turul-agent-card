#![allow(dead_code)]

use crate::common::*;
use turul_ai_catalog::validate::{Report, Severity};

use turul_ai_catalog::model::TrustManifest;
use turul_ai_catalog::validate::validate_manifest;

/// The identifier of the entry built by `bound_doc`.
pub const ENTRY_ID: &str = "urn:air:example.com:fixture:a";
pub const ENTRY_TYPE: &str = "application/mcp-server-card+json";

/// A valid SHA-256 digest value.
pub fn digest() -> String {
    format!("sha256:{}", hex(64))
}

/// A manifest member that makes a manifest substantive and is otherwise inert.
pub const SUBSTANCE: &str = r#""extensions":{"https://example.com/x":true}"#;

/// A Signature object, members as given; `expires_at` is omitted when `None`.
pub fn signature(signer: &str, profile: &str, issued_at: &str, expires_at: Option<&str>) -> String {
    let expires = expires_at
        .map(|e| format!(r#","expiresAt":"{e}""#))
        .unwrap_or_default();
    format!(
        r#"{{"signer":"{signer}","profile":"{profile}","issuedAt":"{issued_at}"{expires},"jws":"a..b"}}"#
    )
}

/// A valid Signature object for `signer` with the reserved profile.
pub fn good_signature(signer: &str) -> String {
    signature(signer, "did-web-v1", "2026-09-12T10:00:00Z", None)
}

/// A Subject object; `version` is omitted when `None`.
pub fn subject(identifier: &str, ty: &str, digest: &str, version: Option<&str>) -> String {
    let v = version
        .map(|v| format!(r#","version":"{v}""#))
        .unwrap_or_default();
    format!(r#"{{"identifier":"{identifier}","type":"{ty}","digest":"{digest}"{v}}}"#)
}

/// A Subject that binds the entry built by `bound_doc` with a digest and no version.
pub fn matching_subject() -> String {
    subject(ENTRY_ID, ENTRY_TYPE, &digest(), None)
}

/// A catalog document whose one entry carries `entry_extra` (members with a leading comma)
/// and the given manifests.
pub fn bound_doc(entry_extra: &str, manifests: &[&str]) -> String {
    doc_with_entry_extra(&format!(
        r#""trustManifests":[{}]{entry_extra}"#,
        manifests.join(",")
    ))
}

/// The entry `digest` member, with a leading comma.
pub fn entry_digest() -> String {
    format!(r#","digest":"{}""#, digest())
}

/// The first manifest of the first entry of a document that parses.
pub fn manifest_of(document: &str) -> TrustManifest {
    ok(document).entries[0]
        .trust_manifests
        .as_ref()
        .expect("manifests")[0]
        .clone()
}

/// Parses `manifest` inside a wrapper document and validates it standalone.
pub fn vm(manifest: &str) -> Report {
    validate_manifest(&manifest_of(&doc_with_manifest(manifest)))
}

/// A manifest object from its contributor and further members (each with a leading comma).
pub fn manifest(contributor: &str, extra: &str) -> String {
    format!(r#"{{"contributor":"{contributor}"{extra}}}"#)
}

/// The diagnostics at or under `prefix`, with the prefix removed, as (code, severity, path).
pub fn under(r: &Report, prefix: &str) -> Vec<(String, Severity, String)> {
    found(r)
        .into_iter()
        .filter(|(_, _, p)| p == prefix || p.starts_with(&format!("{prefix}/")))
        .map(|(c, s, p)| (c, s, p[prefix.len()..].to_string()))
        .collect()
}
