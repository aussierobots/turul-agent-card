# ADR-014: `did:web` resolution, DID document and key selection (trust crate)

- **Status:** Accepted (T3 landed both test files; trust `src/` reaches the network only through `client::fetch`)
- **Date:** 2026-11-17
- **Implements:** `crates/turul-ai-catalog-trust/tests/did_resolution.rs`, `crates/turul-ai-catalog-trust/tests/key_selection.rs`.
- **Depends on:** ADR-002 (`json::read`, `Json`), ADR-003 (`Json::to_canonical_vec`), ADR-008 (`client::fetch`, `Fetched`, `Fetched::new`, `FetchPolicy`, `FetchError`, the media-type essence rule `accept_response` uses), ADR-012 (`DidWebRoot`, `Kid`). Binding decisions: D8, M4 (loopback-only tests), M10 (JWK checks are the validator's), M14. JWK API: `turul-jwt-validator` 0.3.4 `detached::{check_public_jwk, Es256PublicKey, DetachedJwsError::InvalidJwk}`.
- **Spec:** `target/upstream/ai-catalog/specification/ai-catalog.md` (last commit 2026-11-10). DID Core 1.0 (§3.2 DID URL syntax and relative DID URLs, §5.3 verification relationships, §6 representations), did:web method (Read), RFC 7517.

## Context

"The verifier MUST resolve `signer` according to the `did:web` method [[DIDWEB]] and process the result as a DID document according to DID Core [[DIDCORE]]. The resolution MUST satisfy the safe-fetching requirements … Resolution fails under the `did:web` Signer Profile if the DID document cannot be retrieved and validated or if its `id` is not exactly equal to `signer`" (#did-document-resolution-and-key-selection). The key must be "authorized by the DID document's `assertionMethod` verification relationship … After resolving relative DID URLs as defined by DID Core, the verifier MUST select exactly one verification method whose `id` exactly equals `kid`", its `controller` "MUST exactly equal `signer`", and it "MUST contain `publicKeyJwk`" describing a P-256 public key with the `alg`/`use`/`key_ops` limits. "This profile verifies against the DID document returned at verification time" (#current-state-verification). The Signature object's `signer` gains authority only through this: "The applicable signer profile MUST establish that the verification key is authorized to sign for this identity" (#signature-object).

ADR-012 has already fixed the signer to a root `did:web` DID (no port, path or IP), so a DID-document URL never names a loopback server and `resolve` cannot run against M4's loopback servers.

Spec anchors implemented: #the-did-web-signer-profile (key discovery), #did-document-resolution-and-key-selection, #current-state-verification, #safe-fetching (DID-document fetch), #signature-object (`signer` authorization).

## Decision

### 1. Resolution (`did_web` module)

- `did_web::document_url(&DidWebRoot) -> String` = `https://` + `domain()` + `/.well-known/did.json` (did:web Read for a DID with no path; the domain is already lowercase LDH with no port, so no percent-decoding applies).
- `did_web::accept_document(&Fetched, &DidWebRoot) -> Result<DidDocument, ResolveError>` is the one DID-document gate, in order:
  1. status ≠ 200 → `Status(u16)`;
  2. media-type essence (the client's rule, ADR-008) not `application/did+json`, `application/did+ld+json` or `application/json`, or absent → `MediaType(Option<String>)` (AMB-80; DID Core §6 representations plus the generic JSON type did:web hosts commonly serve);
  3. body through `json::read` (ADR-002: strict RFC 8259, duplicate members refused, which a DID document's JSON representation also forbids) → `Json(ParseError)`;
  4. DID Core structure of what the profile reads (AMB-321), else `Invalid { pointer }`: the root is an object; `id` is a string; `verificationMethod`, when present, is an array of objects each with string `id`, `type` and `controller`; `assertionMethod`, when present, is an array whose items are strings or such objects;
  5. `id` exactly equal to `root.as_str()` (decoded strings, case-sensitive) → otherwise `IdMismatch`.
  Nothing else in the document is read (`service`, `controller`, `authentication`, `alsoKnownAs`, …), so nothing else can affect the outcome; every value is untrusted input (#safe-fetching).
- `did_web::resolve(&DidWebRoot, &FetchPolicy) -> Result<DidDocument, ResolveError>` is `client::fetch(&document_url(root), policy)` (`Fetch(FetchError)` on refusal) then `accept_document`, nothing else. Redirects, address vetting, size and time limits are `client::fetch`'s (ADR-008). Blocking; rustdoc says to call it through `spawn_blocking` from async code (§2 rules).
- No cache, no stored document, no retry: every verification resolves the document current at that time (#current-state-verification). A key removed from `assertionMethod` stops verifying at once; history is not provable (#current-state-verification, second paragraph), and nothing here claims it.
- `DidDocument` holds its `DidWebRoot` (`DidDocument::id()`) and the read `Json`; it is built only by `accept_document`, so a `DidDocument` always passed steps 1 to 5 for that root.

### 2. Key selection

`select_key(&DidDocument, &Kid) -> Result<PublicJwk, KeySelectionError>`, in order:

1. Candidates (AMB-28): every object in `verificationMethod`, and every object item of `assertionMethod`, whose resolved `id` equals `kid.as_str()` exactly. A string item of `assertionMethod` is a reference, not a candidate, so a reference and its target count once; an embedded `assertionMethod` object with the same `id` as a `verificationMethod` object is a second, distinct method. Zero candidates → `NotFound`; more than one → `Ambiguous`.
2. Authorisation: the candidate is authorised when it is embedded in `assertionMethod`, or when an `assertionMethod` string resolves exactly to `kid`. Otherwise `NotAuthorized` (a key under `authentication`, `keyAgreement` or only in `verificationMethod` is never accepted).
3. `controller` exactly equal to the document's `id` (= signer, §1 step 5) → otherwise `ControllerMismatch`.
4. `publicKeyJwk` present → otherwise `NoPublicKeyJwk`.
5. `turul_jwt_validator::detached::check_public_jwk(&jwk.to_canonical_vec())` (ADR-003 bytes; `JwkNotIJson { pointer }` if the writer refuses): `kty` `EC`, `crv` `P-256`, `x`/`y` 32-byte base64url coordinates on the curve, no `d`, and `alg`/`use`/`key_ops` limits when present, each refused as `Jwk(DetachedJwsError::InvalidJwk(reason))` (handoff "Agreed": one rejection per rule, tested in the validator on both backends). The bytes are the strict reader's value re-serialised, so the validator's `serde_json` never sees a duplicate member (AMB-323).
6. `Ok(PublicJwk)`: a newtype over the validator's `Es256PublicKey`, built only here, so ADR-015 can hand `verify_detached` only a selected and checked key. A JWK `kid` member is never read: the verification method's `id` is authoritative.

Relative DID URLs (AMB-322): an `id` or `assertionMethod` string that starts with `#` resolves to the document `id` followed by it (RFC 3986 §5.2.2 for a fragment-only reference; DID Core §3.2.2 uses the DID document `id` as base). Any other value is compared as given. `client::resolve_reference` is not used: it serialises through the WHATWG URL parser, which re-encodes and normalises, and the profile compares exact strings.

### 3. Test seam (M4) and named gap

`document_url` can never name a loopback server (AMB-77 forbids ports and IP addresses), so T3 tests `document_url` by string and `accept_document` on `Fetched::new` values, and `resolve`'s fetch half is `client::fetch`, tested by S8. Named gap, not a new control: `resolve` never runs against a real `did:web` host in CI. No fetch override, injectable resolver or test-only feature is added.

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| DID-document URL of a signer | `did_web::document_url(&DidWebRoot)` | `did_web::resolve` |
| Accepted DID document | `did_web::accept_document(&Fetched, &DidWebRoot) -> DidDocument` | `did_web::resolve`; tests through `Fetched::new` |
| Resolution | `did_web::resolve(&DidWebRoot, &FetchPolicy)` = `client::fetch` + `accept_document` | ADR-015 `verify` |
| Selected verification key | `select_key(&DidDocument, &Kid) -> PublicJwk` (JWK content: `check_public_jwk`) | ADR-015 `verify_signature` |
| Errors | `ResolveError::{Fetch(FetchError), Status(u16), MediaType(Option<String>), Json(ParseError), Invalid { pointer }, IdMismatch}`, `KeySelectionError::{NotFound, Ambiguous, NotAuthorized, ControllerMismatch, NoPublicKeyJwk, JwkNotIJson { pointer }, Jwk(DetachedJwsError)}` (`#[non_exhaustive]`) | — |

No derived artifact or golden belongs to this ADR.

## Requirement → test

Test paths are under `crates/turul-ai-catalog-trust/tests/`. Ids number requirement statements per section as in PLAN §4 (the JWK `kty`/`crv`/`x`/`y` sentence is one statement, and so is the `alg`/`use`/`key_ops` sentence).

| Requirement | Spec text (short) | Test function |
|---|---|---|
| did-document-resolution-and-key-selection-1 | "MUST resolve `signer` according to the `did:web` method … and process the result as a DID document according to DID Core" | `did_resolution::document_url_is_the_well_known_did_json` (`did:web:example.com` → `https://example.com/.well-known/did.json`), `did_resolution::document_id_other_than_signer_is_refused`, `did_resolution::malformed_verification_relationships_are_refused`, `did_resolution::duplicate_member_is_refused`, `did_resolution::status_other_than_200_is_refused` (404), `did_resolution::non_did_media_type_is_refused` (`text/html`, absent), `did_resolution::did_and_json_media_types_are_accepted` (`application/did+json`, `application/did+ld+json`, `application/json`, with a `charset` parameter) |
| did-document-resolution-and-key-selection-2 | "The resolution MUST satisfy the safe-fetching requirements" | `did_resolution::resolve_is_fetch_then_accept_document` (source scan of `did_web::resolve`), `did_resolution::trust_src_reaches_the_network_only_through_client_fetch` (source scan) |
| did-document-resolution-and-key-selection-3 | "MUST be authorized by the DID document's `assertionMethod` verification relationship" | `key_selection::referenced_assertion_method_key_is_selected`, `key_selection::embedded_assertion_method_key_is_selected` |
| did-document-resolution-and-key-selection-4 | "MUST select exactly one verification method whose `id` exactly equals `kid`" | `key_selection::two_methods_with_the_kid_are_ambiguous`, `key_selection::reference_and_target_count_once`, `key_selection::fragment_reference_resolves_against_the_document_id` (`#k`), `key_selection::unknown_kid_is_not_found` |
| did-document-resolution-and-key-selection-5 | "A key used only for another relationship … MUST NOT be accepted" | `key_selection::authentication_only_key_is_not_authorized`, `key_selection::verification_method_only_key_is_not_authorized`, `key_selection::key_removed_from_assertion_method_is_not_authorized` |
| did-document-resolution-and-key-selection-6 | "`controller` MUST exactly equal `signer`" | `key_selection::controller_other_than_signer_is_refused` (`did:web:other.example`, `did:web:Example.com`) |
| did-document-resolution-and-key-selection-7 | "MUST contain `publicKeyJwk`" | `key_selection::method_without_public_key_jwk_is_refused` |
| did-document-resolution-and-key-selection-8 | "`kty` MUST be `EC`, `crv` MUST be `P-256`, and `x` and `y` MUST contain valid curve coordinates" | `key_selection::jwk_with_p384_is_refused`, `key_selection::jwk_off_the_curve_is_refused`, `key_selection::jwk_with_kty_rsa_is_refused` (each `Jwk(InvalidJwk(_))`) |
| did-document-resolution-and-key-selection-9 | "It MUST NOT contain private key material" | `key_selection::jwk_with_d_is_refused` |
| did-document-resolution-and-key-selection-10 | "`alg` MUST be `ES256`, `use` MUST be `sig`, and `key_ops` MUST permit `verify`" | `key_selection::jwk_with_alg_es384_is_refused`, `key_selection::jwk_with_use_enc_is_refused`, `key_selection::jwk_with_key_ops_sign_is_refused`, `key_selection::jwk_with_es256_sig_verify_is_accepted` |
| signature-object-4 | "The applicable signer profile MUST establish that the verification key is authorized to sign for this identity" | `key_selection::only_an_assertion_method_key_controlled_by_the_signer_is_authorized` |

Decisions owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| AMB-28 | count distinct method objects whose resolved `id` = `kid`; a reference and its target once; exactly one, authorised by `assertionMethod` | `key_selection::two_methods_with_the_kid_are_ambiguous`, `key_selection::reference_and_target_count_once`, `key_selection::embedded_duplicate_of_a_listed_method_is_ambiguous`, `key_selection::other_es256_keys_are_never_counted` |
| AMB-80 | status 200; DID or JSON media type; strict reader | the `did_resolution` status, media-type and duplicate tests above |
| AMB-321 | "validated" = strict JSON, `id`, and the shape of `verificationMethod` / `assertionMethod`; nothing else read | `did_resolution::malformed_verification_relationships_are_refused` (`verificationMethod: [42]`, `assertionMethod: {}`, a method without `controller`), `did_resolution::unread_members_do_not_affect_acceptance` (a `service` of any shape) |
| AMB-322 | `#`-references resolve against the document `id`; anything else compared verbatim | `key_selection::fragment_reference_resolves_against_the_document_id`, `key_selection::non_fragment_relative_reference_is_compared_verbatim` (`key-1`, `./web:example.com#k`) |
| AMB-323 | the JWK reaches the validator as JCS bytes of the strictly read value | `key_selection::jwk_is_checked_from_its_canonical_bytes` (member order and whitespace in the source do not matter) |
| JWK `kid` | ignored for selection | `key_selection::jwk_kid_differing_from_the_method_id_is_ignored` |
| Seam | `accept_document` on `Fetched::new`; `resolve` never offline | `did_resolution::resolve_is_fetch_then_accept_document` |

New ids (not in PLAN §6), from the block AMB-321..:

- AMB-321: "cannot be retrieved and validated" does not say how much of DID Core is checked. DID Core makes `verificationMethod` a set of verification-method maps (each with `id`, `type`, `controller`) and a verification relationship a set of references or embedded methods; those two members decide selection, so their shape is checked and a non-conforming shape fails resolution. Other DID Core properties are never read, so validating them would refuse documents without changing any outcome. `type` is required to be a string and not otherwise checked: the profile defines the key by `publicKeyJwk`, not by a method type.
- AMB-322: DID Core resolves relative DID URLs per RFC 3986 against the document `id`. For a root `did:web` DID, the only reference that can resolve to `signer#fragment` and is written in practice is `#fragment`, which RFC 3986 §5.2.2 resolves to `id#fragment`. Exotic forms such as `./web:example.com#k` would resolve there under full RFC 3986 merging; they are compared verbatim instead and so never match. Trade-off accepted: a document written that way fails selection with `NotFound`, and no second URL resolver exists.
- AMB-323: `check_public_jwk` takes raw JSON bytes and parses them with `serde_json`, which keeps the last of duplicate members. Handing it the original bytes would let a JWK be read one way here and another way there; the canonical bytes of the strictly read value have no duplicates.

## Divergences from ai-catalog-rust 0.4.0

| Id | ai-catalog-rust | This crate | Spec | Pinned by |
|---|---|---|---|---|
| RD-19 (DID half) | `ai_catalog_validate::verify` fetches the DID document through a caller `Fetch` returning bytes, so status, media type and Safe Fetching are the caller's | `client::fetch` under `FetchPolicy`; status 200 and a DID or JSON media type (AMB-80) | #did-document-resolution-and-key-selection "MUST satisfy the safe-fetching requirements" | `did_resolution::status_other_than_200_is_refused`, `did_resolution::non_did_media_type_is_refused` |
| RD-83 | `did::select_key` skips non-object `verificationMethod` items and treats a non-array member as empty | an invalid DID document (`Invalid { pointer }`) | #did-document-resolution-and-key-selection "cannot be … validated" | `did_resolution::malformed_verification_relationships_are_refused` |

Agreements: `id` = signer exactly; `#`-references resolved against the signer; candidates counted as here (a reference and its target once, an embedded duplicate twice); `assertionMethod` authorisation; `controller` = signer; `publicKeyJwk` required; the JWK rules of `P256Key::from_jwk` match `check_public_jwk`'s.

New id: RD-83 (block RD-81..), as stated in the table.

## Consequences

- `client::fetch` stays the one request path for catalogs, DID documents and artifacts (ADR-008). Trust `src/` has no `std::net` and no other HTTP client (`crypto_boundary`, ADR-013).
- Verification cost: one DID-document fetch per distinct signer per `verify` call (ADR-015), with no cache. A cache would be a new control and is not built.
- Plan drift recorded:
  - PLAN writes `select_key(&DidDocument, kid)`; the `kid` argument is ADR-012's `Kid`, so only a header-checked `kid` reaches selection.
  - `accept_document` needs the client's media-type essence rule: ADR-008 made `media_type_essence` public (its revision log), so no second media-type parser exists here.
  - `Kid::parse(&str, &DidWebRoot) -> Option<Kid>` is the one constructor of a `Kid` (`header::inspect` uses it), so selection tests and ADR-015 build a `Kid` only through the `kid_matches_signer` check.
  - `ResolveError::{Status, MediaType, Invalid}` and `KeySelectionError::{NotFound, JwkNotIJson}` are not named in PLAN; they are AMB-80, AMB-321 and AMB-323.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created ahead of T3 from PLAN §3 (ADR-014 row), §4, T3, D8, AMB-28, AMB-77, AMB-80, RD-19 and validator 0.3.4 `check_public_jwk`. AMB-321, AMB-322, AMB-323 and RD-83 added. |
| 2026-11-17 | Accepted at T3: `did_resolution.rs` (10) and `key_selection.rs` (25) green. `Kid::parse` added as the one `Kid` constructor; `media_type_essence` made public in the client (ADR-008). |
