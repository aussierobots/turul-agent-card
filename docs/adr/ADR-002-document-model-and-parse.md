# ADR-002: Strict reader, document model, version gate and unrecognised members

- **Status:** Accepted (2026-11-17)
- **Date:** 2026-11-17
- **Implements:** under `crates/turul-ai-catalog/tests/`: `required_members.rs`, `artifact_source.rs`, `duplicate_member_names.rs`, `string_escapes.rs`, `strict_json.rs`, `spec_version_gate.rs`, `unrecognized_members.rs` (parse half in S1, serialise half in S2), `producer_defaults.rs`
- **Depends on:** none. Binding decisions: M6 / D3 (zero runtime dependencies, own `Json`), M12 (no JWS or base64url in this crate), M15 / D23 (missing Signature member is a `ParseError`), D4 (JCS is the only serialisation, ADR-003).
- **Spec:** https://ai-catalog.io, `specification/ai-catalog.md` as read in ADR-001 (last commit 2026-11-10).

## Context

`turul-ai-catalog` is the single turul owner of the AI Catalog format (M1, M5). M6 requires a hand-rolled strict RFC 8259 reader and a `Json` type that keeps number lexemes and rejects duplicate member names, with no runtime dependency. This ADR fixes the bytes → model contract: what `parse` accepts, what it refuses and with which `ParseErrorKind` and JSON Pointer, and how unrecognised members survive. Semantic validation (`validate`, diagnostics, conformance) is ADR-005; serialisation is ADR-003; identity newtypes are ADR-004.

Spec anchors implemented: #top-level-structure, #host-info, #catalog-entry (shape only), #publisher-object, #trust-manifest, #optional-members, #subject-object, #trust-schema-object, #attestation-object, #provenance-link-object, #provenance-statements, #signature-object, #unrecognized-members, #consumer-requirements, #compatibility-rules, #consumer-behavior, #producer-behavior, #media-type-registration-application-ai-catalog-json, #cddl-schema, #design-goals, #embedded-content-safety (`data` well-formed), #signed-payload-and-jws-construction (duplicate names; unpaired surrogates; noncharacters kept at parse, AMB-93).

## Decision

### 1. One strict reader

The crate-private `json::read(&[u8]) -> Result<Json, ParseError>` is the only JSON reader in the crate. It implements RFC 8259 exactly:

