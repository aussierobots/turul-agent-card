# ADR-018: Publisher signing tools: keys, DID document, artifact digest, endorsement, CLI

- **Status:** Proposed (accepted when every test file below is green, the `ssm` feature is in CI and docs.rs metadata, and no caller in the workspace computes an artifact digest, builds a DID document or signs an entry's manifest except through the owners below)
- **Date:** 2026-11-18
- **Implements:** under `crates/turul-ai-catalog-trust/tests/`: `signing_keys.rs`, `did_document.rs`, `artifact_digest.rs`, `endorsement.rs`, `parameter_store.rs` (feature `ssm`); under `crates/turul-ai-catalog-cli/tests/`: `trust_commands.rs`, and the migrated `dependency_direction.rs`
- **Depends on:** ADR-012 (`DidWebRoot`, `Kid`, `ES256`), ADR-013 (`SigningKey`, `SignerParams`, `sign_manifest`, `sign_catalog`, `compute`, `verify_artifact`), ADR-014 (`accept_document`, `select_key`, `document_url`), ADR-015 (`verify`, `verify_resolved`, `TrustPolicy`, `Endorsement`), ADR-008 (`fetch`, `FetchPolicy`, `load`), ADR-011 (the CLI; its dependency table changes, §6)
- **Spec:** https://ai-catalog.io, `specification/ai-catalog.md` (#the-did-web-signer-profile, #did-document-resolution-and-key-selection, #jws-algorithm-and-key-requirements, #digest-format, #verifying-artifact-integrity, #entry-release-coverage, #catalog-snapshot-coverage, #level-3-trusted-catalog).

## Context

M32 (maintainer, 2026-11-18): a publisher reaches Level 3 with this workspace's tools alone. A publisher needs a P-256 key held outside its code, a `did:web` DID document carrying the public key, the `digest` of every entry's artifact, a signed publisher Trust Manifest per entry, optionally a whole-catalog signature, and a way to check the result. ADR-013 already owns signing one manifest and one catalog, and ADR-015 owns verification. What is missing: generating a key, holding it in AWS Systems Manager Parameter Store, building the DID document, producing the digest a verifier will recompute, endorsing every entry of a catalog, and CLI commands over all of it. Without these a publisher writes its own key, JWK, DID and digest code, which is a second implementation of the format this workspace owns.

Spec text this ADR relies on:

- #level-3-trusted-catalog: a Trusted Catalog "Includes an acceptably signed, publisher-authorized Trust Manifest on each entry whose publisher authenticity is to be relied upon"; the catalog-level signature is a SHOULD.
- #entry-release-coverage: "A signed `subject` whose `identifier`, `type`, and `digest` exactly equal the corresponding entry fields. The entry MUST contain `digest`." and "Either both `subject.version` and `entry.version` absent, or both present and exactly equal."
- #verifying-artifact-integrity: "Retrieve bytes from `entry.url`, or use `entry.data`, observing Safe Fetching. Compute the digest of the retrieved bytes or the UTF-8 JCS-canonicalized `data` value".
- #jws-algorithm-and-key-requirements: ES256 over P-256 is the interoperable algorithm (ADR-012 allows only ES256).
- #did-document-resolution-and-key-selection: the key is the verification method whose `id` equals the `kid`, authorised by `assertionMethod`, with `controller` equal to the DID and a `publicKeyJwk` (ADR-014's `select_key` rules).

## Decision

### 1. Key generation (`SigningKey`)

`SigningKey::generate() -> SigningKey` draws a fresh P-256 key from the operating system's random source (`p256` 0.13 with `rand_core` `getrandom`, the `p256` that `turul-jwt-validator` already uses). `SigningKey::public_jwk(&self) -> Result<Json, SignError>` returns the public key as a JWK object with exactly `kty` `"EC"`, `crv` `"P-256"`, `x` and `y` (base64url, no padding), and never `d`; `SignError::Key` when the bytes are not a P-256 PKCS#8 key. The private key leaves the process only through a key sink (§2); `SigningKey` has no public accessor for its bytes and its `Debug` prints `SigningKey(..)`. Two calls to `generate` give different keys.

### 2. Parameter Store key source and sink (feature `ssm`)

Module `turul_ai_catalog_trust::parameter_store`, behind the crate feature `ssm` (`aws-sdk-ssm` 1, as `aws-sdk-dynamodb` is for storage). `ParameterKeyStore::new(client: aws_sdk_ssm::Client, name: String)`; the client is injected: tests point it at a loopback stand-in for the SSM JSON protocol and record each request with an interceptor.

- `create(&self, key: &SigningKey) -> Result<(), KeyStoreError>`: one `PutParameter` with `Type` `SecureString`, `Overwrite` false, `Tier` `Standard`, no `KeyId` (the AWS-managed `aws/ssm` key), value the PKCS#8 PEM of the key. `ParameterAlreadyExists` is `KeyStoreError::AlreadyExists`, and nothing is written: a key is never replaced by this crate.
- `load(&self) -> Result<SigningKey, KeyStoreError>`: one `GetParameter` with `WithDecryption` true; `ParameterNotFound` is `KeyStoreError::NotFound`; a parameter that is not a `SecureString` (never encrypted at rest), or a value that is not a P-256 PKCS#8 PEM, is `KeyStoreError::Malformed`.
- Any other service error is `KeyStoreError::Service(String)`, its message.
- IAM: the publishing role needs `ssm:PutParameter` (key creation only) and `ssm:GetParameter` on the parameter, and `kms:Decrypt` through `aws/ssm` is implied by SecureString. This repo ships no IAM.

The PEM is held only in memory; nothing here writes a file or prints a private key.

### 3. DID document (`did_document`)

`did_document(signer: &DidWebRoot, key_fragment: &str, public_jwk: &Json) -> Result<Json, SignError>` returns:

```json
{"@context":["https://www.w3.org/ns/did/v1","https://w3id.org/security/suites/jws-2020/v1"],
 "id":"did:web:<domain>",
 "verificationMethod":[{"id":"did:web:<domain>#<fragment>","type":"JsonWebKey2020","controller":"did:web:<domain>","publicKeyJwk":{...}}],
 "assertionMethod":["did:web:<domain>#<fragment>"]}
```

`SignError::Kid` when the fragment is not a non-empty RFC 3986 fragment (the `Kid` rule ADR-012 owns); `SignError::Key` when `public_jwk` is not a P-256 public JWK (`check_public_jwk`). The document is what ADR-014 accepts: its canonical bytes served as `application/did+json` pass `accept_document`, and `select_key` with the `kid` returns the key. It is served at `document_url(signer)`.

### 4. Artifact digest (`artifact_digest`)

`artifact_digest(entry: &CatalogEntry, base: Option<&str>, policy: &FetchPolicy) -> Result<String, IntegrityError>` returns `sha256:<hex>` of the entry's artifact bytes. The bytes come from the one private retrieval that `verify_artifact` also uses: JCS of `data`, or the body of `url` resolved against `base`, fetched with `fetch(url, None, policy)` and status 200. The request is the same as verification's, so a producer and a verifier hash the same representation; the spec names no `Accept` for artifact retrieval, and none is sent. Errors are `verify_artifact`'s, minus `DigestMissing`, `DigestInvalid` and `Mismatch`. SHA-256 is the floor the spec sets and the one algorithm produced.

### 5. Endorsement (`endorse_entry`, `endorse_catalog`)

`endorse_entry(entry: CatalogEntry, params: &SignerParams, key: &SigningKey) -> Result<CatalogEntry, SignError>`: the entry must carry `digest` (`SignError::DigestMissing`). A manifest is kept when its `contributor` is the signer, its `subject` binds the entry (`Subject::binds`: identifier, type, digest, version), neither it nor `params` has `expiresAt`, and its signature is accepted by `verify_signature` under the DID document `did_document` builds for this key and fragment (so the profile, the `kid` and the signature itself are checked, judged at 9999-12-31T23:59:59Z so the clock plays no part). Then the signer's other manifests are dropped as stale and the kept one stays byte for byte, `issuedAt` included. Any other case re-signs: a manifest made by another key, under another fragment, edited after signing, or with an expiry, since keeping it could leave a manifest a verifier rejects, and a requested expiry must be applied. Otherwise every manifest whose `contributor` is the signer is removed and one new manifest, `TrustManifest::new(signer)` signed by `sign_manifest`, is appended. Manifests of other contributors are never touched.

`endorse_catalog(catalog: AiCatalog, base: Option<&str>, params: &SignerParams, key: &SigningKey, policy: &FetchPolicy) -> Result<AiCatalog, EndorseError>`: for each entry, in order, whose identifier `publisher_authorized` accepts for the signer (ADR-015): keep `digest` as written when `verify_artifact` accepts it (so another contributor's manifest that binds that exact string keeps binding), else set it to `artifact_digest`; then `endorse_entry`. When any entry changed, the catalog's `signature` and `additionalSignatures` are removed, since they covered the old entries; `sign_catalog` signs the result again. Entries the signer is not the publisher of are left as they are; a nested-catalog entry is endorsed like any other when authorised. `EndorseError { entry: usize, kind: EndorseErrorKind }` with `Digest(IntegrityError)` or `Sign(SignError)`; the first failure stops and returns nothing. It does not sign the catalog itself; `sign_catalog` does, after endorsement when both are wanted. Blocking, as `verify_artifact`.

Re-running over an unchanged catalog and unchanged artifacts returns identical bytes (§5's keep rule; ES256 here is RFC 6979, so even a re-signature with the same `issuedAt` is identical). A changed artifact changes its digest, so its entry's subject no longer binds and the entry gets a new manifest: every artifact release needs an endorsement run before the catalog is published.

### 6. CLI

The CLI gains a dependency on `turul-ai-catalog-trust` (ADR-011 §1 changes: the trust crate is allowed in the CLI, not in the TUI) and a feature `ssm` that enables the trust crate's `ssm`, `aws-config` and `aws-sdk-ssm`. Commands, each thin over the owners above:

- `turul-ai-catalog key generate --ssm-parameter NAME --did DID --fragment F` (feature `ssm`): `SigningKey::generate`, `did_document` of its public JWK (so a bad fragment exits 2 before anything is written), `ParameterKeyStore::create`, then writes the document to stdout as canonical JSON. `AlreadyExists` exits 2. The private key is never printed or written to a file.
- `turul-ai-catalog key did-document --ssm-parameter NAME --did DID --fragment F` (feature `ssm`): `load`, then the same document.
- `turul-ai-catalog sign SOURCE --ssm-parameter NAME --did DID --fragment F --issued-at T [--expires-at T] [--catalog]` (feature `ssm`): loads SOURCE (`--file`, `--url`, `--well-known`) through `load` with `max_requests` 0 for a file and 1 for a URL (the root request), so nested catalogs are not followed and a root that redirects fails with a budget error; then `endorse_catalog` with the root's base, or with `--catalog` `sign_catalog` of the document as it is; writes the canonical result to stdout. Exit 0 written, 1 the loaded document has an error diagnostic, 2 any other failure.
- `turul-ai-catalog verify SOURCE [--require-publisher]` (always built): `load`, then `verify` with `TrustPolicy::default()` and the system clock. One line per catalog node (its source and its signature outcome) and per entry (identifier, the strongest endorsement `EntryReport::endorsement` gives, `publisher`, `contributor` or `none`, and integrity `ok`, `-` or the failure). A nested catalog that was not loaded is printed as `skipped <reason> <url>`, and one embedded as `data` (which `load` does not make a node, ADR-009) as `skipped embedded <identifier>`. Exit 1 when any present signature is rejected, any relied-on artifact fails integrity, any nested catalog was skipped or embedded, any loaded catalog has an error diagnostic, or, with `--require-publisher`, any entry has no publisher endorsement; 2 for load or usage failures; else 0. Every document-derived string goes through `DisplayText`.

## Owning symbols and entrypoints

| Artifact | Owner | Entrypoints |
|---|---|---|
| P-256 signing key | `SigningKey::generate`, `SigningKey::public_jwk` | `key generate` |
| Key at rest | `parameter_store::ParameterKeyStore::{create, load}` | `key generate`, `key did-document`, `sign` |
| DID document | `did_document` | `key generate`, `key did-document` |
| Artifact digest | `artifact_digest` (bytes shared with `verify_artifact`) | `endorse_catalog`, `sign` |
| Per-entry manifest | `endorse_entry` over `sign_manifest` | `endorse_catalog`, `sign` |
| Catalog endorsement | `endorse_catalog` | `sign` |
| Verification | `verify` (ADR-015) | `verify` |

## Requirement → test

| Requirement | Rule | Test function |
|---|---|---|
| key generation | fresh, distinct, P-256; JWK has `kty`, `crv`, `x`, `y` only | `signing_keys::generated_keys_are_distinct_p256_keys`, `signing_keys::public_jwk_has_no_private_member`, `signing_keys::debug_never_shows_the_key` |
| DID document | accepted by `accept_document`; `select_key` returns the key; a signature by the key verifies through `verify_resolved` as `Endorsement::Publisher` | `did_document::document_is_accepted_and_selects_the_key`, `did_document::generated_key_endorses_an_entry_end_to_end`, `did_document::bad_fragment_or_jwk_is_refused` |
| artifact digest | `data` digest is JCS SHA-256; `url` digest is the fetched body's; `verify_artifact` accepts it; non-200 refused | `artifact_digest::data_digest_is_sha256_of_jcs`, `artifact_digest::url_digest_matches_verification` (loopback), `artifact_digest::non_200_is_refused` |
| keep an unchanged release | an entry whose signer manifest binds it, under the same `kid` and without expiry, is returned byte-identical; otherwise re-signed | `endorsement::unchanged_release_keeps_its_manifest`, `endorsement::manifest_under_another_key_id_is_resigned`, `endorsement::manifest_signed_by_another_key_under_the_same_kid_is_resigned`, `endorsement::expiring_manifest_is_resigned`, `endorsement::requested_expiry_always_resigns`, `endorsement::stale_signer_manifests_are_dropped_when_one_is_kept` |
| re-endorse a changed release | a changed digest replaces only the signer's manifests | `endorsement::changed_digest_replaces_the_signers_manifest`, `endorsement::other_contributors_are_untouched`, `endorsement::entry_without_digest_is_refused` |
| endorse a catalog | authorised entries get digest and manifest; others untouched; result verifies with every authorised entry `Publisher` and integrity `Ok`; a second run is byte-identical | `endorsement::endorse_catalog_reaches_publisher_endorsement` (loopback), `endorsement::endorse_catalog_skips_entries_of_other_publishers`, `endorsement::endorse_catalog_is_idempotent`, `endorsement::first_failure_names_its_entry_and_returns_nothing`, `endorsement::a_matching_digest_is_kept_as_written`, `endorsement::a_changed_entry_drops_the_catalog_signature` |
| Parameter Store | `PutParameter` SecureString, `Overwrite` false; exists refused; `GetParameter` with decryption; not found and malformed refused | `parameter_store::create_puts_a_secure_string_without_overwrite`, `parameter_store::existing_parameter_is_refused`, `parameter_store::load_decrypts_and_parses`, `parameter_store::missing_or_malformed_parameter_is_refused` (including a plain `String` parameter) |
| CLI verify | one line per node and entry; exit 1 for a rejected signature, a skipped nested catalog, an invalid catalog, or with `--require-publisher` an unsigned entry; 2 for an unreadable source. A CLI run cannot resolve a DID document from loopback (`document_url` is always `https://`), so acceptance and integrity of CLI-signed output are proved through `verify_resolved` with the document injected | `trust_commands::verify_unsigned_entry_exits_0_and_prints_one_line_per_node_and_entry`, `trust_commands::verify_require_publisher_with_an_unsigned_entry_exits_1`, `trust_commands::verify_rejected_signature_exits_1`, `trust_commands::verify_skipped_nested_catalog_exits_1`, `trust_commands::verify_embedded_nested_catalog_exits_1`, `trust_commands::verify_catalog_with_an_error_diagnostic_exits_1`, `trust_commands::verify_unreadable_source_exits_2` |
| CLI key and sign | `key generate` writes the parameter and prints a DID document holding only the public key; an existing parameter or a bad fragment exits 2 and writes nothing; `sign` output verifies; `--catalog` signs the document as it is; exit 1 for a document error, 2 otherwise | `trust_commands::key_generate_writes_the_parameter_and_prints_only_the_public_key`, `trust_commands::key_generate_with_an_existing_parameter_exits_2_and_writes_nothing`, `trust_commands::key_generate_with_a_bad_fragment_exits_2_and_writes_nothing`, `trust_commands::sign_output_verifies`, `trust_commands::sign_catalog_flag_signs_the_document_as_it_is`, `trust_commands::sign_failures_exit_1_for_a_document_error_and_2_otherwise` (a loopback stand-in for the SSM endpoint) |
| dependency direction | the CLI may depend on the trust crate; the TUI may not; the trust crate's direct dependencies add `p256` and `rand_core` | `dependency_direction::cli_dependencies_are_exactly_the_allowed_set`, `dependency_direction::tui_dependencies_are_exactly_the_allowed_set`, `dependency_direction::trust_optional_features_add_only_storage_async_trait_tokio_and_the_ssm_client`, `crypto_boundary::direct_dependencies_are_only_the_named_crates` |

## Consequences

- A publisher writes no key, JWK, DID, digest or manifest code; it orchestrates these owners.
- Every artifact release changes that entry's digest and needs `sign` (or `endorse_catalog`) before the catalog is published; between the artifact release and the endorsement run, verification reports the entry's integrity as a mismatch.
- Parameter Store keeps the private key decryptable by the publishing role. Unlike a KMS asymmetric key, the private half enters the signing process's memory. The maintainer chose this for cost (M32).
- The CLI gains the trust crate and, with `ssm`, the AWS SDK for SSM.

## What this ADR does not do

- No KMS, no other key store, no key rotation command (a new key is a new parameter name and a new fragment in the DID document).
- No signing in the Lambda or the store: entries arrive signed through `import`, which keeps their bytes (ADR-016).
- No whole-catalog signature on a store-served catalog: `import` refuses a signed document (ADR-016); `sign --catalog` is for a catalog served as a static file.

## Revision log

| Date | Change |
|---|---|
| 2026-11-18 | Created (M32). |
| 2026-11-18 | Implementation review: `endorse_entry` keeps a manifest only under the same `kid` and without `expiresAt` (else a rotated or expiring manifest would be kept and later rejected); CLI `verify` fails a skipped nested catalog or an invalid catalog, which it had passed unverified; `sign` loads a URL root with one request; `key generate` builds the DID document before writing the key; the CLI `ssm` feature also enables `aws-sdk-ssm`; Parameter Store tests use a loopback SSM stand-in. |
| 2026-11-18 | Pre-push review: the keep rule verifies the kept signature under the DID document for this key (a reused fragment after a key change had been kept and rejected forever), drops the signer's stale manifests, and never keeps when an expiry is requested; `endorse_catalog` keeps a digest the artifact still matches and drops a catalog signature its changes invalidate; `load` refuses a parameter that is not a `SecureString`; CLI `verify` fails a catalog embedded as `data`; `EntryReport::endorsement` owns the strongest-endorsement rule the CLI prints. |
