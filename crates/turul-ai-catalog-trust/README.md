# turul-ai-catalog-trust

Trust for [AI Catalog](https://ai-catalog.io) documents: sign a catalog or a trust manifest,
accept a signature against the signer's `did:web` DID document, check artifact digests, and build
a per-signature trust report for a resolved catalog tree. Built on
[`turul-ai-catalog`](../turul-ai-catalog) and [`turul-ai-catalog-client`](../turul-ai-catalog-client).

Part of [turul-agent-card](https://github.com/aussierobots/turul-agent-card).

The crate implements the `did-web-v1` signature profile: ES256 detached JWS over the RFC 8785
canonical form of the signed object, with the signer's key taken from its root `did:web`
DID document. Every outcome is reported per signature; there is no catalog-wide verdict.

## Quick start

```sh
cargo add turul-ai-catalog-trust
```

Sign a catalog, then accept the signature against the signer's DID document. The DID document
is built in memory here; `resolve` fetches the real one over HTTPS.

```rust
use std::time::{Duration, UNIX_EPOCH};

use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
use turul_ai_catalog::model::{AiCatalog, ArtifactSource, CatalogEntry};
use turul_ai_catalog_client::Fetched;
use turul_ai_catalog_trust::{
    DidWebRoot, SignedObject, SignerParams, SigningKey, TrustPolicy, accept_document, sign_catalog,
    verify_signature,
};

# // The RFC 7515 appendix A.3 test key; never use it to sign anything real.
# const KEY_DER: [u8; 138] = [
# 0x30, 0x81, 0x87, 0x02, 0x01, 0x00, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86,
# 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d,
# 0x03, 0x01, 0x07, 0x04, 0x6d, 0x30, 0x6b, 0x02, 0x01, 0x01, 0x04, 0x20,
# 0x8e, 0x9b, 0x10, 0x9e, 0x71, 0x90, 0x98, 0xbf, 0x98, 0x04, 0x87, 0xdf,
# 0x1f, 0x5d, 0x77, 0xe9, 0xcb, 0x29, 0x60, 0x6e, 0xbe, 0xd2, 0x26, 0x3b,
# 0x5f, 0x57, 0xc2, 0x13, 0xdf, 0x84, 0xf4, 0xb2, 0xa1, 0x44, 0x03, 0x42,
# 0x00, 0x04, 0x7f, 0xcd, 0xce, 0x27, 0x70, 0xf6, 0xc4, 0x5d, 0x41, 0x83,
# 0xcb, 0xee, 0x6f, 0xdb, 0x4b, 0x7b, 0x58, 0x07, 0x33, 0x35, 0x7b, 0xe9,
# 0xef, 0x13, 0xba, 0xcf, 0x6e, 0x3c, 0x7b, 0xd1, 0x54, 0x45, 0xc7, 0xf1,
# 0x44, 0xcd, 0x1b, 0xbd, 0x9b, 0x7e, 0x87, 0x2c, 0xdf, 0xed, 0xb9, 0xee,
# 0xb9, 0xf4, 0xb3, 0x69, 0x5d, 0x6e, 0xa9, 0x0b, 0x24, 0xad, 0x8a, 0x46,
# 0x23, 0x28, 0x85, 0x88, 0xe5, 0xad,
# ];
# const PUBLIC_JWK: &str = r#"{"kty":"EC","crv":"P-256","x":"f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU","y":"x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0"}"#;
let signer = DidWebRoot::parse("did:web:example.com").ok_or("not a root did:web DID")?;

let catalog = AiCatalog::new(vec![CatalogEntry::new(
    EntryIdentifier::new("urn:air:example.com:a2a:research"),
    ArtifactType::A2A_AGENT_CARD,
    ArtifactSource::Url("https://agents.example.com/.well-known/agent-card.json".into()),
)]);
let params = SignerParams {
    signer: signer.clone(),
    key_fragment: "key-1".into(),
    issued_at: "2026-09-12T10:00:00Z".into(),
    expires_at: None,
};
let signed = sign_catalog(catalog, &params, &SigningKey::from_pkcs8_der(&KEY_DER))?;

// The DID document the signer would publish at https://example.com/.well-known/did.json.
let did_json = format!(
    r#"{{"id":"did:web:example.com","verificationMethod":[{{"id":"did:web:example.com#key-1",
    "type":"JsonWebKey2020","controller":"did:web:example.com","publicKeyJwk":{PUBLIC_JWK}}}],
    "assertionMethod":["did:web:example.com#key-1"]}}"#
);
let fetched = Fetched::new(200, Some("application/did+json".into()), did_json.into_bytes());
let doc = accept_document(&fetched, &signer)?;

let now = UNIX_EPOCH + Duration::from_secs(1_789_207_200);
let accepted = verify_signature(SignedObject::Catalog(&signed), &doc, &TrustPolicy::default(), now)?;
assert_eq!(accepted.kid.as_str(), "did:web:example.com#key-1");
# Ok::<(), Box<dyn std::error::Error>>(())
```

For a whole resolved tree, `verify` fetches each signer's DID document once and returns a
`TrustReport` with one signature outcome per catalog and per trust manifest, grouped by entry.

## Modules

| Module | Purpose |
| --- | --- |
| `payload` | The canonical bytes a detached JWS covers. |
| `sign` | ES256 signing of a manifest or a catalog. |
| `header` | The JWS protected-header policy and `kid` check. |
| `did_web` | DID document retrieval, acceptance and verification-key selection. |
| `accept` | Acceptance of one signature under a `TrustPolicy`. |
| `digest` | Artifact digests and integrity checks. |
| `report` | Entry coverage, publisher authority, endorsement, the trust report and `verify_entry`. |
| `endorse` | `endorse_entry` and `endorse_catalog`: digest and publisher manifest per entry. |
| `parameter_store` | Feature `ssm`: the signing key held in an SSM Parameter Store SecureString. |
| `registration` | Feature `registration`: the publisher verifier for runtime registration. |
| `identity` | `did:web` root and `urn:air` identifier shapes. |

## Feature flags

- `registration` (off by default): `DidWebPublisherVerifier`, the `PublisherVerifier` of
  [`turul-ai-catalog-storage`](../turul-ai-catalog-storage). It accepts an entry only when its
  `urn:air` publisher signed a trust manifest that covers it. Adds `turul-ai-catalog-storage`
  (without default features, so without a backend), `async-trait` and `tokio` (`rt`).
- `ssm` (off by default): `ParameterKeyStore`, which creates the signing key as a SecureString
  (never overwriting one) and loads it in process. Adds `aws-sdk-ssm`.

## Licence

Apache-2.0.
