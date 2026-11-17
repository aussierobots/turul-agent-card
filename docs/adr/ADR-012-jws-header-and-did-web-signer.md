# ADR-012: JWS protected header and `did:web` signer shape (trust crate)

- **Status:** Accepted at S14 close (every test file below is present and green, `crypto_boundary` shows no JWS or base64url decoding outside `turul-jwt-validator`, and no header or signer-shape check exists in `turul-ai-catalog`)
- **Date:** 2026-11-17
- **Implements:** under `crates/turul-ai-catalog-trust/tests/`: `jws_header.rs`, `did_web_signer.rs`, `urn_air.rs`, and the S14 half of `sdk_trust_interop.rs`. `crypto_boundary.rs` (ADR-013) keeps holding.
- **Depends on:** ADR-002 (`Signature`, `AiCatalog`, `TrustManifest`, `json::read`), ADR-005 (`validate`, the crate-private RFC 3986 `uri` grammar), ADR-006 (`validate_manifest`, `DID_WEB_V1`), ADR-013 (T2 signer output, `crypto_boundary`). Binding decisions: D6 as amended by M12 and M13, M10 (no local JWS cryptography), M14 (validator line), M15 (scope of M13). JWS API: `turul-jwt-validator` 0.3.4 `detached::{inspect_detached, verify_detached, DetachedJwsPolicy, DetachedJwsError, JwsAlg}` (`docs/plan/handoff-turul-jwt-validator.md`, "Agreed", released 2026-11-17).
- **Spec:** `target/upstream/ai-catalog/specification/ai-catalog.md` (last commit 2026-11-10). RFC 7515 (§4.1.11 `crit`, Appendix F detached content), RFC 7797 (`b64`), RFC 3986 (§3.5 fragment), DID Core, did:web method.

## Context

