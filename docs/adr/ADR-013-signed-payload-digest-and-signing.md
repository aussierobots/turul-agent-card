# ADR-013: Signed payload, artifact digest and ES256 signing (trust crate)

- **Status:** Accepted (2026-11-17): every test file below exists and passes, `schema_digest::signed_payload_goldens_match_recorded_digest` matches §4, and `manifest_payload` / `catalog_payload`, `verify_artifact` and `sign_manifest` / `sign_catalog` in the trust crate are the only payload, artifact-digest and signing paths (storage's SHA-256 builds record keys and ETags, not artifact digests).
- **Date:** 2026-11-17
- **Implements:** T1 added, under `crates/turul-ai-catalog-trust/tests/`: `signed_payload.rs`, `artifact_integrity.rs`, `schema_digest.rs`, `crypto_boundary.rs`, `sdk_trust_interop.rs` (payload and digest half). T2 added `signing.rs` and the signature half of `sdk_trust_interop.rs` and the validator line of `crypto_boundary.rs`.
- **Depends on:** ADR-002 (`parse`, model, `Members`), ADR-003 (`AiCatalog::to_json`, `TrustManifest::to_json`, `Json::to_canonical_vec`, `CanonicalError::NotIJson`, the schema-digest algorithm and its owner, the `spec-signed-payload.jcs` golden), ADR-005 (`DigestValue::parse`, `validate`), ADR-006 (`validate_manifest`, `Subject::for_entry`, `DID_WEB_V1`), ADR-008 (`client::fetch`, `Fetched`, `FetchPolicy`, `FetchError`, `client::resolve_reference`), ADR-009 (`ResolvedCatalog::base`), ADR-012 (`DidWebRoot`, `header::kid_matches_signer`, `TYP_*`). Binding decisions: D4 (JCS only), D5 (`sha2` in the trust crate only), D7 (ES256 signing through the validator), D17 (goldens, schema digest), M10, M12, M14.
- **Spec:** `target/upstream/ai-catalog/specification/ai-catalog.md` (last commit 2026-11-10). RFC 8785, RFC 7515 (Appendix A.3 ES256 key, Appendix F detached content), RFC 6234.

## Context

A signature covers "the containing Trust Manifest or catalog object, with only these two members omitted: 1. The immediate `jws` member of its immediate `signature` object. 2. Its immediate `additionalSignatures` member, if present. All other members and values MUST be retained … Ignoring an unrecognized field's meaning MUST NOT remove it from the payload. The omissions are not recursive" (#signed-payload-and-jws-construction). For a catalog that is the whole snapshot: "`specVersion`, every entry in order, Host Info, extensions, and all other present members, including unknown fields. Nested manifest signatures and nested `additionalSignatures` members remain included … Adding the catalog's own reserved `additionalSignatures` does not change its signed payload" (#catalog-snapshot-coverage). The payload is JCS bytes (ADR-003 owns JCS).

Artifact integrity: "Retrieve bytes from `entry.url`, or use `entry.data`, observing Safe Fetching. Compute the digest … using the algorithm named by `entry.digest`. Compare with `entry.digest`. A mismatch MUST fail artifact verification" and "`provenance[].sourceDigest` … MUST NOT be substituted for the digest of this entry's artifact" (#verifying-artifact-integrity). For `url`, "hash the exact retrieved artifact bytes"; for `data`, "the UTF-8 JCS-canonicalized … JSON value" (#catalog-entry `digest`).

D7 and M10 put ES256 signing in this crate through `turul-jwt-validator`'s `sign_detached_es256(pkcs8_der, typ, kid, payload) -> header..sig` (0.3.4). The format crate has no signing, hashing or JWS code (M6, M12).

Spec anchors implemented: #signed-payload-and-jws-construction (payload), #catalog-snapshot-coverage, #verifying-artifact-integrity, #well-known-uri (artifact retrieval by `url`), #safe-fetching (`data:` URIs, AMB-81), #catalog-entry (`digest`, `version`), #digest-format (computation), #level-3-trusted-catalog (catalog-level integrity), #trust-manifest-substitution, #consumer-requirements (retain unrecognised members in a payload).

## Decision

### 1. Payload (`payload` module)

- `manifest_payload(&TrustManifest) -> Result<Vec<u8>, PayloadError>` and `catalog_payload(&AiCatalog) -> Result<Vec<u8>, PayloadError>` both call one crate-private `payload_of(Json)`: take the object from `to_json()` (ADR-003, the one model → `Json` mapping), build a new `Members` holding every top-level member except `additionalSignatures`, and for `signature` a new `Members` holding every member except `jws`; then `Json::to_canonical_vec()` (no second JCS). Nothing below those two names is touched, so `jws` and `additionalSignatures` inside `data`, `extensions`, attestations, unknown members, nested manifests and nested catalogs stay, and every unknown member of the manifest, the catalog and the `signature` object stays (ADR-002 keeps them in the model; ADR-003 writes them).
- `CanonicalError::NotIJson { pointer }` becomes `PayloadError::NotIJson { pointer }`, the pointer relative to the signed object. Every finite number is written in its RFC 8785 form and only a number that is not a finite double is refused (ADR-003 §3, M20); no string is altered.
- An object without `signature` still has a payload (the object minus `additionalSignatures`); signing always sets `signature` first (§3).
- Law (T1), against the original bytes `b` and never our own output: for every corpus manifest and catalog whose numbers are finite doubles (documents holding another number are excluded and listed, as ADR-003 law 4 does), `manifest_payload(m)` equals `ai_catalog_trust::canonicalize_value_for_signature(v)` for the `serde_json` value `v` at the manifest's pointer in `ai_catalog_trust::parse_json(b)`, and `catalog_payload(parse(b))` equals `ai_catalog_trust::canonicalize_for_signature(b)`.

### 2. Artifact digest (`digest` module)

- `digest::compute(algorithm: &str, bytes: &[u8]) -> Result<String, IntegrityError>`: `algorithm` compared ASCII-case-insensitively (RD-13) with `sha256`, `sha384`, `sha512` (`sha2`, D5); the result is `<lowercase name>:<lowercase hex>`. Any other name is `IntegrityError::Unsupported(Unsupported::Algorithm(name))`: ADR-005 lets an unknown algorithm of 256 bits or more validate with a warning (AMB-23), but nothing here can compute it.
- `digest::verify_artifact(entry: &CatalogEntry, base: Option<&str>, policy: &FetchPolicy) -> Result<(), IntegrityError>`, in order:
  1. no `entry.digest` → `DigestMissing`; `DigestValue::parse` (ADR-005, the one digest parser) refuses it → `DigestInvalid`;
  2. bytes: `ArtifactSource::Data(j)` → `j.to_canonical_vec()` (`NotIJson { pointer }` on refusal, #catalog-entry "Inline `data` MUST satisfy JCS's input requirements for artifact verification to succeed"); `ArtifactSource::Url(u)` → `client::resolve_reference(base, u)` (AMB-73; a relative `url` without a base is `Unresolvable`); a resolved URL with the `data` scheme is `Unsupported(Unsupported::DataUri)` with no request (AMB-81, AMB-313); otherwise `client::fetch(&url, policy)` (a refusal, including the 4 MiB ceiling and a disallowed address, is `Fetch(FetchError)`), and status ≠ 200 is `Status(u16)` (AMB-311). The body is used as delivered under `Accept-Encoding: identity` (AMB-11);
  3. `compute(algorithm of entry.digest, bytes)`; its hex differs from the entry's hex → `Mismatch { expected, actual }`.
  Only `entry.digest` is compared; `provenance[].sourceDigest`, attestation digests and `subject.digest` are never read here.
- `base` is the containing `ResolvedCatalog` node's `base` (ADR-009): its fetched URL, `None` for a file. Retrieval by `url` exists only for this check; there is no general artifact pull (M3).
- `verify_artifact` is blocking (it calls `client::fetch`); rustdoc says so (§2 rules).

### 3. Signing (`sign` module)

- `SigningKey::from_pkcs8_der(&[u8]) -> SigningKey` keeps the bytes. The validator parses them at sign time, so a bad key is `SignError::Key` from `sign_detached_es256`'s `InvalidSigningKey`. No key is parsed, held or generated by trust `src/` itself (no `p256`).
- `SignerParams { signer: DidWebRoot, key_fragment: String, issued_at: String, expires_at: Option<String> }`. The signer is a root `did:web` DID by type (ADR-012 `DidWebRoot::parse`), so every `did-web-v1` signature this crate writes meets #the-did-web-signer-profile ("The Signature object's `signer` MUST be a root `did:web` DID"). `kid` is `header::Kid::parse(signer + "#" + key_fragment, &signer)`, the one `kid` rule (`kid_matches_signer`); `None` is `SignError::Kid`, so `key_fragment` must be a non-empty RFC 3986 fragment. `sign` holds no `kid` or signer rule of its own. `TYP_TRUST_MANIFEST` and `TYP_CATALOG` are `header`'s.
- `sign_manifest(entry: &CatalogEntry, manifest: TrustManifest, params: &SignerParams, key: &SigningKey) -> Result<TrustManifest, SignError>`:
  1. `Subject::for_entry(entry)` (ADR-006; `version` exactly when the entry has one) or `SignError::DigestMissing`; a `manifest.subject` that is present and differs is `SubjectMismatch`; otherwise it is set;
  2. `manifest.contributor` ≠ `params.signer` is `ContributorMismatch` (#contributor-identity: "`signature.signer` MUST exactly equal `contributor`"); nothing is rewritten (AMB-312);
  3. `signature` = `{signer, profile: DID_WEB_V1, issuedAt, expiresAt?}` with an empty `jws`; `additionalSignatures` is left as given (it is outside the payload);
  4. `validate_manifest` (ADR-006) error → `SignError::Invalid(Code)` (AMB-312: we never sign what a verifier rejects as `Reason::Invalid`, ADR-015);
  5. `manifest_payload` (`Payload(PayloadError)`), then `sign_detached_es256(key, TYP_TRUST_MANIFEST, kid, payload)` → `jws`.
- `sign_catalog(catalog: AiCatalog, params: &SignerParams, key: &SigningKey) -> Result<AiCatalog, SignError>`: steps 3 to 5 with `validate` and `catalog_payload` and `TYP_CATALOG`. Nested manifests, their signatures and every entry in order are inside the payload, so reordering, adding or removing an entry or a contribution breaks the signature, and adding the root's own `additionalSignatures` does not (#catalog-snapshot-coverage).
- The header is the validator's `{"alg":"ES256","typ":…,"kid":…}`; every output passes `header::inspect` (ADR-012) and the validator's `verify_detached`, and `ai_catalog_trust::verify_es256` accepts it (T2 interop).
- `SignError::{Key, Kid, DigestMissing, SubjectMismatch, ContributorMismatch, Invalid(Code), Payload(PayloadError)}` (`#[non_exhaustive]`, `Display`, `std::error::Error`).

### 4. Fixtures, goldens and schema digest (D17)

- `crates/turul-ai-catalog-trust/tests/fixtures/signed_payload/` holds hand-authored inputs and their payload goldens, never produced by our writer: `manifest-reserved-names.input.json` / `.jcs` (a signed manifest with `jws` inside `extensions`, `additionalSignatures` inside an attestation's unknown member, an unknown member inside `signature`, and a root `additionalSignatures`), and `catalog-snapshot.input.json` / `.jcs` (a catalog root with `signature`, root `additionalSignatures`, `host`, `extensions`, an unknown member and an entry carrying a signed manifest with its `jws`). Each `.jcs` is produced by removing the two immediate members by hand and running the `python3 json.dumps(…, ensure_ascii=False, separators=(",",":"), sort_keys=True)` command ADR-003 records (valid because every name is in the BMP and every number is an integer); `SOURCE.md` records the command. The spec payload golden is ADR-003's `crates/turul-ai-catalog/tests/fixtures/canonical/spec-signed-payload.jcs`, read by path and never copied (one golden).
- `crates/turul-ai-catalog-trust/tests/fixtures/keys/`: `rfc7515-a3.pk8` (PKCS#8 DER derived from the RFC 7515 Appendix A.3 private JWK) and `rfc7515-a3.public.jwk.json` (the same JWK without `d`), with a `SOURCE.md` giving the RFC section, the derivation command and each file's sha256. It is a published test key, never a credential, so no `p256` dev-dependency is needed. It is outside the schema-digest directory.
- Schema digest: ADR-003's algorithm (owner `crates/turul-ai-catalog/tests/support/schema_digest.rs`, included by `#[path]`) over `tests/fixtures/signed_payload/`. Recorded digest: `eebddd1ef08d76185c6de695dbf491e024a33509469b16c59bcc0c3497822e35` (also `RECORDED` in `tests/schema_digest.rs`).

### 5. Crypto boundary

`tests/crypto_boundary.rs`: `cargo tree -e normal -p turul-ai-catalog-trust --depth 1` lists only `turul-ai-catalog`, `turul-ai-catalog-client`, `sha2` and (from T2) `turul-jwt-validator`; `cargo metadata` resolve data shows the validator's features are `detached_rust_crypto` and what it implies, never `jwks` (M14); trust `src/` has no `std::net`, no base64 alphabet literal and no `split('.')` / `split(".")`; `crates/turul-ai-catalog/src/` has no `jws` or `base64url` module and no base64 alphabet literal (M12). Validator line: `turul-jwt-validator = { version = "0.3.4", default-features = false, features = ["detached_rust_crypto"] }` (= `^0.3.4`) in `[workspace.dependencies]`.

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Manifest payload bytes | `payload::manifest_payload` → crate-private `payload_of` → `Json::to_canonical_vec` | `manifest_payload`; `sign_manifest`; ADR-015 `verify_signature` |
| Catalog payload bytes | `payload::catalog_payload` → `payload_of` | `catalog_payload`; `sign_catalog`; ADR-015 `verify_signature` |
| Artifact digest string | `digest::compute` | `compute`; `verify_artifact` |
| Artifact integrity verdict | `digest::verify_artifact` (bytes through `client::fetch` / `Json::to_canonical_vec`, reference through `client::resolve_reference`) | itself; ADR-015 `entry_release_coverage` |
| Stored `jws` (producer) | `sign::sign_manifest`, `sign::sign_catalog` → `sign_detached_es256` | themselves |
| Signed subject | `Subject::for_entry` (ADR-006) | `sign_manifest` |
| Signing key input | `sign::SigningKey::from_pkcs8_der` | — |
| Payload goldens | `tests/fixtures/signed_payload/` with `SOURCE.md` | `tests/signed_payload.rs` |
| Schema digest of the goldens | §4 recorded line (set in T1) | `schema_digest::signed_payload_goldens_match_recorded_digest` |
| Errors | `PayloadError::NotIJson`, `IntegrityError::{DigestMissing, DigestInvalid, Unsupported(Unsupported::{Algorithm(String), DataUri}), Unresolvable, Fetch(FetchError), Status(u16), NotIJson, Mismatch}`, `SignError` | — |

## Requirement → test

Test paths are under `crates/turul-ai-catalog-trust/tests/`. Ids number requirement statements per section as in PLAN §4.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| signed-payload-and-jws-construction-1 | "All other members and values MUST be retained, including `signature.signer`, … and unrecognized members" | `signed_payload::spec_example_manifest_payload_equals_golden` (the #signed-payload-and-jws-construction manifest with a `jws` added equals ADR-003's golden), `signed_payload::every_signature_member_but_jws_is_retained` (`expiresAt` and an unknown `signature` member), `signed_payload::reserved_names_below_the_top_are_retained` (golden `manifest-reserved-names`) |
| signed-payload-and-jws-construction-2 | "Ignoring an unrecognized field's meaning MUST NOT remove it from the payload" | `signed_payload::unknown_members_at_every_depth_are_retained` (manifest, subject, signature, attestation, provenance link, trust schema) |
| consumer-requirements-6 | "MUST retain unrecognized members when constructing a signed payload" | `signed_payload::unknown_members_at_every_depth_are_retained` |
| verifying-artifact-integrity-1 | "A mismatch MUST fail artifact verification" | `artifact_integrity::mismatch_fails` (`data` and loopback `url`), `artifact_integrity::data_entry_hashes_its_jcs_bytes`, `artifact_integrity::url_entry_hashes_the_retrieved_body` |
| verifying-artifact-integrity-2 | "`sourceDigest` … MUST NOT be substituted for the digest of this entry's artifact" | `artifact_integrity::source_digest_is_never_substituted` (a `provenance[].sourceDigest` equal to the bytes' digest leaves `Mismatch`) |
| well-known-uri-3 | "the client SHOULD use the entries' `url` values to retrieve individual artifacts" | `artifact_integrity::relative_url_resolves_against_the_catalog_base` (loopback catalog, D16), `artifact_integrity::relative_url_from_a_file_root_is_unresolvable` |
| catalog-entry-18 | "An entry MAY include that version when it needs to be signed directly" | `signing::subject_version_is_present_exactly_when_the_entry_has_one` |
| level-3-trusted-catalog-5 | "SHOULD provide catalog-level integrity through … a signature satisfying Catalog Snapshot Coverage" | `signing::sign_catalog_covers_the_whole_snapshot` (any change to `specVersion`, `host`, `extensions`, an unknown member, a nested manifest or its `jws` breaks it; root `additionalSignatures` does not) |
| trust-manifest-substitution-1 | "Hosts SHOULD provide a signature satisfying Catalog Snapshot Coverage" | `signing::reordering_entries_breaks_the_catalog_signature`, `signing::adding_or_removing_an_entry_breaks_the_catalog_signature` |

Behaviour of other anchors, pinned without a requirement id of their own:

| Rule | Anchor | Test function |
|---|---|---|
| catalog payload keeps nested signatures; root `additionalSignatures` excluded | #catalog-snapshot-coverage | `signed_payload::catalog_payload_keeps_nested_manifests_with_jws` (golden `catalog-snapshot`), `signed_payload::root_additional_signatures_do_not_change_the_catalog_payload` |
| non-I-JSON number in a payload is refused, never altered | #signed-payload-and-jws-construction | `signed_payload::non_i_json_number_is_refused_with_its_pointer` |
| `compute` names and lengths | #digest-format | `artifact_integrity::sha256_of_empty_input_is_the_known_value` (`sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`), `artifact_integrity::sha384_and_sha512_are_computed`, `artifact_integrity::algorithm_name_is_case_insensitive` |
| ES256 output, header, verification by both implementations | #signed-payload-and-jws-construction, #jws-algorithm-and-key-requirements | `signing::output_is_detached_with_header_alg_typ_kid`, `signing::validator_verifies_the_output`, `sdk_trust_interop::ai_catalog_trust_verifies_our_signature` (`verify_es256` with `P256Key::from_jwk` of the public A.3 JWK), `signing::any_payload_change_breaks_the_signature` (an unknown member, `signature.issuedAt`) |
| key and producer refusals | #contributor-identity, #manifest-validity | `signing::truncated_der_is_a_key_error_at_sign_time`, `signing::entry_without_digest_is_digest_missing`, `signing::contributor_other_than_signer_is_refused`, `signing::differing_subject_is_refused`, `signing::invalid_manifest_is_not_signed`, `signing::kid_fragment_outside_rfc_3986_is_refused`, `signing::signer_outside_the_did_web_profile_cannot_sign` (https, uppercase, encoded and bare port, path-based, IP and non-`did:web` signers do not parse as `DidWebRoot`); `jws_header::signer_output_passes_inspect` |

Decisions owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| RD-13 (verify half) | name case-insensitive; only sha256/384/512 computed; anything else `Unsupported` | `artifact_integrity::unknown_algorithm_is_unsupported` (`sha3-256:…`); interop `sdk_trust_interop::digest_verdicts_agree_for_every_data_entry` |
| RD-40 (payload half, closed) | a payload holding a number that is not a finite double is refused, and the document is excluded from the interop law; every finite number is written as ai-catalog-trust writes it (ADR-003 M20) | `signed_payload::non_i_json_number_is_refused_with_its_pointer`; `sdk_trust_interop::payloads_agree_for_every_i_json_corpus_object` |
| RD-19 (artifact half) | every artifact fetch goes through `client::fetch` | `crypto_boundary::trust_src_has_no_network_access_but_client_fetch` |
| AMB-11 | digest of the body under `Accept-Encoding: identity` | `artifact_integrity::url_entry_hashes_the_retrieved_body` |
| AMB-20 (integrity step) | integrity is one step of coverage, called once per entry by ADR-015 | `artifact_integrity::verify_artifact_compares_entry_digest_only` |
| AMB-73 | relative `url` through `client::resolve_reference` | `artifact_integrity::relative_url_resolves_against_the_catalog_base`, `artifact_integrity::relative_url_from_a_file_root_is_unresolvable` |
| AMB-81 | `data:` entry `url` is `Unsupported(DataUri)`, no request | `artifact_integrity::data_uri_url_is_unsupported_without_a_request` (the loopback server's log is empty) |
| AMB-311 | artifact status ≠ 200 is `Status` | `artifact_integrity::non_200_status_is_refused` (404 whose body would match) |
| AMB-312 | the signer never rewrites `contributor` or `subject` and never signs an invalid object | `signing::contributor_other_than_signer_is_refused`, `signing::differing_subject_is_refused`, `signing::invalid_manifest_is_not_signed` |
| AMB-313 | `data:` is detected on the resolved URL | `artifact_integrity::data_uri_url_is_unsupported_without_a_request` (`DATA:…` and `data:…`) |
| Fetch ceiling | a body over 4 MiB is `Fetch` with the client's size error | `artifact_integrity::body_over_the_ceiling_is_a_fetch_error` |
| Payload law | `manifest_payload` / `catalog_payload` equal ai-catalog-trust over the original bytes | `sdk_trust_interop::payloads_agree_for_every_i_json_corpus_object` (corpus includes unknown members on every kind and a nested signed manifest) |
| Schema digest | ADR-003 algorithm over `signed_payload/` | `schema_digest::signed_payload_goldens_match_recorded_digest`, `schema_digest::schema_digest_has_one_definition` |
| Crypto boundary | §5 | `crypto_boundary::direct_dependencies_are_only_the_named_crates`, `crypto_boundary::validator_features_exclude_jwks`, `crypto_boundary::trust_src_has_no_network_access_but_client_fetch`, `crypto_boundary::no_base64_or_jws_splitting_in_trust_src`, `crypto_boundary::format_crate_has_no_jws_or_base64url` |

New ids (not in PLAN §6), from the block AMB-311..:

- AMB-311: #verifying-artifact-integrity says "Retrieve bytes from `entry.url`" and does not say what an error response is. A non-200 body is not the artifact, so it is refused as `Status` rather than hashed; a mismatch report would misdescribe it.
- AMB-312: #contributor-identity and #subject-object make `signer` = `contributor` and subject = entry MUSTs on the producer. Rewriting a caller's `contributor` or `subject` to make them hold would silently change the claim being signed, so the signer refuses instead; it also refuses an object with an error diagnostic, which ADR-015 would reject as `Reason::Invalid`.
- AMB-313: `client::resolve_reference` returns the serialised resolved URL, whose scheme is lowercase, so the `data:` test runs on it and needs no scheme parser of its own.

## Divergences from ai-catalog-rust 0.4.0

| Id | ai-catalog-rust | This crate | Spec | Pinned by |
|---|---|---|---|---|
| RD-13 | `ParsedDigest::parse` refuses any algorithm outside sha256/384/512 (and calls md5/sha1/sha224 weak) | the same in effect at verification (`Unsupported`); validation keeps the AMB-23 length rule (ADR-005) | #digest-format | `sdk_trust_interop::digest_verdicts_agree_for_every_data_entry` |
| RD-19 (artifacts) | `ai_catalog_validate::verify` hands the URL to a caller `Fetch` returning bytes only, so status and Safe Fetching are the caller's; a relative `url` joins `Options::base_url`, a caller value | `client::fetch` under `FetchPolicy`, status 200 only, base = the containing catalog's fetched URL | #safe-fetching MUST; #verifying-artifact-integrity | `artifact_integrity::non_200_status_is_refused`, `artifact_integrity::relative_url_resolves_against_the_catalog_base` |

Agreements: payload member selection (`strip_signature` omits the same two immediate members, non-recursively), `data` hashed as its JCS bytes, `data:` and other non-`https` artifact URLs never fetched, signatures verified by `verify_es256`. ai-catalog-rust has no signing API, so T2 interop checks our output with its verifier only.

## Consequences

- One payload path: ADR-015 verification and this ADR's signer both call `manifest_payload` / `catalog_payload`, so a verifier and a signer cannot disagree on bytes. A second canonical writer or a hand-built payload is a defect (adr-review check 3).
- An artifact only reachable as a `data:` URL, or a payload with a non-I-JSON number, can never be covered here; publishers use `data` and string-encoded large numbers (AMB-81, ADR-003).
- `sign_*` takes timestamps as text and relies on `validate_manifest` / `validate` for their syntax and order, so one validator decides what a signer may emit.
- Plan drift recorded:
  - PLAN names `sign_manifest` / `sign_catalog` without parameters; `SignerParams` and the `ContributorMismatch`, `SubjectMismatch`, `Invalid` and `Kid` refusals are added here (AMB-312).
  - PLAN's T1 law compares `manifest_payload(parse(b))` with `canonicalize_for_signature(b)`; a corpus manifest sits inside a catalog, so the law is stated on the manifest's subtree through `canonicalize_value_for_signature`.
  - `IntegrityError::{DigestMissing, DigestInvalid, Status}` are not in PLAN; `Status` is AMB-311.
  - The schema digest of §4 cannot be computed before the goldens exist; T1 records it in the same slice.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created ahead of T1 from PLAN §3 (ADR-013 row), §4, T1, T2, D5, D7, D17, AMB-11, AMB-20, AMB-73, AMB-81, RD-13, RD-19, RD-40 and validator 0.3.4 `sign_detached_es256`. AMB-311 to AMB-313 added. |
| 2026-11-17 | T1 landed: payload and digest code, tests, goldens. Schema digest of `signed_payload/` recorded as `eebddd1ef08d76185c6de695dbf491e024a33509469b16c59bcc0c3497822e35`. `validator_features_exclude_jwks` stays with T2 (no validator dependency yet). |
| 2026-11-17 | T2 landed: `sign` module, `signing.rs`, signature half of `sdk_trust_interop.rs`, `validator_features_exclude_jwks`. `SignerParams.signer` is `String` until ADR-012's `DidWebRoot` exists (S14). |
| 2026-11-17 | Signer-shape defect fixed: `sign_manifest` and `sign_catalog` wrote `profile: did-web-v1` for any non-empty signer without `#` (`https://…`, `did:web:Example.COM`, `did:web:example.com:8443`, `did:example:…`), output the crate's own `header::inspect` refused as `KidMismatch`. `SignerParams.signer` is now `DidWebRoot`, and `kid` is built through `header::Kid::parse`; the local `kid` rule in `sign` is deleted. `SignError::Kid` now means only a bad `key_fragment`. Added `signing::signer_outside_the_did_web_profile_cannot_sign`; `signing.rs`, `jws_header.rs`, `sdk_trust_interop.rs` and `support/world.rs` build `SignerParams` from `DidWebRoot`. |
| 2026-11-17 | M20 (ADR-003 AMB-94): a payload or `data` digest holds every finite number in its RFC 8785 form, so payloads from RFC 8785-conformant producers are reproduced; `NotIJson` is only a number that is not a finite double. RD-40 closed: its divergence row is deleted, because `canonicalize_for_signature` (`serde_jcs` 0.2) also writes integers through `f64`. The payload law excludes only documents with a number that is not a finite double. |
| 2026-11-17 | Accepted: T1 and T2 test files exist and pass, the recorded schema digest matches, and no second payload, artifact-digest or signing path exists in the workspace. |
