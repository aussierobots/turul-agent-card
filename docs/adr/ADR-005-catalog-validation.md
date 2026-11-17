# ADR-005: Catalog validation

- **Status:** Accepted (2026-11-17)
- **Date:** 2026-11-17
- **Implements:** under `crates/turul-ai-catalog/tests/`: `spec_version_format.rs`, `entry_uniqueness.rs`, `extension_keys.rs`, `digest_format.rs`, `timestamps.rs`, `nested_inline_catalogs.rs`, `logo_url_privacy.rs`, `number_diagnostics.rs`, `noncharacter_diagnostics.rs`, `conformance_level.rs`; the diagnostic halves of `artifact_types.rs` (`type-empty`, ADR-004) and `artifact_source.rs` (`url-empty`, RD-02).
- **Depends on:** ADR-001 (corpus, declared level D2), ADR-002 (`parse`, `parse::document`, `ParseError`, `JsonPointer`), ADR-003 (`AiCatalog::to_json`, `number::to_canonical`, `json::is_noncharacter`), ADR-004 (`EntryKey`, `ArtifactType::AI_CATALOG`, `HostIdentifier`). Binding decisions: M5, M6 (no runtime dependency, so RFC 3339 and RFC 3986 are hand-rolled), M7 (D2), M9.
- **Spec:** https://ai-catalog.io, `specification/ai-catalog.md` as read in ADR-001 (last commit 2026-11-10).

## Context

`parse` (ADR-002) accepts every document of the right CDDL shape and stops at the first shape defect. The spec also has content rules that the CDDL cannot express: uniqueness of entries, extension-key form, digest strength, timestamp syntax, the I-JSON payload rule, nesting depth and the conformance levels. This ADR adds one validator that judges a parsed document, reports every defect with a JSON Pointer, and names the conformance level the document meets. The client (ADR-008, ADR-009), the lambda (ADR-010) and the CLI (ADR-011) read its `Report`; none of them judges a document itself.

Spec anchors implemented: #version-format, #multi-version-entries, #catalog-entry (`identifier`, `type`, `url`, `version`, `updatedAt`, `digest`, `data`), #resolving-an-artifact-s-version (the multi-version REQUIRED), #format-and-key-naming, #official-extensions, #policy-urls-extension, #nested-catalog-entries, #nested-catalog-depth-and-circular-references (depth), #digest-format, #privacy-considerations, #conformance-levels, #level-1-minimal-catalog, #level-2-discoverable-catalog, #level-3-trusted-catalog (graceful absence), #signed-payload-and-jws-construction (I-JSON).

Manifest-scope and signature-scope checks (`contributor`, `subject`, `signature` members, `issuedAt`/`expiresAt`, manifest `extensions`, subject, attestation and provenance digests) belong to `validate_manifest` (ADR-006, S5), which `validate` calls from S5 on. It uses this ADR's `DigestValue::parse`, `Timestamp`, extension-key rule and `Report`; it adds no second copy of any of them.

## Decision

### 1. Entrypoints and report