- Input MUST be UTF-8 (RFC 8259 §8.1; IANA "binary (UTF-8 encoded JSON)"). Invalid UTF-8 is `InvalidUtf8`. A leading byte order mark is `Syntax`: RFC 8259 §8.1 lets a parser ignore it, and the strict reading is chosen (AMB-87).
- Only the four RFC 8259 whitespace characters. Anything after the top-level value except whitespace is `Syntax`.
- Number grammar per RFC 8259 §6. `01`, `1.`, `.5`, `+1`, `-`, `1e`, `NaN`, `Infinity` are `Syntax`. A `Number` keeps its lexeme byte-for-byte (`Number::as_str`), so `18446744073709551617` (2^64+1) and a 25-digit decimal are never rounded (M6). Two `Number`s are equal when they denote the same exact decimal value (`1.0 == 1 == 1e0`, `-0 == 0`), for an exponent of any length (`10e<N> == 1e<N+1>` with `N` beyond any fixed-width integer; comparison never overflows), so that ADR-003's law `parse(to_canonical_vec(m)) == m` can hold when JCS rewrites `1.0` as `1` (AMB-88).
- Strings: the eight RFC 8259 escapes and `\uXXXX`. An unescaped U+0000..U+001F, or any other escape, is `Syntax`. A `\uD800`..`\uDBFF` escape followed by a `\uDC00`..`\uDFFF` escape decodes to one scalar value (`"😀"` is U+1F600). Any unpaired surrogate escape, in a value or a member name, is `LoneSurrogate` (AMB-82), because a Rust `String` cannot hold it; I-JSON (RFC 7493 §2.1), which signed payloads must satisfy (#signed-payload-and-jws-construction), forbids it too. Nothing is replaced. Unicode noncharacters (U+FDD0..U+FDEF and every code point ending in FFFE or FFFF), raw or escaped, in a value or a member name, are kept unchanged: RFC 8259 §8.2 allows them and a `String` holds them. Their I-JSON check belongs to the signed payload and digested `data`, not to parse (AMB-93).
- Duplicate member names are refused at every depth, inside `data`, `extensions`, unrecognised members and trust objects alike: `DuplicateMember` (#signed-payload-and-jws-construction: "parsers MUST reject duplicate JSON member names rather than silently discard one"). Names are compared after unescaping, code point by code point (RFC 8259 §8.3), so `{"a":1,"a":2}` is a duplicate (AMB-84).
- At most 128 nested arrays and objects; the 129th is `DepthLimit`. RFC 8259 §9 permits a nesting limit, and #safe-fetching requires every fetched document to be treated as untrusted input; without a limit a recursive reader aborts the process on a stack overflow instead of returning an error (AMB-86). This is the reader's own limit, not the catalog nesting depth of ADR-005.
- Object member order is not significant (RFC 8259 §4: "an unordered collection"). `Json::Object` and `Members` compare as maps, and no API promises source order (AMB-92).

Reader errors (`Syntax`, `InvalidUtf8`, `DepthLimit`, `LoneSurrogate`, `DuplicateMember`) are found before any shape or version check, so `{"specVersion":"2.0","specVersion":"1.0"}` is `DuplicateMember`, not `UnsupportedMajor`.

### 2. Version gate before shape

`parse` reads, then requires the root to be an object (`WrongType` at the empty pointer), then gates `specVersion` before mapping any other member (#consumer-behavior: "Parse the `specVersion` field before processing the document"):

- Missing, not a string, or not matching `^[0-9]+\.[0-9]+$` → `SpecVersionInvalid` at `/specVersion` (#version-format: "Major and minor components are non-negative integers"; `+1.0`, `1`, `1.0.0`, `v1.0`, `" 1.0"` and `""` are invalid; AMB-44, RD-14).
- Numeric major ≠ 1 → `UnsupportedMajor { found }` at `/specVersion`, whose `Display` names the value found and the supported major (#compatibility-rules: "SHOULD reject the document with an informative error"; #consumer-behavior). The numeric value ignores leading zeros, so `01.0` and `1.00` pass the gate (their `spec-version-noncanonical` warning is ADR-005). Major 0 is `UnsupportedMajor` (AMB-43). Any minor of major 1 is accepted (#consumer-behavior: "regardless of minor version differences").
- A 2.0 document without `entries` therefore gives `UnsupportedMajor`, never `MissingMember`.

The supported major comes from `identity::SPEC_VERSION` (`"1.0"`), the one definition of the produced version. S1 creates the `identity` module with this constant at its final path; S3 adds the rest of the module (ADR-004).

### 3. Shape mapping (CDDL, fail fast)

The crate-private `parse::document(&Json, &JsonPointer) -> Result<AiCatalog, ParseError>` maps a read value to the model per the #cddl-schema rules, and is the one mapping. It is reached only through the crate-private `parse::catalog(&Json, &JsonPointer)`, which runs the root-object check and the version gate (§2) and then `document`: `parse` calls `catalog` after `json::read`, and `validate` (ADR-005) calls it for inline nested `data` (AMB-152). It stops at the first defect. When a document has several, the reader and the version gate come first; the order among shape defects is otherwise unspecified, and tests use single defects.

- Every CDDL-required member that is absent → `MissingMember`, pointer of the absent member (`/entries/0/identifier`). Nothing is defaulted or fabricated (RD-01). This includes a Signature object missing `signer`, `profile`, `issuedAt` or `jws` (M15).
- A defined member whose value has the wrong JSON type → `WrongType` at the value. `text` is a JSON string, arrays are arrays, maps are objects. `null` is not `text`, an object or an array, so a defined member set to `null` is `WrongType`, except `data` and `additionalSignatures`, whose type is `any` (AMB-85, RD-38).
- `tdate` members (`updatedAt`, `issuedAt`, `expiresAt`) must be strings and are stored as text. RFC 3339 syntax is checked by `validate` as a `timestamp-invalid` error (ADR-005, AMB-64), so `"updatedAt": "yesterday"` parses (AMB-90).
- `size` (CDDL `uint`, which is CBOR major type 0, RFC 8610 Appendix D: 0..2^64−1) must be a number whose lexeme is `0` or `[1-9][0-9]*` with a value ≤ 18446744073709551615. Anything else (`-1`, `1.0`, `1e3`, `-0`, 2^64) is `WrongType`. It is stored as `u64` (AMB-89).
- Exactly one of `url` and `data` (#catalog-entry: "MUST contain exactly one"; AMB-31, AMB-52): both → `ArtifactSourceConflict`, neither → `ArtifactSourceMissing`, each at the entry's pointer. `"data": null` is present (`ArtifactSource::Data(Json::Null)`); `"url": ""` is present (`ArtifactSource::Url("")`; its `url-empty` error is ADR-005) (RD-02, AMB-09).
- `data` is any JSON value and stays opaque `Json`, whatever `type` says, including `application/ai-catalog+json` (#design-goals 1; inline nested catalogs are validated by ADR-005 through the same `document`). Because it came through the strict reader it is well-formed JSON by construction (#embedded-content-safety).
- Opaque text (host and publisher `identifier`, `identityType`, `registryUri` such as `oci://…`, `governanceUri`, `logoUrl`) is stored verbatim and never interpreted at parse (AMB-17, AMB-18, AMB-21, AMB-63).
- Optional arrays are `Option<Vec<_>>`: absent and `[]` are distinct values (the model half of RD-03; ADR-003 pins the round trip).

### 4. Unrecognised members

Every model struct carries `unrecognized: Members`, holding each member the spec does not define for that object, with its value unchanged as `Json`. This applies to all ten object kinds: AiCatalog, HostInfo, CatalogEntry, Publisher, TrustManifest, Subject, Signature, TrustSchema, Attestation and ProvenanceLink. The eleventh CDDL rule, PolicyUrls, is an `extensions` value, so its members, known or not, are kept inside that `Json` value.

- The four open objects (#unrecognized-members) and the six trust objects, which are closed in the CDDL but "consumers MUST ignore unrecognized members when validating … but MUST retain them" (#cddl-schema), are handled identically: never rejected, never interpreted, always kept (AMB-40, AMB-41, AMB-42, AMB-43, AMB-65). An unknown member inside `signature` is kept (AMB-25).
- An unrecognised member never changes a defined member: an entry with and without `"Identifier"`, `"url2"` or `"lifecycle"` parses to equal defined fields (#consumer-requirements).
- `additionalSignatures` on the catalog root and on a Trust Manifest is accepted with any JSON value, `null` included, and kept in its own field `additional_signatures: Option<Json>`, never validated (#top-level-structure, #optional-members; AMB-03, AMB-33).
- "MUST NOT treat their presence as a conformance failure" (#consumer-requirements): `parse` never fails on one. The `validate` half (no diagnostic, conformance unchanged) is added to the same test function when `validate` exists (S4).

### 5. Producer defaults

Every model struct has `new(required members…)`, which sets only the required members, leaves every optional member `None`, `unrecognized` empty and `additional_signatures` `None`. `AiCatalog::new(entries)` sets `spec_version` to `identity::SPEC_VERSION` (#producer-behavior: "MUST set `specVersion` to the version of this specification they implement"; "SHOULD NOT set … higher"; AMB-58). No constructor sets `additionalSignatures` (#top-level-structure, #optional-members: "Producers SHOULD omit this member").

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Document model from bytes | `turul_ai_catalog::parse::parse(&[u8]) -> Result<AiCatalog, ParseError>` | `parse` |
| JSON text → value | `json::read(&[u8]) -> Result<Json, ParseError>` (the one reader; crate-private in S1, public from S2, unchanged) | `parse`; bare JSON values (S2 canonical vectors, later T3 DID documents) |
| Value → model | crate-private `parse::document(&Json, &JsonPointer)`, called only by `parse::catalog` | `parse::catalog` |
| Version gate | crate-private gate, called only by `parse::catalog(&Json, &JsonPointer)` (object check, gate, `document`); constant `identity::SPEC_VERSION` | `parse`; `validate` for inline nested `data` (ADR-005) |
| JSON value | `json::{Json, Number, JsonPointer}`; `Json::{Null, Bool, Number, String, Array, Object(Members)}` | — |
| Error | `parse::{ParseError { kind, pointer }, ParseErrorKind}`; both `#[non_exhaustive]`, `Display`, `std::error::Error` | — |
| Artifact source | `model::ArtifactSource::{Url(String), Data(Json)}` | `parse`, `CatalogEntry::new` |
| Unrecognised members | `model::Members` (map `String → Json`; `get`, `iter`, `len`, `is_empty`, `insert`), field `unrecognized` on every struct | `parse` |
| Producer defaults | `AiCatalog::new(entries)` and each struct's `new` | — |

`ParseErrorKind` = `Syntax`, `InvalidUtf8`, `DepthLimit`, `LoneSurrogate`, `DuplicateMember`, `SpecVersionInvalid`, `UnsupportedMajor { found: String }`, `MissingMember`, `WrongType`, `ArtifactSourceConflict`, `ArtifactSourceMissing`.

`ParseError::pointer` is an RFC 6901 pointer (`Display` renders it, `~0` and `~1` escaped; the root is the empty string) (AMB-91): the absent member for `MissingMember`; the value for `WrongType`; the second occurrence for `DuplicateMember` (object pointer + name); the entry for `ArtifactSource*`; `/specVersion` for the gate; the string value, or for a member name the containing object, for `LoneSurrogate`; and for `Syntax`, `InvalidUtf8` and `DepthLimit`, the innermost member or element whose text holds the defect, so a defect inside `data` has a pointer starting with `/entries/<n>/data`.

Model structs (`#[non_exhaustive]`, public fields, `Clone`, `Debug`, `PartialEq`). Field names are the spec names in snake case. `type` is `artifact_type` on CatalogEntry and Subject and `attestation_type` on Attestation. Identity values are `String` in S1. ADR-004 (S3) replaces the entry `identifier`, entry `type` and host `identifier` with typed keys and deletes the `String` fields in that slice.

No golden bytes or schema digest belong to this ADR (D17 applies to ADR-003, ADR-009 and ADR-013). M6 is checked by the slice gate `cargo tree -e normal -p turul-ai-catalog --depth 1` listing only the crate, with `[dependencies]` empty.

## Requirement → test

Test paths are under `crates/turul-ai-catalog/tests/`.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| top-level-structure-1 | catalog "MUST contain" `specVersion`, `entries` | `required_members::missing_required_catalog_member_is_parse_error` |
| top-level-structure-2 | `entries` "MAY be empty" | `required_members::empty_entries_array_parses` |
| top-level-structure-3 | `host`, `extensions`, `signature`, `additionalSignatures` OPTIONAL | `required_members::optional_catalog_members_may_be_absent_or_present` |
| top-level-structure-4 | `additionalSignatures`: "Producers SHOULD omit" | `producer_defaults::constructors_never_set_additional_signatures` |
| top-level-structure-5 | "MUST accept and ignore it regardless of its value" | `unrecognized_members::additional_signatures_of_any_type_is_accepted_and_kept`, `unrecognized_members::additional_signatures_survive_serialisation` (S2) |
| top-level-structure-6 | top level "MAY also contain members this specification does not define" | `unrecognized_members::unknown_members_are_kept_on_every_object_kind` |
| host-info-1 | Host Info "MUST contain" `displayName` | `required_members::missing_host_display_name_is_parse_error` |
| host-info-2 | `identifier`, `documentationUrl`, `logoUrl` OPTIONAL | `required_members::optional_host_members_may_be_absent_or_present` |
| host-info-3 | Host Info MAY contain undefined members | `unrecognized_members::unknown_members_are_kept_on_every_object_kind` |
| catalog-entry-1 | entry "MUST contain" `identifier`, `type` | `required_members::missing_required_entry_member_is_parse_error` |
| catalog-entry-7 | "MUST contain exactly one of" `url`, `data` | `artifact_source::entry_with_both_url_and_data_is_parse_error`, `artifact_source::entry_with_neither_url_nor_data_is_parse_error` |
| catalog-entry-9 | entry optional members | `required_members::optional_entry_members_may_be_absent_or_present` |
| catalog-entry-13 | `description` OPTIONAL | `required_members::optional_entry_members_may_be_absent_or_present` |
| catalog-entry-22 | entry MAY contain undefined members | `unrecognized_members::unknown_members_are_kept_on_every_object_kind` |
| trust-manifest-1 | Trust Manifest is OPTIONAL | `required_members::optional_entry_members_may_be_absent_or_present` |
| trust-manifest-2 | "Each manifest MUST contain `contributor`" | `required_members::missing_manifest_contributor_is_parse_error` |
| trust-manifest-3 | manifest "MAY contain the optional members" | `required_members::optional_manifest_members_may_be_absent_or_present` |
| optional-members-2 | manifest `additionalSignatures`: "Producers SHOULD omit" | `producer_defaults::constructors_never_set_additional_signatures` |
| optional-members-3 | "MUST accept and ignore it regardless of its value" | `unrecognized_members::additional_signatures_of_any_type_is_accepted_and_kept`, `unrecognized_members::additional_signatures_survive_serialisation` (S2) |
| subject-object-1 | Subject "MUST contain" `identifier`, `type`, `digest` | `required_members::missing_required_subject_member_is_parse_error` |
| trust-schema-object-1 | "MUST contain" `identifier`, `version` | `required_members::missing_required_trust_schema_member_is_parse_error` |
| trust-schema-object-2 | `governanceUri`, `verificationMethods` OPTIONAL | `required_members::optional_trust_schema_members_may_be_absent_or_present` |
| attestation-object-1 | "MUST contain" `type`, `uri` | `required_members::missing_required_attestation_member_is_parse_error` |
| attestation-object-2 | `digest`, `size`, `description` OPTIONAL | `required_members::optional_attestation_members_may_be_absent_or_present` |
| provenance-link-object-1 | "MUST contain" `relation`, `sourceId` | `required_members::missing_required_provenance_link_member_is_parse_error` |
| provenance-link-object-2 | four optional members | `required_members::optional_provenance_link_members_may_be_absent_or_present` |
| provenance-statements-1 | "MAY reference a signed provenance statement via `statementUri` … `signatureRef`" | `required_members::optional_provenance_link_members_may_be_absent_or_present` |
| signature-object-1 | "A Trust Manifest or catalog root MAY contain one `signature`" | `required_members::signature_parses_on_catalog_root_and_manifest` |
| signature-object-2 | "MUST contain `signer`, `profile`, `issuedAt`, and `jws`" (M15) | `required_members::missing_required_signature_member_is_parse_error` |
| signature-object-3 | "MAY contain `expiresAt`" | `required_members::signature_expires_at_may_be_absent_or_present` |
| signature-object-6 | `expiresAt`: "An OPTIONAL RFC 3339 timestamp" | `required_members::signature_expires_at_may_be_absent_or_present` |
| publisher-object-1 | "MUST contain" `identifier`, `displayName` | `required_members::missing_required_publisher_member_is_parse_error` |
| publisher-object-2 | `identityType` OPTIONAL | `required_members::optional_publisher_members_may_be_absent_or_present` |
| publisher-object-3 | Publisher MAY contain undefined members | `unrecognized_members::unknown_members_are_kept_on_every_object_kind` |
| unrecognized-members-1 | open objects "MAY contain members it does not define" | `unrecognized_members::unknown_members_are_kept_on_every_object_kind` |
| consumer-requirements-1 | "MUST NOT reject a document because it contains unrecognized members" | `unrecognized_members::document_with_unknown_members_is_not_rejected` |
| consumer-requirements-2 | "MUST NOT derive semantics … from an unrecognized member" | `unrecognized_members::unknown_member_does_not_change_defined_members` |
| consumer-requirements-3 | "MUST NOT alter how a consumer interprets `identifier`, `type`, …" | `unrecognized_members::unknown_member_does_not_change_defined_members` |
| consumer-requirements-4 | "SHOULD preserve unrecognized members, unchanged" | `unrecognized_members::unknown_member_values_are_kept_unchanged` (model), `unrecognized_members::unknown_members_survive_serialisation_on_every_object_kind` (re-serialisation through ADR-003's writer, S2) |
| consumer-requirements-5 | "MAY drop members it cannot represent" (we drop none) | `unrecognized_members::unknown_member_values_are_kept_unchanged` |
| consumer-requirements-8 | "MUST NOT treat their presence as a conformance failure" | `unrecognized_members::document_with_unknown_members_is_not_rejected` (parse half; `validate` half in S4) |
| compatibility-rules-2 | "Consumers MUST ignore unrecognized members" | `unrecognized_members::document_with_unknown_members_is_not_rejected` |
| compatibility-rules-3 | unsupported major "SHOULD reject … with an informative error" | `spec_version_gate::higher_major_is_rejected_with_informative_error` |
| consumer-behavior-1 | "Parse the `specVersion` field before processing" | `spec_version_gate::spec_version_is_gated_before_shape` |
| consumer-behavior-2 | accept matching major "regardless of minor" | `spec_version_gate::any_minor_of_major_one_is_accepted` |
| consumer-behavior-3 | "Ignore unrecognized members, and preserve them" | `unrecognized_members::unknown_members_are_kept_on_every_object_kind` |
| consumer-behavior-4 | reject a higher major "with an informative error message" | `spec_version_gate::higher_major_is_rejected_with_informative_error` |
| producer-behavior-1 | "MUST set `specVersion` to the version … they implement" | `producer_defaults::new_catalog_sets_spec_version_1_0` |
| producer-behavior-2 | "SHOULD NOT set `specVersion` … higher" | `producer_defaults::new_catalog_sets_spec_version_1_0` |
| signed-payload-and-jws-construction-4 | "parsers MUST reject duplicate JSON member names" | `duplicate_member_names::duplicate_member_name_at_any_depth_is_parse_error` |
| media-type-registration-application-ai-catalog-json-1 | "MUST contain `specVersion` and `entries`" | `required_members::missing_required_catalog_member_is_parse_error` |
| cddl-schema-1 | "MUST ignore unrecognized members when validating" (trust maps) | `unrecognized_members::unknown_members_are_kept_on_every_object_kind` |
| cddl-schema-2 | "MUST retain them when constructing signed payloads" | `unrecognized_members::unknown_members_are_kept_on_every_object_kind` (the model keeps them), `unrecognized_members::unknown_members_survive_serialisation_on_every_object_kind` (`TrustManifest::to_json` writes them on all six trust object kinds, S2); ADR-013 builds the payload |
| design-goals-1 | index "any type of AI artifact without … its internal schema" | `artifact_source::data_of_any_json_shape_with_unknown_type_parses` |
| embedded-content-safety-4 | "SHOULD validate that the `data` content is well-formed JSON" | `artifact_source::data_is_well_formed_json_by_construction` |

Decisions owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| RD-01 | missing required member is `MissingMember`, never defaulted | `required_members::missing_required_catalog_member_is_parse_error` |
| RD-02 | `data: null` and `url: ""` are present | `artifact_source::data_null_is_present`, `artifact_source::empty_url_is_present` |
| RD-04 | duplicates refused everywhere | `duplicate_member_names::duplicate_member_name_at_any_depth_is_parse_error` |
| RD-14 | `+1.0` is `SpecVersionInvalid`; leading zeros pass the gate | `spec_version_gate::malformed_spec_version_is_invalid`, `spec_version_gate::leading_zero_spec_version_parses` |
| RD-38 | defined member `null` is `WrongType` | `required_members::defined_member_of_wrong_type_is_parse_error` |
| RD-39 | number lexemes kept exactly | `strict_json::number_lexemes_are_kept_exactly` |
| AMB-03, AMB-33 | `additionalSignatures` any value, root and manifest alike | `unrecognized_members::additional_signatures_of_any_type_is_accepted_and_kept` |
| AMB-09 (null part) | `data: null` is present | `artifact_source::data_null_is_present` |
| AMB-17, AMB-18, AMB-21, AMB-63 | opaque text kept verbatim | `required_members::opaque_text_members_are_kept_verbatim` |
| AMB-25, AMB-40, AMB-41, AMB-42, AMB-65 | unknown members on every kind, trust objects and `signature` included, kept | `unrecognized_members::unknown_members_are_kept_on_every_object_kind` |
| AMB-30 | the one reader rejects duplicates | `duplicate_member_names::duplicate_member_name_at_any_depth_is_parse_error` |
| AMB-31, AMB-52 | both or neither of `url`/`data` fails at parse | `artifact_source::entry_with_both_url_and_data_is_parse_error`, `artifact_source::entry_with_neither_url_nor_data_is_parse_error` |
| AMB-43 | major 0 is `UnsupportedMajor` | `spec_version_gate::major_zero_is_unsupported` |
| AMB-44 | gate grammar `^[0-9]+\.[0-9]+$` | `spec_version_gate::malformed_spec_version_is_invalid` |
| AMB-58 | constructors set `SPEC_VERSION` only | `producer_defaults::new_catalog_sets_spec_version_1_0`, `producer_defaults::constructors_set_only_required_members` |
| AMB-64, AMB-90 | `tdate` is text at parse; syntax is ADR-005 | `required_members::opaque_text_members_are_kept_verbatim` |
| AMB-82 | unpaired surrogate escape is `LoneSurrogate`; pairs decode | `string_escapes::unpaired_surrogate_escape_is_parse_error`, `string_escapes::surrogate_pair_decodes_to_one_scalar` |
| AMB-84 | duplicate names compared after unescaping | `duplicate_member_names::duplicate_names_are_compared_after_unescaping` |
| AMB-85 | `null` for a defined non-`any` member is `WrongType` | `required_members::defined_member_of_wrong_type_is_parse_error` |
| AMB-86 | 128 nested containers accepted, 129 is `DepthLimit` | `strict_json::nesting_beyond_128_is_depth_limit` |
| AMB-87 | BOM is `Syntax`; invalid UTF-8 is `InvalidUtf8` | `strict_json::byte_order_mark_is_rejected`, `strict_json::invalid_utf8_is_rejected` |
| AMB-88 | `Number` equality by exact decimal value | `strict_json::numbers_compare_by_decimal_value` |
| AMB-89 | `size` is `uint` (0..2^64−1, integer lexeme) | `required_members::attestation_size_is_an_unsigned_64_bit_integer` |
| AMB-91 | pointer conventions | asserted in every negative test; `duplicate_member_names::duplicate_is_reported_before_the_spec_version_gate` for ordering |
| AMB-92 | member order not significant | `strict_json::object_member_order_is_not_significant` |
| AMB-93 | noncharacters are kept at parse; the I-JSON check is ADR-003 and ADR-005 | `string_escapes::noncharacters_parse_and_are_kept_unchanged` |

Further reader tests, which pin RFC 8259 and own no requirement id: `string_escapes::every_rfc_8259_escape_decodes`, `string_escapes::invalid_escape_or_raw_control_character_is_syntax_error`, `strict_json::non_rfc_8259_numbers_are_syntax_errors`, `strict_json::trailing_content_is_rejected`, `required_members::optional_arrays_absent_and_empty_are_distinct` (the model half of RD-03, which ADR-003 owns).

New ids (not in PLAN §6): RD-38, RD-39 and AMB-84 to AMB-93 are introduced here. AMB-83 is in ADR-001.

- AMB-84: RFC 8259 §8.3 compares strings as code-unit sequences after unescaping; a name-text comparison would let `"a"` bypass duplicate rejection.
- AMB-85: The CDDL gives `? host: HostInfo` and `? displayName: text`; `null` matches neither. Treating `null` as absent would make a document with an explicit `null` parse to the same model as one without the member, so re-serialisation would silently drop a member.
- AMB-86: the limit is 128 levels; see §1.
- AMB-87: strict BOM handling, as §1 states.
- AMB-88: value equality, because lexeme equality would break ADR-003's law for I-JSON numbers such as `1.0`.
- AMB-89: CDDL `uint` range, from RFC 8610.
- AMB-90: syntax lives in one place, `validate` (ADR-005), because #signature-acceptance-and-time makes timestamp validation an acceptance step, not a shape step.
- AMB-91: pointers as listed above.
- AMB-92: RFC 8259 §4.
- AMB-93: I-JSON (RFC 7493 §2.1) forbids noncharacters as well as surrogates. #signed-payload-and-jws-construction requires I-JSON of the signed payload only ("The payload MUST satisfy JCS's I-JSON constraints"), and #catalog-entry requires JCS input of inline `data` only "for artifact verification to succeed". Neither is a document-validity rule, so refusing noncharacters at parse would reject conformant unsigned documents; an unpaired surrogate is refused only because it cannot be represented. The MUST is owned beside the number rule (AMB-09, AMB-30), through one crate-private predicate `json::is_noncharacter(char)` added with its first caller in S2: the JCS writer (ADR-003) refuses a string value or member name holding a noncharacter with `NotIJson { pointer }`, as it refuses a non-I-JSON number, because RFC 8785 §3.1 accepts only I-JSON, so no signed payload or `data` digest is ever computed over one (`tests/i_json_strings.rs`); `validate` (ADR-005, S4) reports `string-not-i-json`, an error inside an object that carries `signature` or inside the `data` of an entry with `digest`, a warning otherwise, exactly the AMB-09 severity (`tests/noncharacter_diagnostics.rs`). The round-trip law stays stated for I-JSON documents only.

## Divergences from ai-catalog-rust 0.4.0

| Id | ai-catalog-rust | This crate | Spec | Pinned by |
|---|---|---|---|---|
| RD-01 | `#[serde(default)]` on required members fabricates `""`/`[]` | `MissingMember` | #top-level-structure, #catalog-entry, #host-info MUST | `required_members::missing_required_catalog_member_is_parse_error`; agreement row in `sdk_interop` (S7) |
| RD-02 | `data: Option<Value>` maps `null` to absent | `Data(Json::Null)` | CDDL `data: any` | `artifact_source::data_null_is_present` |
| RD-04 | `serde_json` keeps the last duplicate | `DuplicateMember` | #signed-payload-and-jws-construction | `duplicate_member_names::duplicate_member_name_at_any_depth_is_parse_error` |
| RD-14 | `+1.0` accepted | `SpecVersionInvalid` | #version-format | `spec_version_gate::malformed_spec_version_is_invalid` |
| RD-38 | `Option<T>` maps a `null` defined member to absent | `WrongType` | #cddl-schema | `required_members::defined_member_of_wrong_type_is_parse_error` |
| RD-39 | `serde_json` without `arbitrary_precision` reads large numbers in `data` and unknown members as `f64` | lexeme kept | #consumer-requirements "preserve … unchanged" | `strict_json::number_lexemes_are_kept_exactly` |

ai-catalog-rust's `Attestation::size: Option<u64>` agrees with AMB-89 apart from `null`, which is RD-38.

## Consequences

- One reader, one mapping and one version gate. The client, lambda and trust crates obtain a model only through `parse`. A bare `Json` from bytes comes only from `json::read`, public from S2 as that same function (ADR-003); no second reader is added.
- Strictness costs: a document that RFC 8259 alone allows is refused when it has an unpaired surrogate escape, a BOM, more than 128 nested levels or an explicit `null` on a defined member. Each case is a named kind with a pointer.
- Plan drift recorded: PLAN §3 lists seven test files for ADR-002. This ADR adds `tests/strict_json.rs`, because M6's strict RFC 8259 reader needs a contract test of its own (numbers, BOM, UTF-8, depth, member order). The "11 object kinds" of PLAN S1 and S7 are the ten object kinds plus PolicyUrls, as §4 states. PLAN §4 row signed-payload-and-jws-construction-3 names numbers only; its string half is AMB-93, which ADR-003 (S2) and ADR-005 (S4) take with the test files named there.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created for S1: reader, model, version gate and unrecognised members (parse half). RD-38, RD-39 and AMB-84 to AMB-92 added. S2 adds the serialise half of `unrecognized_members` and closes this ADR. |
| 2026-11-17 | S1 review: `Number` equality is exact for exponents of any length. The exponent was held in a saturating `i128`, so distinct values beyond `i128::MAX` compared equal and the digit normalisation after it overflowed; `numbers_compare_by_decimal_value` now covers large-exponent equal and unequal cases. |
| 2026-11-17 | S1 review: the #signed-payload-and-jws-construction anchor read "I-JSON strings", but the reader enforces only the unpaired-surrogate part of I-JSON and kept noncharacters with no owner. Anchor narrowed to unpaired surrogates; AMB-93 added: noncharacters are kept at parse (`string_escapes::noncharacters_parse_and_are_kept_unchanged`), and their I-JSON check is assigned to the ADR-003 writer and an ADR-005 diagnostic with AMB-09 severity. The finding's reader option was not taken: the spec requires I-JSON of signed payloads and digested `data` only. |
| 2026-11-17 | S2: `json::read` made public, unchanged, for bare JSON values (ADR-003 vectors). The serialise half of `unrecognized_members` is assigned: `unknown_members_survive_serialisation_on_every_object_kind` (consumer-requirements-4, cddl-schema-2) and `additional_signatures_survive_serialisation` (top-level-structure-5, optional-members-3). The writer half of AMB-93 is implemented by ADR-003 (`tests/i_json_strings.rs`). |
| 2026-11-17 | Accepted: the serialise half of `unrecognized_members.rs` is green and `json::read` and `parse` are the only reader and parse path in the workspace. |
| 2026-11-17 | S4 (ADR-005 AMB-152): inline nested `data` now passes the version gate as well as the shape mapping. The crate-private `parse::catalog` (object check, gate, `document`) is the one entrypoint for both `parse` and `validate`; the "Value → model" and "Version gate" owner rows name it. `parse` behaviour is unchanged. |