A Signature object's `jws` is "a detached compact JWS [[RFC7515]] … `encodedProtectedHeader..encodedSignature`" (#signature-object). Its protected header carries the rules a consumer must apply before any key is fetched: "The protected header MUST contain `alg` and `typ`", the `typ` per containing object, "Consumers MUST reject a missing or mismatched `typ`", "The `alg` value `none` MUST NOT be accepted", "MUST NOT contain `b64`" (#signed-payload-and-jws-construction), and under `did-web-v1` ES256 only, a `kid` of the form signer + `#` + fragment, and none of `jku`, `jwk`, `x5u`, `x5c` (#jws-algorithm-and-key-requirements). `did-web-v1` also fixes the signer's shape: "The Signature object's `signer` MUST be a root `did:web` DID. Its domain MUST be lowercase ASCII, with no port, IP address, trailing root dot, or Unicode U-label; IDNA A-labels are permitted" (#the-did-web-signer-profile). The Publisher Profile reuses those domain restrictions for the `urn:air` publisher component (#the-did-web-publisher-profile).

M12 places all of this in `turul-ai-catalog-trust` and routes the header through the validator, so turul has one base64url decoder and one header-policy implementation. M13 (narrowed by M15) makes every violation here a per-signature non-endorsement, never a document diagnostic. The validator's `inspect_detached` (0.3.4) runs the same private `check_header` path as `verify_detached` with no key and stops before the signature segment; it does not check `kid`.

Spec anchors implemented: #signature-object (`jws` stored form), #signed-payload-and-jws-construction (protected header), #the-did-web-signer-profile (signer shape), #jws-algorithm-and-key-requirements (header policy and `kid`), #the-did-web-publisher-profile (the `urn:air` publisher component and its domain rule; the authorization itself is ADR-015).

## Decision

### 1. Identity shapes (`identity` module)

- `DidWebRoot::parse(&str) -> Option<DidWebRoot>` is the one root-`did:web` DID parser (AMB-77), used for `signer` and for `TrustPolicy::operators` (ADR-015). It accepts exactly when the value starts with `did:web:` (exact, case-sensitive) and the rest satisfies `turul_ai_catalog::identity::is_root_did_web_domain` (ADR-004), the one domain rule, which the publisher component also uses (ADR-017). That rule accepts exactly when:
  1. the domain is 1 to 253 bytes;
  2. split on `.`, every label is 1 to 63 bytes of `[a-z0-9-]`, not starting or ending with `-` (so uppercase, `%3A` ports, a further `:` path, `/`, `?`, `#`, `[`, any non-ASCII U-label, an empty label and a trailing `.` are refused);
  3. the last label is not all digits and is not `0x` followed only by `[0-9a-f]` (the WHATWG host parser's "ends in a number" test, so `192.0.2.1`, `2130706433`, `127.1` and `0x7f.0.0.1` are IP spellings and refused; IPv6 needs `[`, refused by 2).
  `xn--` A-labels are accepted as LDH text and are not IDNA-validated (M6 for the format crate; the trust crate adds no IDNA dependency either, §0b names none). Accessors: `as_str()` (the whole DID), `domain()`.
- `UrnAir::parse(&str) -> Option<UrnAir>` is the one `urn:air` parser (AMB-78): exact, case-sensitive prefix `urn:air:`, then at least three `:`-separated non-empty components: `publisher()` (first), `name()` (last), `namespace()` (all between, `:` kept). The publisher is not checked here: one that fails `DidWebRoot::parse("did:web:" + publisher)` still parses and is never authorised (ADR-015).

### 2. Header policy (`header` module)

- Constants: `TYP_TRUST_MANIFEST = "ai-catalog-trust-manifest+jws"`, `TYP_CATALOG = "ai-catalog+jws"` (#signed-payload-and-jws-construction table), `ES256: JwsAlg = JwsAlg::Es256`, `DID_WEB_V1_FORBIDDEN_HEADERS = ["jku", "jwk", "x5u", "x5c"]`.
- `header::policy(SignedObject) -> DetachedJwsPolicy<'static>` is the one policy builder: `expected_typ` = `TYP_TRUST_MANIFEST` for `SignedObject::Manifest` and `TYP_CATALOG` for `SignedObject::Catalog` (ADR-015 owns `SignedObject`), `allowed_algs = [ES256]`, `forbidden_params = DID_WEB_V1_FORBIDDEN_HEADERS`. ADR-012's `inspect` and ADR-015's `verify_detached` call both take it from here; no other `DetachedJwsPolicy` is built in the workspace.
- What the validator enforces through that policy, as released (each with its own `DetachedJwsError` variant): anything but `header..sig` with an empty payload segment, a padded or non-alphabet header segment (`Malformed`); a header that is not a JSON object, has a duplicate member or lacks a string `alg` (`InvalidHeader`); `alg: "none"` (`AlgNone`); any other `alg` but `ES256` (`AlgNotAllowed`); any `b64` (`B64Present`); any `crit` (`CritPresent`, AMB-29: `did-web-v1` defines no extension parameter, so none is understood, RFC 7515 §4.1.11); a forbidden parameter (`ForbiddenParam(name)`); a missing, non-string or different `typ`, compared byte-exact and case-sensitive (`TypMismatch`). Order and variants are the validator's (AMB-301).

### 3. `header::inspect`

`header::inspect(object: SignedObject, signature: &Signature) -> Result<Kid, HeaderError>`:

1. `turul_jwt_validator::detached::inspect_detached(&signature.jws, &header::policy(object))`; an error is `HeaderError::Jws(DetachedJwsError)`, the validator's value unchanged.
2. `kid` is read from the returned header map through inherent methods only (`get("kid")`, then `as_str()`), so no `serde_json` type is named in trust `src/` (§2 rules). A missing or non-string `kid` (AMB-302), or one for which `kid_matches_signer` is false, is `HeaderError::KidMismatch`. The signer is `DidWebRoot::parse(&signature.signer)`; `inspect` is called only after that succeeds (ADR-015 order), and a signer that does not parse is `HeaderError::KidMismatch` when `inspect` is called directly.
3. `Ok(Kid)`. `Kid` is the one owner of a checked `kid` value: constructed only here, `as_str()`, `fragment()`.

`kid_matches_signer(kid: &str, signer: &DidWebRoot) -> bool` (AMB-79): `kid` is exactly `signer.as_str()`, then `#`, then a non-empty string matching the RFC 3986 `fragment` rule (`*( pchar / "/" / "?" )`, percent-encodings well-formed). Because the signer is an exact prefix, no path or query can precede `#`; `/` and `?` after it are fragment characters (RFC 3986 §3.5), not a path or query (RD-82). The fragment grammar is the format crate's one RFC 3986 implementation (ADR-005 `uri`); S14 makes exactly that predicate public rather than writing a second grammar here.

`Ok` from `inspect` says nothing about the signature segment (handoff "Agreed"); only `verify_detached` checks it (ADR-015).

### 4. Scope: `did-web-v1` only

These rules are the `did-web-v1` rules. ADR-015 applies them only after selecting exactly `did-web-v1`; any other profile is `Reason::Unsupported` before the header is read, so a header is never judged under the wrong profile (#the-did-web-signer-profile: "MUST NOT silently apply these rules to a different mechanism"), and such a signature never counts as an endorsement (#profile-selection).

### 5. Non-endorsement, never a document diagnostic (M12, M13)

A header or signer-shape violation is `Reason::Header(HeaderError)` or `Reason::SignerShape` on that one signature (ADR-015). The format crate decodes nothing, reports nothing about a header, and `validate` of a document whose only defect is a header or signer-shape violation is unchanged (#verification-result: "Failure MUST NOT be treated as a verified endorsement. Consumers MAY retain or display unverified content"; #level-3-trusted-catalog: "Unverified additional contributions do not by themselves invalidate an independently verified publisher endorsement").

Trade-offs, accepted:

1. (M12) One decoder and one header policy, both the validator's. Header checks wait on the 0.3.4 release, like T2; the format crate has no header diagnostic.
2. No header outcome exists for a signature under an unsupported profile. The consumer MUSTs of #signed-payload-and-jws-construction ("Consumers MUST reject a missing or mismatched `typ`", "`none` MUST NOT be accepted") hold because such a signature is never accepted at all.
3. (M13, replacing D6's "as `validate` errors") A violation never makes a document `NotMet`, fails lambda cold start or makes CLI `validate` exit 1.
4. Validator types (`DetachedJwsPolicy`, `DetachedJwsError`, `JwsAlg`) appear in the trust crate's public API. A validator major release is a trust-crate break; the crate is `publish = false` (D11), so only this workspace is affected.

The producer MUSTs (#the-did-web-signer-profile, #jws-algorithm-and-key-requirements: "Producers … MUST use ES256") bind our signer: every T2 output passes `header::inspect`.

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Root `did:web` identity (signer, operator, publisher DID) | `identity::DidWebRoot::parse`, whose domain rule is `turul_ai_catalog::identity::is_root_did_web_domain` | itself; ADR-015 `verify_signature`, `publisher_authorized`, `TrustPolicy::operators` |
| `urn:air` components | `turul_ai_catalog::identity::UrnAir::parse` (moved to the core crate, M24; owned by ADR-004) | ADR-015 `publisher_authorized` |
| Header policy | `header::policy(SignedObject)` | `header::inspect`; ADR-015 `verify_signature` (`verify_detached`) |
| Header check (structure, `alg`, `typ`, `b64`, `crit`, forbidden members) | `turul_jwt_validator::detached::inspect_detached` through `header::policy` | `header::inspect` |
| Checked `kid` | `header::Kid`, built only by `header::inspect` | ADR-014 `select_key` |
| `kid` form | `header::kid_matches_signer` (fragment grammar: ADR-005 `uri`) | `header::inspect`; ADR-013 signer |
| Errors | `header::HeaderError::{Jws(DetachedJwsError), KidMismatch}` (`#[non_exhaustive]`) | — |

No derived artifact or golden belongs to this ADR (D17 applies to ADR-003, ADR-009, ADR-013).

## Requirement → test

Test paths are under `crates/turul-ai-catalog-trust/tests/`. Ids number the spec's requirement statements per section as PLAN §4 does: keywords joined by "and" in one sentence with one subject are one statement.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| signed-payload-and-jws-construction-6 | "The protected header MUST contain `alg` and `typ`" | `jws_header::header_without_alg_is_invalid_header`, `jws_header::header_without_typ_is_typ_mismatch` |
| signed-payload-and-jws-construction-7 | "The `typ` value MUST exactly match the kind of containing object" | `jws_header::manifest_typ_on_catalog_root_is_typ_mismatch`, `jws_header::catalog_typ_on_manifest_is_typ_mismatch`, `jws_header::typ_is_compared_case_sensitively` (`AI-CATALOG+JWS`) |
| signed-payload-and-jws-construction-8 | "Consumers MUST reject a missing or mismatched `typ`" | the three `typ` tests above; replay across kinds through a duplicate `typ`: ADR-015 `signature_acceptance::duplicate_typ_cannot_replay_across_kinds` |
| signed-payload-and-jws-construction-9 | "The `alg` value `none` MUST NOT be accepted" | `jws_header::alg_none_is_refused` |
| signed-payload-and-jws-construction-10 | "The protected header MUST NOT contain `b64`" | `jws_header::b64_is_refused_whatever_its_value` (`false` and `true`) |
| jws-algorithm-and-key-requirements-1 | "Producers … MUST use ES256" | `jws_header::signer_output_passes_inspect` (every `sign_manifest` / `sign_catalog` output, ADR-013; header exactly `{alg: ES256, typ, kid}`) |
| jws-algorithm-and-key-requirements-3 | "The protected JWS header MUST contain `kid`" | `jws_header::missing_kid_is_kid_mismatch`, `jws_header::non_string_kid_is_kid_mismatch` |
| jws-algorithm-and-key-requirements-4 | "MUST NOT contain `jku`, `jwk`, `x5u`, or `x5c`" | `jws_header::jku_is_forbidden`, `jws_header::jwk_is_forbidden`, `jws_header::x5u_is_forbidden`, `jws_header::x5c_is_forbidden` |
| jws-algorithm-and-key-requirements-5 | "`kid` MUST be … the exact `signer` value followed by a non-empty fragment, with no path or query" | `jws_header::kid_of_another_did_is_kid_mismatch` (`did:web:other.example#k`), `jws_header::kid_with_empty_fragment_is_kid_mismatch` (signer + `#`), `jws_header::kid_with_path_is_kid_mismatch` (signer + `/p#k`), `jws_header::kid_fragment_follows_rfc_3986` (`#k/1?x` and `#%41` accepted; `#k#2`, `#k k` and `#%zz` refused) |
| the-did-web-signer-profile-1 | "`signer` MUST be a root `did:web` DID" | `did_web_signer::path_based_did_web_is_refused` (`did:web:example.com:user`), `did_web_signer::other_methods_and_prefix_spellings_are_refused` (`did:key:z6Mk…`, `DID:web:example.com`, `did:WEB:example.com`, `did:web:`) |
| the-did-web-signer-profile-2 | "domain MUST be lowercase ASCII, with no port, IP address, trailing root dot, or Unicode U-label; IDNA A-labels are permitted" | `did_web_signer::uppercase_domain_is_refused` (`did:web:Example.com`), `did_web_signer::port_is_refused` (`did:web:example.com%3A8443`), `did_web_signer::ip_address_spellings_are_refused` (`192.0.2.1`, `2130706433`, `127.1`, `0x7f.0.0.1`), `did_web_signer::trailing_dot_is_refused`, `did_web_signer::u_label_is_refused` (`bücher.example`), `did_web_signer::a_label_and_digit_led_label_are_accepted` (`xn--bcher-kva.example`, `a1.example`, `1a.example`), `did_web_signer::label_and_domain_length_limits` (63/64-byte label, 253/254-byte domain) |

Decisions owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| RD-15 | header rules only in the trust crate, through `inspect_detached`, only under `did-web-v1`; no document diagnostic | `jws_header::header_defects_are_not_document_diagnostics` (each negative header below, in a catalog: `validate` reports nothing new and conformance stays `Met`); interop row in `sdk_trust_interop` (S14 half) |
| RD-16 (shape half) | `did:web` shape violations are `Reason::SignerShape`, never a document diagnostic | `did_web_signer::signer_shape_defects_are_not_document_diagnostics` |
| RD-81 | IP spellings beyond dotted quads are refused | `did_web_signer::ip_address_spellings_are_refused`; interop `sdk_trust_interop::did_web_ip_spellings_are_refused_here_only` |
| RD-82 | `kid` fragment is RFC 3986 `fragment`, `/` and `?` included | `jws_header::kid_fragment_follows_rfc_3986`; interop `sdk_trust_interop::kid_fragment_with_slash_is_accepted_here_only` |
| AMB-29 | any `crit` is refused; a `kid` mismatch fails verification | `jws_header::any_crit_is_refused` (`["exp"]`, `[]`, `"x"`); `jws_header::kid_of_another_did_is_kid_mismatch` |
| AMB-66 (header half) | the two spec-example placeholder `jws` values pass `header::inspect` | `jws_header::spec_example_jws_values_pass_inspect` (`did:web:acme.com#release-signing-key` and `did:web:acme-corp.com#release-signing-key`, both `typ` `ai-catalog-trust-manifest+jws`); that they never verify is ADR-015 |
| AMB-77 | root `did:web` domain rule (§1) | the `did_web_signer` functions above |
| AMB-78 | `urn:air` needs publisher, namespace and name; exact prefix | `urn_air::components_split_into_publisher_namespace_and_name` (`urn:air:example.com:finance:agent:billing` → `example.com`, `finance:agent`, `billing`), `urn_air::fewer_than_three_components_is_none` (`urn:air:example.com:billing`), `urn_air::empty_component_is_none` (`urn:air:example.com::billing`), `urn_air::prefix_is_case_sensitive` (`URN:AIR:example.com:a:b`), `urn_air::publisher_failing_the_domain_rule_still_parses` (`urn:air:Example.com:a:b`) |
| AMB-79 | `kid` = signer + `#` + non-empty RFC 3986 fragment | the `kid_*` tests above |
| AMB-301 | order and variants of header defects are the validator's; each negative test has one defect | `jws_header::policy_is_es256_with_kind_typ_and_forbidden_headers` (asserts `policy(Manifest)` and `policy(Catalog)` field by field), and one test per variant: `jws_header::non_detached_forms_are_malformed` (`h.p.s`, `h..s..`, `h`), `jws_header::padded_or_non_alphabet_header_is_malformed`, `jws_header::duplicate_header_member_is_invalid_header`, `jws_header::non_object_header_is_invalid_header`, `jws_header::alg_outside_allowlist_is_refused` (`ES384` → `AlgNotAllowed`) |
| AMB-302 | a non-string `kid` is `KidMismatch` | `jws_header::non_string_kid_is_kid_mismatch` |

Out of scope here, with owner: the signature segment and the JWK (validator, through ADR-015 `verify_detached` and ADR-014 `select_key`); key lookup by `kid` (ADR-014); acceptance (ADR-015).

New ids (not in PLAN §6), taken from the ranges RD-81.. / AMB-301.., which no other ADR used on 2026-11-17 (ADR-004 to ADR-011 were being written concurrently):

- RD-81: ai-catalog-validate 0.4.0 `identity::is_valid_publisher` refuses an IP address only when `str::parse::<IpAddr>` accepts it (dotted quads and IPv6 text), so `did:web:2130706433`, `did:web:127.1` and `did:web:0x7f.0.0.1` pass its root-`did:web` check and its publisher check. The spec refuses any "IP address"; AMB-77 uses the rule the client's `url` host parser applies, under which those spellings are IPv4 addresses.
- RD-82: ai-catalog-validate 0.4.0 (`profile.rs`, `verify.rs`) refuses a `kid` fragment containing `/` or `?` and accepts one with a space. RFC 3986 §3.5 allows `/` and `?` in a fragment and forbids a space; the spec's "no path or query" is already guaranteed by the exact signer prefix.
- AMB-301: #signed-payload-and-jws-construction lists the header rules without an order. The validator's order decides which variant a header with several defects gets; this ADR pins one variant per single-defect header and claims nothing for combinations.
- AMB-302: a `kid` that is a JSON number, object or `null` cannot be "an absolute DID URL"; it is treated as a mismatch ("A mismatch MUST fail verification under this profile").

## Divergences from ai-catalog-rust 0.4.0

| Id | ai-catalog-rust | This crate | Spec | Pinned by |
|---|---|---|---|---|
| RD-15 | `ai_catalog_validate` checks JWS structure for every profile (warnings); `ai_catalog_trust::verify_es256` refuses `crit` and `b64` only at verify | header rules only under `did-web-v1`, in the trust crate, through `inspect_detached` before any key work | #signed-payload-and-jws-construction; #profile-selection | `jws_header::header_defects_are_not_document_diagnostics`, `sdk_trust_interop` |
| RD-16 (shape) | `did:web` signer-shape violations are validation warnings (`Code::ProfileSigner`) | `Reason::SignerShape`, a non-endorsement; no document diagnostic | #the-did-web-signer-profile MUST | `did_web_signer::signer_shape_defects_are_not_document_diagnostics` |
| RD-81 | `2130706433`, `127.1`, `0x7f.0.0.1` accepted as domains | refused | #the-did-web-signer-profile "no … IP address" | `sdk_trust_interop::did_web_ip_spellings_are_refused_here_only` |
| RD-82 | `kid` fragment with `/` or `?` refused, with a space accepted | RFC 3986 `fragment` | #jws-algorithm-and-key-requirements; RFC 3986 §3.5 | `sdk_trust_interop::kid_fragment_with_slash_is_accepted_here_only` |

Agreements: `typ`, `alg` ES256 only, `none`, the four forbidden members and the `urn:air` publisher prefix match ai-catalog-validate's verify path; RD-25 (no `urn:air` arity check there) is decided by AMB-78 and pinned in ADR-015.

## Consequences

- `turul-ai-catalog` stays free of JWS and base64url (M12). In trust `src/`, JWS text reaches only `inspect_detached`, `verify_detached` and `sign_detached_es256`; `crypto_boundary` (ADR-013) scans for a second path.
- A header defect is never a document error, so a catalog with a broken optional signature still serves and validates (S10, S12). The cost is that no tool in this workspace reports a header defect outside `TrustReport`.
- Plan drift recorded:
  - PLAN S14 names a unit test `header::tests::policy_matches_adr_012`; adr-review check 2 forbids a function named after an ADR. It is `jws_header::policy_is_es256_with_kind_typ_and_forbidden_headers`, an integration test on the public `policy`.
  - PLAN §2 lists the format crate's `uri` module as crate-private; `kid_matches_signer` needs its RFC 3986 `fragment` rule, so S14 makes that one predicate public (ADR-005's revision log records it) instead of adding a second grammar.
  - PLAN's ADR-012 row and S14 criterion say a duplicate header member is refused "with the validator's own variant"; as released that variant is `InvalidHeader`, which this ADR pins.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created ahead of S14 from PLAN §3 (ADR-012 row), §4, S14, AMB-29, AMB-66, AMB-77 to AMB-79, RD-15, RD-16 and the released validator 0.3.4 `detached` module. RD-81, RD-82, AMB-301 and AMB-302 added. `policy_matches_adr_012` renamed per adr-review check 2. |
| 2026-11-17 | S14 landed: `identity`, `header` and `object` modules, the three test files and the two `sdk_trust_interop` rows. `SignedObject` is defined in `object` (ADR-015 reuses it). `TYP_*` moved from `sign` to `header`, and `sign` now takes its `kid` fragment check from the format crate's public `uri::is_fragment` (one grammar). Added tests `inspect_returns_a_checked_kid` and `unparsable_signer_is_kid_mismatch`. |
| 2026-11-17 | `Kid::parse(&str, &DidWebRoot) -> Option<Kid>` added (T3): the one constructor of a `Kid`, applying `kid_matches_signer`; `header::inspect` builds its `Kid` through it. |
| 2026-11-17 | The ADR-013 signer now builds its `kid` through `Kid::parse` and takes its signer as `DidWebRoot`, so the owner-table row "`kid` form … ADR-013 signer" holds: `sign`'s second `kid` rule (non-empty signer without `#`, then `is_fragment`) is deleted. |
| 2026-11-17 | M24: `UrnAir` moved to `turul_ai_catalog::identity` (ADR-004) so consumers that only build and validate catalogs can parse `urn:air` without the trust crate. The trust crate imports it from core and does not re-export it. The grammar and its tests (`urn_air.rs`, now in the core crate) are unchanged. |
| 2026-11-17 | ADR-017 R3: the domain rule of `DidWebRoot::parse` moved to `turul_ai_catalog::identity::is_root_did_web_domain` (ADR-004), which `parse` calls after stripping `did:web:`; behaviour unchanged, `did_web_signer.rs` stays green. Storage's `WriterGrant::new` calls the same function, so there is one definition. |