- `validate::validate(&AiCatalog) -> Report` is `validate_at(catalog, NestingDepth::ROOT)`.
- `validate::validate_at(&AiCatalog, NestingDepth) -> Report` validates one document that sits at the given depth. The client calls it for a catalog it fetched through a nested `url` (ADR-009). It is the one validator; no other crate judges document content.
- `Report` has private fields and two readers: `diagnostics() -> &[Diagnostic]` and `conformance() -> Conformance`. The conformance is computed once, from the diagnostics and the root `host`, when the report is built, so the two can never disagree. `has_errors() -> bool` is true when any diagnostic has `Severity::Error`.
- `Diagnostic { severity: Severity, code: Code, path: JsonPointer }` (`#[non_exhaustive]`, public fields). `path` is the RFC 6901 pointer of the offending value in the document that was passed to `validate` (§6 for nested documents). `Severity::{Error, Warning}`. `Code` is a `#[non_exhaustive]` enum; `Code::as_str()` gives the kebab-case name in §2 and `Display` writes it.
- Diagnostics come in a fixed order: document checks (§3, §4 on the root `extensions`), then each entry in array order (its own checks, then its inline nested catalog's diagnostics), then uniqueness (§3), then the I-JSON walk (§5) in document order. The same document always gives the same report.
- `validate` never fails and never panics. It never fetches, never decodes `jws` (M12), and never changes the document.

### 2. Codes

| Code | Severity | Path | Rule |
|---|---|---|---|
| `spec-version-noncanonical` | Warning | `/specVersion` | §3 |
| `identifier-empty` | Error | `/entries/i/identifier` | §3 |
| `type-empty` | Error | `/entries/i/type` | §3 |
| `url-empty` | Error | `/entries/i/url` | §3 |
| `entry-duplicate` | Error | `/entries/j` | §3 |
| `version-missing-in-group` | Error | `/entries/i` | §3 |
| `timestamp-invalid` | Error | `/entries/i/updatedAt` | §3 |
| `digest-invalid` | Error | `/entries/i/digest` | §7 |
| `digest-weak` | Error | `/entries/i/digest` | §7 |
| `digest-algorithm-unknown` | Warning | `/entries/i/digest` | §7 |
| `extension-key-invalid` | Error | `…/extensions/<key>` | §4 |
| `policy-urls-invalid` | Error | the policy-urls value, or its member | §4 |
| `logo-url-not-data-uri` | Warning | `/host/logoUrl` | §3 |
| `nested-catalog-invalid` | Error | the `ParseError` pointer, under `/entries/i/data` | §6 |
| `nested-depth-limit` | Warning | `/entries/i/data` | §6 |
| `number-not-i-json` | Error in a signed context, otherwise Warning | the number | §5 |
| `number-inexact` | Warning | the number | §5 |
| `string-not-i-json` | Error in a signed context, otherwise Warning | the string, or the member whose name holds the noncharacter | §5 |

### 3. Document and entry checks

- **Version format.** `parse` has already gated `specVersion` (ADR-002 §2). A component with a leading zero (`01.0`, `1.00`) gets `spec-version-noncanonical`. #version-format says the components "are non-negative integers", and `01` still denotes one, so the value is valid but not in the form every example uses (AMB-44). `1.10` is canonical.
- **Empty identity values.** `identifier: ""` gives `identifier-empty` and `type: ""` gives `type-empty` (AMB-69, AMB-06, AMB-99). An identifier without `:` and an unknown or reserved `type` give nothing (#catalog-entry "open text format"; RD-10, AMB-19).
- **Empty `url`.** `url: ""` is present (RD-02) and gets `url-empty`: an empty string is not a reference to anything, so the entry has no retrievable artifact. Other `url` values are not checked here; resolving them is ADR-008.
- **Uniqueness** (#multi-version-entries: "When `version` is present, the combination of `identifier` and `version` MUST be unique within the catalog. When `version` is absent, `identifier` alone MUST be unique"; #catalog-entry `version`: "REQUIRED when a catalog lists multiple versions of the same `identifier`"). Entries are grouped by `CatalogEntry::key()` (ADR-004), never by rebuilt strings.
  - Each entry whose key equals an earlier entry's key gets `entry-duplicate` at its own pointer. The first occurrence gets nothing.
  - In an identifier group that holds at least one entry with `version` and at least one without, each entry without `version` gets `version-missing-in-group` (RD-09). Its identifier is not "alone unique", and the group lists versions of that identifier.
  - `version: ""` is a present version (AMB-70): `(id, "")` and `(id, "1")` are a valid pair, and `(id, "")` with `(id, absent)` is a mixed group.
  - Several entries with one identifier and distinct versions give nothing (#multi-version-entries "MAY contain multiple entries with the same `identifier`").
  - Scope is one document. An inline nested catalog has its own scope (§6), so an entry there may reuse a parent's identifier (#nested-catalog-entries "MAY reuse the same `identifier`"; AMB-46).
- **`updatedAt`.** Must parse as `Timestamp` (§8), otherwise `timestamp-invalid` (AMB-64). `""` is present and invalid (RD-50).
- **Logo.** `host.logoUrl` whose scheme is not `data` (compared without regard to case, RFC 3986 §3.1) gets `logo-url-not-data-uri` (#privacy-considerations: "Logo URLs SHOULD use Data URIs"; AMB-62). A SHOULD gives a warning, never an error. The Data URI's own syntax is not checked: no consumer here renders it.

### 4. Extensions

- **Key form** (#format-and-key-naming: "the keys MUST be a valid URL or a reverse-DNS string"; AMB-47). The crate-private `extensions::is_valid_key(&str)` is the one rule, applied here to the root and entry `extensions` and by `validate_manifest` to manifest `extensions` (ADR-006):
  - A key containing `://` is judged as a URL: it must match RFC 3986 `URI` (§3, fragment allowed) with a `hier-part` that starts with `//` (an authority). `https://openknowledgeformat.org/ns#` is valid; `https://` and `https://a b/` are not.
  - Any other key is judged as reverse-DNS: two or more `.`-separated non-empty labels of `[A-Za-z0-9_-]`, the first starting with an ASCII letter. `com.example.confidenceScore` and `io.acme.deploymentRegion` are valid; `example`, `.com.x`, `com..x`, `1com.x` and `urn:x` are not.
  - A violation gets `extension-key-invalid` at the pointer of that member (`/entries/0/extensions/com~1x` style escaping). Parsing never fails on a key (AMB-47).
- **Unknown keys** (#format-and-key-naming: "Consumers that do not recognize an extension key MUST ignore it without throwing an error"). A valid key this crate does not know gets nothing, and its value is never read.
- **Metadata** (`https://ai-catalog.org/extensions/metadata`, #official-extensions: "schemaless"). Its value is never checked (AMB-48), so no constant is defined for it.
- **Policy URLs** (#policy-urls-extension). `extensions::POLICY_URLS` is the key. `CatalogEntry::policy_urls(&self) -> Result<Option<PolicyUrls<'_>>, PolicyUrlsError>` is the one reader: `Ok(None)` when absent, `Ok(Some(PolicyUrls { privacy_policy_url: Option<&str>, terms_of_service_url: Option<&str> }))` when valid. A value that is not an object ("Its value MUST be a JSON object") gives `PolicyUrlsError::NotObject`, and `privacyPolicyUrl` or `termsOfServiceUrl` present but not a string gives `PolicyUrlsError::MemberNotString(name)`. `validate` turns either into `policy-urls-invalid` at the value or member pointer. `{}` is valid ("The following members are OPTIONAL"). The URL strings are not scheme-checked: the spec says "a URL" and nothing here fetches them (AMB-49; fetching is #safe-fetching, ADR-009). Other members of the object are unknown and ignored. The extension is recognised only in `entry.extensions` ("It is used in `entry.extensions`"); under the root or a manifest it is a valid URL key like any other (AMB-49).

### 5. I-JSON numbers and strings

#signed-payload-and-jws-construction: "The payload MUST satisfy JCS's I-JSON constraints … Large integers that cannot round-trip as IEEE 754 doubles SHOULD be encoded as strings." #catalog-entry `digest`: "Inline `data` MUST satisfy JCS's input requirements for artifact verification to succeed."

- One walk over `AiCatalog::to_json()` per document (AMB-151). `to_json` is the one model → `Json` mapping (ADR-003), so the walk sees every number and string the writer would see, including unknown members, extension values, `additionalSignatures`, inline `data` and the typed `size`.
- Numbers are judged by the crate-private `number::defect(lexeme)`, on `number::to_canonical` (ADR-003 §3, M20), so the validator and the writer share one rule. `Defect::NotIJson`, a number that is not a finite double (`1e400`, `-1e400`, `1.8e308`), which the writer refuses, gives `number-not-i-json` (the MUST: JCS cannot write it). `Defect::Inexact`, a number whose JCS form denotes another value (`2^53+1`, `2^64+1`, a 25-digit integer, `333333333.33333329`, `1e-400`), gives `number-inexact`, always a Warning (the SHOULD: RFC 7493 §2.2 "SHOULD NOT"; #signed-payload-and-jws-construction "SHOULD be encoded as strings"). The writer writes it in that form, and a conformant producer signs or digests the same bytes, so it is a Warning in a signed context too. `1.0`, `1e2` and `0.1` give nothing. A string value or member name fails when it holds a character for which `json::is_noncharacter` is true (ADR-002 AMB-93).
- **Signed context** (AMB-09, AMB-30): the failing value lies inside an object that carries `signature` (the root, when the catalog is signed, so the whole document is the snapshot payload; or a trust manifest with `signature`), or inside the `data` of an entry with `digest`. There `number-not-i-json` and `string-not-i-json` are Errors, because the signature or digest cannot be computed over that value. Everywhere else it is a Warning: an unsigned document with a large integer is valid JSON and conformant, and the SHOULD asks the producer to change it.
- The validator never rewrites a number as a string (signed-payload-and-jws-construction-5 is the producer's choice) and never changes the model; the JCS form is the writer's (ADR-003).

### 6. Nested inline catalogs

#nested-catalog-entries: an entry whose `type` is `application/ai-catalog+json` "embeds (via `data`) another AI Catalog document"; "Clients processing nested catalogs SHOULD impose a maximum nesting depth … A depth limit of 4 is RECOMMENDED. Implementations MAY support deeper nesting but SHOULD document their limit." #nested-catalog-depth-and-circular-references repeats the limit.

- Detection: `entry.artifact_type == ArtifactType::AI_CATALOG` (exact, AMB-98) and `entry.source` is `ArtifactSource::Data`. Any other entry's `data` stays opaque (#design-goals 1).
- **Depth** (AMB-45, WG ADR 0001). `NestingDepth` is a `Copy` newtype with a private `u8`. `NestingDepth::ROOT` is 1, `get() -> u8` reads it, and `child(self) -> Option<NestingDepth>` is `None` when `self` is already `MAX_NESTING_DEPTH` (`pub const MAX_NESTING_DEPTH: u8 = 4`). Each nested catalog, inline here or by `url` in the client, is one level deeper. The rustdoc of `NestingDepth` and `MAX_NESTING_DEPTH` states the limit, as the SHOULD asks. No deeper limit is offered: no consumer needs one.
- A nested entry in a document at depth 4 is not descended. It gets `nested-depth-limit` (a Warning, because the limit is a client safeguard and the document breaks no rule), and the report's conformance is `Undetermined`, because content was left unvalidated. The I-JSON walk (§5) still covers its `data`, which is part of the document's bytes.
- Otherwise the `data` value goes through the crate-private `parse::catalog(&Json, &JsonPointer)`: the same object check, version gate and shape mapping `parse` runs after reading (ADR-002 §2, §3; AMB-152). Its pointer base is `/entries/i/data`, so a `ParseError` already carries the full pointer. A `ParseError` gives exactly one `nested-catalog-invalid` at that pointer (fail fast, FR-12, AMB-46); this includes a nested `specVersion` with major ≠ 1.
- A nested document that maps is validated by `validate_at(&nested, depth.child())` with its own uniqueness scope, and every diagnostic in its report is appended with the `/entries/i/data` prefix. `host` is not required in a nested catalog (AMB-46). The nested document's I-JSON walk is skipped, because the parent's walk already covered those bytes, with the parent's signed context.
- Inline nesting is a tree of values, so it cannot cycle. Cycle tracking for `url` nesting is ADR-009.

### 7. Digests

#digest-format: "`algorithm:hex-value`, where `algorithm` is a hash algorithm identifier and `hex-value` is the lowercase hexadecimal encoding of the hash output … Consumers MUST reject digest values using algorithms shorter than SHA-256."

`digest::DigestValue::parse(&str) -> Result<DigestValue, DigestError>` is the one parser for every digest field: entry `digest` here; subject `digest`, attestation `digest` and provenance `sourceDigest` in ADR-006; computation and comparison in the trust crate (ADR-013). `DigestValue` has `algorithm() -> DigestAlgorithm` (`Sha256`, `Sha384`, `Sha512`, `Other(String)`, `#[non_exhaustive]`) and `hex() -> &str`. `DigestError::{Invalid, Weak}`. The rules, in this order (AMB-23, AMB-153):

1. Split at the first `:`. The algorithm must match `component (separator component)*` with `component = [A-Za-z0-9]+` and `separator = [+._-]` (the OCI digest grammar, with letters of either case). The hex must be non-empty, of even length, and only `[0-9a-f]`. Otherwise `Invalid`. Uppercase hex is `Invalid` ("lowercase hexadecimal"; RD-13). `""` is `Invalid` (RD-50).
2. Hex shorter than 64 characters (output under 256 bits) is `Weak`, whatever the name: `md4:<32 hex>`, `sha1:<40 hex>`, `blake2s:<62 hex>`, `sha256:<32 hex>`.
3. `sha256`, `sha384` and `sha512`, compared without regard to case, must have hex length 64, 96 and 128; otherwise `Invalid` (`sha512:<64 hex>`). `SHA256:<64 lowercase hex>` is valid.
4. Any other name is `Ok(DigestAlgorithm::Other)`, and `validate` adds `digest-algorithm-unknown` (a Warning: the output is long enough, so the MUST does not reject it, but no consumer here can compute it; `sha3-256:<64 hex>`).

`validate` maps `Invalid` to `digest-invalid` and `Weak` to `digest-weak`. "Producers SHOULD use SHA-256 or stronger" (digest-format-1): every value this crate accepts without a diagnostic is at least 256 bits, and the trust crate's producer computes SHA-256 (ADR-013).

### 8. Timestamps

`timestamp::Timestamp` is the one RFC 3339 instant (AMB-64, AMB-154). `Timestamp::parse(&str) -> Result<Timestamp, TimestampError>` accepts RFC 3339 §5.6 `date-time` exactly:

- `T` and `Z` in either case (§5.6 NOTE). A space separator is refused (the ABNF has none).
- Day ranges by month, with Gregorian leap years (§5.7). Hours 00–23, minutes 00–59, seconds 00–60; `:60` is accepted as a leap second (§5.6 `time-second`) and denotes the same instant as the next second, as POSIX time does.
- Fractional seconds of any length; precision is the nanosecond, and further digits are truncated.
- Offsets `±hh:mm` with hours 00–23 and minutes 00–59; `-00:00` (§4.3) is the same instant as `Z`.

`Timestamp` implements `Eq` and `Ord` by instant, so `2026-01-01T10:00:00+10:00 == 2026-01-01T00:00:00Z`. `Timestamp::from_system_time(SystemTime)` gives the current instant for the trust crate's time acceptance (ADR-015). `Timestamp` keeps no text: `Display` writes the instant as `YYYY-MM-DDTHH:MM:SS[.fraction]Z`.

### 9. Conformance

#conformance-levels: "Implementations MUST satisfy all requirements of their declared level." This workspace declares Level 2 (ADR-001, D2). `validate` reports the level a document meets, never the implementation's level (AMB-51):

- Any Error → `Conformance::NotMet` (RD-06). Errors win over the depth limit: a known defect is not undetermined.
- Otherwise, any `nested-depth-limit` → `Conformance::Undetermined`.
- Otherwise, root `host` present → `Met(ConformanceLevel::Discoverable)` (#level-2-discoverable-catalog: "Includes a `host` object"; AMB-53); absent → `Met(ConformanceLevel::Minimal)` (#level-1-minimal-catalog: "All other defined members are OPTIONAL").
- Warnings never change the outcome.
- `ConformanceLevel` has `Minimal` and `Discoverable` only (`#[non_exhaustive]`, `Ord` with `Minimal < Discoverable`). Level 3 is never reported (D2, AMB-22, AMB-54); the trust crate reports per-manifest outcomes instead (ADR-015).
- A document with no `trustManifests`, `signature` or `digest` gets no diagnostic for their absence (#level-3-trusted-catalog: "Consumers MAY ignore fields defined at higher conformance levels and SHOULD gracefully handle their absence").
- The conformance of a nested catalog never sets the parent's level; only its diagnostics are appended (§6).

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Document judgement | `validate::validate`, `validate::validate_at` | lambda cold start (ADR-010), client `load` (ADR-008, ADR-009) |
| Report | `validate::{Report, Diagnostic, Severity, Code, Conformance, ConformanceLevel}` | `validate`, `validate_at`; `validate_manifest` from S5 |
| Nesting depth | `validate::{NestingDepth, MAX_NESTING_DEPTH}` | `validate_at`; client traversal (ADR-009) |
| Nested value → model | crate-private `parse::catalog(&Json, &JsonPointer)` (object check, gate, `document`) | `parse` (after `json::read`); `validate` for inline `data` |
| Digest value | `digest::{DigestValue, DigestAlgorithm, DigestError}`, `DigestValue::parse` | `validate`; `validate_manifest` (ADR-006); trust crate (ADR-013) |
| Instant | `timestamp::{Timestamp, TimestampError}`, `Timestamp::parse`, `Timestamp::from_system_time` | `validate`; `validate_manifest` (ADR-006); trust crate (ADR-015) |
| Extension-key rule | crate-private `extensions::is_valid_key` | `validate`; `validate_manifest` (ADR-006) |
| Policy URLs | `extensions::{POLICY_URLS, PolicyUrls, PolicyUrlsError}`, `CatalogEntry::policy_urls` | `validate`; client display (ADR-009) |
| URI grammar | crate-private `uri::is_uri_with_authority` (RFC 3986 `URI`) | `extensions::is_valid_key`; `uri` gains `absolute-URI` with its first caller in S5 (ADR-006) |
| I-JSON number rule | crate-private `number::defect` (on `number::to_canonical`, ADR-003) | `validate`; the writer uses `number::to_canonical` (ADR-003) |
| Noncharacter rule | crate-private `json::is_noncharacter` (ADR-002) | writer (ADR-003); `validate` |

No derived artifact, golden bytes or schema digest belongs to this ADR (D17 applies to ADR-003, ADR-009 and ADR-013). `Code` names are the contract shown by the CLI's `--format json` (ADR-011); the table in §2 is their one list, and `conformance_level::every_code_name_is_listed_once` pins it.

## Requirement → test

Test paths are under `crates/turul-ai-catalog/tests/`. Ids are the PLAN §4 ids.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| multi-version-entries-1 | "MAY contain multiple entries with the same `identifier` and different `version` values" | `entry_uniqueness::one_identifier_with_distinct_versions_is_valid` |
| multi-version-entries-2 | (`identifier`, `version`) "MUST be unique within the catalog" | `entry_uniqueness::repeated_identifier_and_version_is_entry_duplicate` |
| multi-version-entries-3 | without `version`, "`identifier` alone MUST be unique" | `entry_uniqueness::repeated_versionless_identifier_is_entry_duplicate`, `entry_uniqueness::mixed_versioned_and_versionless_group_is_version_missing_in_group` |
| catalog-entry-19 | `version` "REQUIRED when a catalog lists multiple versions of the same `identifier`" | `entry_uniqueness::mixed_versioned_and_versionless_group_is_version_missing_in_group` |
| resolving-an-artifact-s-version-2 | "Within a multi-version listing it is REQUIRED and, combined with `identifier`, uniquely addresses the entry" | `entry_uniqueness::mixed_versioned_and_versionless_group_is_version_missing_in_group`, `entry_uniqueness::repeated_identifier_and_version_is_entry_duplicate` |
| AMB-70, RD-09 | `version: ""` is present | `entry_uniqueness::empty_version_is_a_present_version` |
| AMB-69 | empty `identifier` | `entry_uniqueness::empty_identifier_is_identifier_empty` |
| AMB-06, AMB-19 (diagnostic half) | empty `type` is an error; unknown or reserved types are not | `artifact_types::empty_type_is_type_empty`, `artifact_types::unknown_and_reserved_types_give_no_diagnostic` |
| RD-10 (diagnostic half) | identifier without `:` is opaque | `entry_uniqueness::identifier_without_colon_gives_no_diagnostic` |
| AMB-46 (uniqueness scope) | a nested catalog has its own scope | `nested_inline_catalogs::nested_entry_may_reuse_a_parent_identifier` |
| RD-02 (diagnostic half) | empty `url` | `artifact_source::empty_url_is_url_empty` |
| #version-format, AMB-44 | components "are non-negative integers" | `spec_version_format::leading_zero_is_spec_version_noncanonical`, `spec_version_format::canonical_versions_give_no_diagnostic` |
| format-and-key-naming-2, AMB-47 | keys "MUST be a valid URL or a reverse-DNS string" | `extension_keys::url_and_reverse_dns_keys_are_valid`, `extension_keys::malformed_keys_are_extension_key_invalid`, `extension_keys::invalid_key_never_fails_parse` |
| format-and-key-naming-3 | unrecognised key: "MUST ignore it without throwing an error" | `extension_keys::unknown_valid_keys_give_no_diagnostic` |
| AMB-48 | metadata is schemaless | `extension_keys::metadata_value_is_not_validated` |
| policy-urls-extension-1 | "Its value MUST be a JSON object" | `extension_keys::non_object_policy_urls_is_policy_urls_invalid` |
| policy-urls-extension-2, AMB-49 | members OPTIONAL; strings | `extension_keys::empty_policy_urls_object_is_valid`, `extension_keys::non_string_policy_url_member_is_policy_urls_invalid`, `extension_keys::policy_urls_reader_returns_both_links` |
| policy-urls-extension-3, AMB-49 | "A publisher or catalog operator MAY populate this extension"; `entry.extensions` only | `extension_keys::policy_urls_outside_entry_extensions_is_an_ordinary_key` |
| nested-catalog-entries-1 | "MAY reuse the same `identifier`" | `nested_inline_catalogs::nested_entry_may_reuse_a_parent_identifier` |
| nested-catalog-entries-2, -3, nested-catalog-depth-and-circular-references-1, -2, AMB-45 | "SHOULD impose a maximum nesting depth"; "A depth limit of 4 is RECOMMENDED" | `nested_inline_catalogs::four_levels_are_validated`, `nested_inline_catalogs::fifth_level_is_nested_depth_limit_and_undetermined`, `nested_inline_catalogs::validate_at_starts_from_the_given_depth` |
| nested-catalog-entries-4, -5 | "MAY support deeper nesting but SHOULD document their limit" | `nested_inline_catalogs::max_nesting_depth_is_four` (the rustdoc states it) |
| AMB-46, AMB-152, FR-12 | nested shape error is one diagnostic | `nested_inline_catalogs::nested_shape_error_is_one_diagnostic_under_data`, `nested_inline_catalogs::nested_unsupported_major_is_nested_catalog_invalid`, `nested_inline_catalogs::nested_diagnostics_carry_the_data_prefix`, `nested_inline_catalogs::nested_catalog_needs_no_host` |
| AMB-98 (nested half) | only the exact type is nested | `nested_inline_catalogs::other_case_type_keeps_data_opaque` |
| digest-format-1 | "Producers SHOULD use SHA-256 or stronger" | `digest_format::sha256_384_512_are_valid_in_any_name_case` |
| digest-format-2, AMB-23, AMB-153 | "MUST reject digest values using algorithms shorter than SHA-256" | `digest_format::short_output_is_digest_weak`, `digest_format::known_name_with_wrong_length_is_digest_invalid`, `digest_format::uppercase_hex_is_digest_invalid`, `digest_format::malformed_values_are_digest_invalid`, `digest_format::unknown_long_algorithm_is_a_warning` |
| AMB-64, AMB-154 | `tdate` is RFC 3339 `date-time` | `timestamps::rfc3339_forms_parse`, `timestamps::non_rfc3339_forms_are_refused`, `timestamps::offsets_compare_as_instants`, `timestamps::leap_second_is_accepted`, `timestamps::invalid_updated_at_is_timestamp_invalid` |
| RD-50 | empty `updatedAt` and `digest` are present | `timestamps::empty_updated_at_is_timestamp_invalid`, `digest_format::empty_digest_is_digest_invalid` |
| catalog-entry-21, AMB-09 | inline `data` "MUST satisfy JCS's input requirements" | `number_diagnostics::non_i_json_number_in_digested_data_is_an_error` |
| signed-payload-and-jws-construction-3, AMB-30 | "The payload MUST satisfy JCS's I-JSON constraints" | `number_diagnostics::non_i_json_number_in_signed_manifest_is_an_error`, `number_diagnostics::non_i_json_number_in_signed_catalog_is_an_error`, `noncharacter_diagnostics::noncharacter_in_signed_or_digested_context_is_an_error` |
| signed-payload-and-jws-construction-5 | large integers "SHOULD be encoded as strings" | `number_diagnostics::inexact_number_is_a_warning_in_every_context`, `number_diagnostics::non_i_json_number_elsewhere_is_a_warning`, `number_diagnostics::validate_never_rewrites_numbers`, `number_diagnostics::typed_size_above_2_pow_53_is_reported`, `noncharacter_diagnostics::noncharacter_elsewhere_is_a_warning` |
| privacy-considerations-1, AMB-62 | "Logo URLs SHOULD use Data URIs" | `logo_url_privacy::non_data_logo_url_is_a_warning`, `logo_url_privacy::data_logo_url_gives_no_diagnostic` |
| conformance-levels-1, D2, AMB-51 | "MUST satisfy all requirements of their declared level" | `conformance_level::valid_corpus_documents_meet_their_recorded_level` (every ADR-001 corpus document with its outcome, recorded in the test) |
| level-1-minimal-catalog-1 | "All other defined members are OPTIONAL" | `conformance_level::minimal_document_is_met_minimal` |
| AMB-53 | Level 2 = `host` present | `conformance_level::document_with_host_is_met_discoverable` |
| RD-06 | errors are not a level | `conformance_level::any_error_is_not_met`, `conformance_level::warnings_do_not_change_conformance` |
| level-3-trusted-catalog-7, -8, AMB-22, AMB-54 | "MAY ignore fields defined at higher conformance levels and SHOULD gracefully handle their absence" | `conformance_level::absent_trust_fields_give_no_diagnostic` |
| AMB-150 | report order and stability | `conformance_level::report_is_deterministic`, `conformance_level::every_code_name_is_listed_once` |

Sentences in the cited sections not owned here (PLAN §4): catalog-entry-16 (OOS), -17 and -20 (OOS: need the artifact), -18 (ADR-013); multi-version-entries-4 (OOS), -5 (ADR-007), -6 (ADR-009); resolving-an-artifact-s-version-1, -3, -5 (ADR-007), -4, -7 (OOS), -6 (ADR-009); format-and-key-naming-1 (OOS: semantic); policy-urls-extension-4 (ADR-009); nested-catalog-depth-and-circular-references-3 (ADR-009); privacy-considerations-2 (OOS); level-1-minimal-catalog-2 and level-3-trusted-catalog-2, -6 (ADR-006); level-3-trusted-catalog-1, -3, -4 (ADR-015), -5 (ADR-013); signed-payload-and-jws-construction-4 (ADR-002).

## Decisions on ambiguity

- AMB-150 (new): one flat `Report` per call. Nested diagnostics are appended with their prefix instead of a tree of reports, because every caller (lambda gate, CLI, client) needs "any error anywhere in these bytes", and the client already keeps one report per fetched document (ADR-009). Conformance is derived inside `Report`, not stored by callers.
- AMB-151 (new): the I-JSON check runs on `to_json()` output, not on a second walk of the typed model, so the writer and validator see the same values. The root `signature` makes the whole document a signed context, because a catalog signature covers the complete snapshot (#catalog-snapshot-coverage).
- AMB-152 (new): inline `data` goes through the same object check and version gate as `parse`, not `document` alone. #nested-catalog-entries calls it "another AI Catalog document", and #compatibility-rules says a consumer that does not support a major "SHOULD reject the document". Skipping the gate would map a 2.0 document as if it were 1.x. ADR-002's owner rows move from `document` to `parse::catalog` (ADR-002 revision log, this slice).
- AMB-153 (new): the algorithm grammar is the OCI digest grammar, because the spec names no grammar and "hash algorithm identifier" needs one to tell `sha256:…` from `:abc` or `a b:…`. Weakness is checked before the known-name length, so `sha256:<32 hex>` is `digest-weak`, the more specific defect.
- AMB-154 (new): RFC 3339 edge cases as in §8. Nanosecond precision is enough for every comparison the spec makes (`expiresAt` after `issuedAt`, skew windows).
- AMB-155 (new): only later duplicates are flagged, so a pair gives one error and the pointer names the entry to remove. A mixed group flags the versionless entries, because they are the ones that break "REQUIRED".

## Divergences from ai-catalog-rust 0.4.0

| Id | ai-catalog-rust | This crate | Spec | Pinned by |
|---|---|---|---|---|
| RD-06 | `Minimal` when there are errors | `NotMet` | #level-1-minimal-catalog is conformant, so errors cannot be Level 1 | `conformance_level::any_error_is_not_met` |
| RD-08 (inline) | containing depth ≥ 4 (root 0) is an error, so five inline levels are validated | root is 1, four levels; a fifth is a `nested-depth-limit` warning with `Undetermined` | SHOULD limit; 4 RECOMMENDED; WG ADR 0001 counts the root | `nested_inline_catalogs::fifth_level_is_nested_depth_limit_and_undetermined` |
| RD-09 | `version: ""` is absent for uniqueness | present | `version` is a string; presence decides (#multi-version-entries) | `entry_uniqueness::empty_version_is_a_present_version` |
| RD-10 | `identifier-not-uri` warning | nothing | #catalog-entry open text, opaque | `entry_uniqueness::identifier_without_colon_gives_no_diagnostic` |
| RD-11 | URL key `^scheme://[^/?#\s]+`, reverse-DNS any dotted name | RFC 3986 `URI` with authority; first label starts with a letter | "valid URL or reverse-DNS" (AMB-47) | `extension_keys::malformed_keys_are_extension_key_invalid` |
| RD-13 (format) | uppercase hex accepted; only sha256/384/512 accepted | uppercase is `digest-invalid`; strength by output length; other names ≥ 256 bits warn | "lowercase hexadecimal"; "shorter than SHA-256" | `digest_format::uppercase_hex_is_digest_invalid`, `digest_format::unknown_long_algorithm_is_a_warning` |
| RD-50 | empty `updatedAt` and `digest` are skipped as absent | present and invalid | `tdate`; #digest-format | `timestamps::empty_updated_at_is_timestamp_invalid`, `digest_format::empty_digest_is_digest_invalid` |
| RD-51 | no logo check | `logo-url-not-data-uri` warning | #privacy-considerations SHOULD | `logo_url_privacy::non_data_logo_url_is_a_warning` |
| RD-52 | no I-JSON number or string diagnostics | `number-not-i-json`, `number-inexact`, `string-not-i-json` (§5) | #signed-payload-and-jws-construction MUST and SHOULD | `number_diagnostics::non_i_json_number_in_signed_manifest_is_an_error`, `number_diagnostics::non_i_json_number_elsewhere_is_a_warning`, `number_diagnostics::inexact_number_is_a_warning_in_every_context` |

Agreements: `entry-duplicate`, `timestamp-invalid`, `extension-key-invalid`, `nested-catalog-invalid`, `policy-urls-invalid` and `digest-invalid` keep upstream's code names; policy-urls string rules agree; algorithm names compare without regard to case in both. The `sdk_interop` cases for RD-06, RD-08 and RD-13 are S7 (ADR-001 RD index).

## Consequences

- One validator, one digest parser, one instant and one depth rule. ADR-006 builds manifest checks on them; the client, lambda and CLI read `Report` and never judge content.
- `validate` costs one `to_json` and one walk per document; nested inline catalogs are mapped once more through `parse::catalog`. No consumer needs it faster today.
- Until S5, `validate` ignores `trustManifests` and `signature` content, so a document with a malformed manifest can report `Met`. S5 adds `validate_manifest` and closes this; ADR-006 owns those tests.
- ADR-002 is revised in this slice: `parse` calls `parse::catalog` (object check, gate, `document`), and its "Value → model" and "Version gate" owner rows name it.
- Plan drift: PLAN §3 lists nine test files. This ADR adds `noncharacter_diagnostics.rs` (already promised by ADR-002 AMB-93) and the diagnostic halves of `artifact_types.rs` (promised by ADR-004) and `artifact_source.rs` (RD-02 `url-empty`). PLAN §2 lists `uri` for S4; only `is_uri_with_authority` lands here, `absolute-URI` lands with its caller in S5 (a helper lands with its first caller, as in ADR-003).

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created for S4: `validate`, `validate_at`, `Report`, `Diagnostic`, `Code` (17 codes), `Conformance`, `ConformanceLevel`, `NestingDepth`, `MAX_NESTING_DEPTH`, `DigestValue::parse`, `Timestamp`, `extensions::{POLICY_URLS, PolicyUrls}`, `CatalogEntry::policy_urls`, crate-private `parse::catalog`, `extensions::is_valid_key`, `uri::is_uri_with_authority`, `number::is_i_json`. AMB-150 to AMB-155 and RD-50 to RD-52 added. |
| 2026-11-17 | Accepted at S4 close (0.0.5): every test file in Implements is present and green, every requirement row names a listed test function, and no other validation, digest, instant, depth or extension-key path exists in the workspace. |
| 2026-11-17 | S14: `uri` is a public module exposing only `is_fragment` (RFC 3986 `fragment`, empty allowed); the other grammar functions stay crate-private. The trust crate's `kid` checks and signer use it. |
| 2026-11-17 | M20 (decided by spec under the standing instruction; ADR-003 AMB-94): the MUST/SHOULD split now has its predicate, `number::defect` on `number::to_canonical`. `number-not-i-json` is a number that is not a finite double (non-finite or overflow), an Error in a signed or digested context and a Warning elsewhere; `number-inexact` (added to §2, 18 codes) is a number whose JCS form denotes another value, a Warning everywhere. §2, §5, the owner row (the unbuilt `number::is_i_json` replaced by `number::defect`) and the signed-payload-and-jws-construction-5 and RD-52 rows updated. Tests: `number_diagnostics` uses non-finite numbers for `number-not-i-json`; `inexact_number_is_a_warning_in_every_context` added; `typed_size_above_2_pow_53_is_reported` expects `number-inexact`. |
