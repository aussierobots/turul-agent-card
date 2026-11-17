# ADR-004: Identity keys and artifact types

- **Status:** Accepted (2026-11-17)
- **Date:** 2026-11-17
- **Implements:** under `crates/turul-ai-catalog/tests/`: `entry_key.rs`, `artifact_types.rs`
- **Depends on:** ADR-002 (model, `parse`, `identity::SPEC_VERSION`), ADR-003 (`to_json` writes each identity value through `as_str`). Binding decisions: M5 (turul owns its model), M6 (zero runtime dependencies), AGENTS.md / CLAUDE.md "one typed constructor per identity key".
- **Spec:** https://ai-catalog.io, `specification/ai-catalog.md` as read in ADR-001 (last commit 2026-11-10).

## Context

S1 stores the entry `identifier`, entry `type`, host `identifier` and the Subject's `identifier` and `type` as `String` (ADR-002 §"Owning symbols"). The repo rule is one typed constructor per identity key, and consumers never rebuild a key from fields. This ADR adds the typed values and the one entry key, and deletes the `String` fields in the same slice.

Spec anchors implemented: #catalog-entry (`identifier`, `type`, `version` as part of the key), #multi-version-entries (the key only; uniqueness diagnostics are ADR-005), #subject-object (`identifier` and `type` share the entry's vocabulary), #host-info (`identifier`), #media-type, #design-goals (goal 2), #well-known-uri (the path constant only; lookup is ADR-008).

## Decision

### 1. Opaque identifiers

`EntryIdentifier::new(impl Into<String>)` and `HostIdentifier::new(impl Into<String>)` are total. They store the text verbatim: no trimming, no case folding, no scheme parsing. Equality, ordering and hashing compare the stored text exactly.

- #catalog-entry: "This field is an open text format (e.g., any valid URI or URN is accepted). Consumers that do not recognize an identifier scheme MUST treat the value as opaque." This crate recognises no scheme. `urn:air` is parsed only by the trust crate's `UrnAir::parse` (ADR-012, AMB-78), and that parse never changes the `EntryIdentifier`. So `x-local:Foo/Bar` ≠ `x-local:foo/bar`, `urn:air:…` ≠ `URN:AIR:…`, and an identifier without `:` is a valid key (RD-10).
- #host-info: the host `identifier` is "a verifiable identifier … (e.g., a DID or domain name)". It is opaque and never treated as verified (AMB-18).
- AMB-99 (new): the constructors accept `""`. The CDDL types `identifier` as `text`, so `parse` must put an empty value into the model. The `identifier-empty` error then comes from `validate` with its JSON Pointer (ADR-005, AMB-69). A fallible constructor would make `parse` a second owner of the emptiness rule: an empty identifier would be a `ParseError` with no diagnostic, against AMB-69. PLAN §3's "opaque, non-empty" is read this way. Non-emptiness is a document rule, and `validate` owns it.
- No `From<String>`, `FromStr` or `Deref<Target = String>`. `new` is the one constructor and `as_str()` is the one reader.

### 2. The entry key

`CatalogEntry::key(&self) -> EntryKey` is the only way to build an `EntryKey`. The key holds `(EntryIdentifier, Option<String>)`, which is the entry's `identifier` and `version` exactly as parsed. It has private fields, the accessors `identifier()` and `version() -> Option<&str>`, and derives `Clone`, `Debug`, `Eq`, `Hash` and `Ord`.

- #multi-version-entries: "When `version` is present, the combination of `identifier` and `version` MUST be unique … When `version` is absent, `identifier` alone MUST be unique." The key is (identifier, version). `identifier` alone is not the key, because the spec allows several entries with one identifier (AMB-04).
- `version: ""` is present (AMB-70, RD-09). `(id, None)`, `(id, Some(""))` and `(id, Some("1"))` are three distinct keys. Equal inputs give equal keys with equal hashes.
- The `version` member stays `Option<String>` on `CatalogEntry`. It is free text (#catalog-entry: "Semantic Versioning … RECOMMENDED but not required"), resolved and sorted by ADR-007, and has no identity rule apart from its place in `EntryKey`. No `EntryVersion` type is added.
- Consumers (`validate` uniqueness in ADR-005, `resolve::latest` in ADR-007, the client) group and match through `EntryKey` or `EntryIdentifier`. None of them builds a key from strings.

### 3. Artifact types

`ArtifactType` wraps `Cow<'static, str>`. `ArtifactType::new(impl Into<String>)` is total, and the known types are `pub const`s on `ArtifactType`. The list below is copied by hand from #catalog-entry `type`, in spec order:

| Const | Value |
|---|---|
| `AI_CATALOG` | `application/ai-catalog+json` (= `MEDIA_TYPE`) |
| `AGENT_CARD` | `application/agent-card+json` |
| `A2A_AGENT_CARD` | `application/a2a-agent-card+json` |
| `MCP_SERVER_CARD` | `application/mcp-server-card+json` |
| `AGENT_SKILLS_JSON` | `application/agent-skills+json` |
| `AGENT_SKILLS_MD` | `application/agent-skills+md` |
| `AGENT_SKILLS_ZIP` | `application/agent-skills+zip` |
| `AGENT_SKILLS_GZIP` | `application/agent-skills+gzip` |
| `AGENT_PLUGINS_ZIP` | `application/agent-plugins+zip` |
| `AGENT_PLUGINS_GZIP` | `application/agent-plugins+gzip` |

- #catalog-entry: "This field is an open text format, so any string value is accepted." Every string builds an `ArtifactType`, including unknown types, the reserved `agent-card+json`, the unregistered `+md` and `""`. `parse` raises no error for any of them (AMB-06, AMB-19). The `type-empty` error belongs to ADR-005.
- AMB-98 (new): equality is exact and case-sensitive. `Application/AI-Catalog+JSON` ≠ `AI_CATALOG`, so its `data` stays opaque and is not validated as a nested catalog (ADR-005). Reasons: the spec calls `type` an open-text "type identifier", not a `Content-Type` header. #subject-object requires `type` to "exactly equal the containing entry's `type`". The spec writes every known type in lowercase. Matching media types without regard to case (RFC 6838 §4.2) applies only to HTTP `Content-Type`, which ADR-008 handles. One comparison rule serves filtering, nested detection and subject binding.
- #design-goals 2: "Each catalog entry MUST declare its artifact type … enabling clients to select, filter, and route entries without parsing artifact content." `type` is CDDL-required (ADR-002 `MissingMember`). A consumer selects by `entry.artifact_type == ArtifactType::MCP_SERVER_CARD` and never reads `data` or fetches `url` to do it.
- No `KNOWN` array and no `is_known()`. No caller needs them: the spec attaches no behaviour to "known", and AMB-06 forbids a diagnostic for unknown types.

### 4. Constants

- `MEDIA_TYPE: &str = "application/ai-catalog+json"` (#media-type). `ArtifactType::AI_CATALOG` is `ArtifactType(Cow::Borrowed(MEDIA_TYPE))`, so the workspace holds the literal only once. The client's `Accept`, the response media-type check (ADR-008) and the lambda's `Content-Type` (ADR-010) import `MEDIA_TYPE`.
- `WELL_KNOWN_PATH: &str = "/.well-known/ai-catalog.json"` (#well-known-uri). The client and the lambda import it (PLAN §2).
- `SPEC_VERSION` is unchanged (ADR-002).

### 5. Model fields retyped in this slice

| Struct | Field | S1 type | S3 type |
|---|---|---|---|
| `CatalogEntry` | `identifier` | `String` | `EntryIdentifier` |
| `CatalogEntry` | `artifact_type` | `String` | `ArtifactType` |
| `HostInfo` | `identifier` | `Option<String>` | `Option<HostIdentifier>` |
| `Subject` | `identifier` | `String` | `EntryIdentifier` |
| `Subject` | `artifact_type` | `String` | `ArtifactType` |

`CatalogEntry::new(EntryIdentifier, ArtifactType, ArtifactSource)` and `Subject::new(EntryIdentifier, ArtifactType, …)` take the typed values. `parse` builds them through `new`, and `to_json` writes `as_str()`. The `String` fields are deleted, not kept beside the new ones.

Subject fields are included because #subject-object requires each to "exactly equal" the entry's value. They are the same identity, so they use the same type, and ADR-006's binding check compares two `EntryIdentifier`s and two `ArtifactType`s.

These stay `String`, because this workspace never keys, groups or compares them as identities. Each is opaque text with no rule in the spec beyond its being present:

- `Publisher.identifier` (AMB-17: "never treated as verified").
- `TrustSchema.identifier`.
- `Signature.signer`. The trust crate parses it into its own `DidWebRoot` (ADR-012), and the format crate decodes nothing (M12).
- `Attestation.attestation_type`. This is an attestation format, not an artifact type.

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Entry identifier | `identity::EntryIdentifier::new`, `as_str` | `parse`; `CatalogEntry::new`, `Subject::new` |
| Entry key | `identity::EntryKey`, built only by `model::CatalogEntry::key` | `validate` uniqueness (ADR-005), `resolve::latest` (ADR-007), client |
| Artifact type | `identity::ArtifactType::new`, `as_str` and the 10 consts in §3 | `parse`; `CatalogEntry::new`, `Subject::new` |
| Host identifier | `identity::HostIdentifier::new`, `as_str` | `parse` |
| Media type | `identity::MEDIA_TYPE` (= `ArtifactType::AI_CATALOG`) | client, lambda |
| Well-known path | `identity::WELL_KNOWN_PATH` | client, lambda |

All identity types are newtypes with private fields, so `#[non_exhaustive]` adds nothing to them. They derive `Clone`, `Debug`, `PartialEq`, `Eq`, `Hash`, `PartialOrd` and `Ord`. No derived artifact, golden bytes or schema digest belongs to this ADR (D17 applies to ADR-003, ADR-009 and ADR-013).

## Requirement → test

Test paths are under `crates/turul-ai-catalog/tests/`.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| design-goals-2 | each entry "MUST declare its artifact type … select, filter, and route entries without parsing artifact content" | `artifact_types::entries_are_selected_by_type_without_reading_the_artifact` |
| catalog-entry-2 | unrecognised identifier scheme "MUST treat the value as opaque" | `entry_key::unrecognised_scheme_identifier_is_opaque_and_compared_exactly`, `entry_key::identifier_survives_parse_and_serialisation_verbatim` |
| catalog-entry-6 | `type` "open text format, so any string value is accepted"; known types RECOMMENDED | `artifact_types::known_type_consts_match_the_spec_list`, `artifact_types::any_string_is_an_artifact_type` |
| multi-version-entries (key only, AMB-04, AMB-70) | uniqueness is on (`identifier`, `version`) | `entry_key::absent_empty_and_nonempty_versions_give_distinct_keys`, `entry_key::same_identifier_and_version_give_equal_keys` |
| AMB-98 | exact, case-sensitive type comparison | `artifact_types::type_comparison_is_exact` |
| AMB-99 | empty identity values reach the model | `entry_key::empty_identifier_parses_into_the_model`, `artifact_types::any_string_is_an_artifact_type` (`""` case) |
| AMB-18 | host `identifier` opaque | `entry_key::host_identifier_is_kept_verbatim` |
| #media-type, #well-known-uri (constants) | `application/ai-catalog+json`; `/.well-known/ai-catalog.json` | `artifact_types::media_type_and_well_known_path_match_the_spec`, `artifact_types::media_type_literal_has_one_definition` (scan of `crates/*/src`: the literal appears once) |
| PLAN S3 gate, RD-36 | no public `String` identity field | `entry_key::identity_fields_are_typed` (type ascriptions on every §5 field; fails to compile if one reverts) |

The requirements left out are each owned elsewhere. catalog-entry-3, -4 and -5 are OOS (PLAN §4). Uniqueness diagnostics (`identifier-empty`, `type-empty`, duplicate keys, `version-missing-in-group`) are ADR-005, `tests/entry_uniqueness.rs` and `tests/artifact_types.rs` diagnostic half in S4. Subject binding is ADR-006.

## Divergences from ai-catalog-rust 0.4.0

| Id | ai-catalog-rust | This crate | Spec | Pinned by |
|---|---|---|---|---|
| RD-36 | `CatalogEntry::identifier`, `r#type` and the host and subject `identifier` are public `String` fields | `EntryIdentifier`, `ArtifactType`, `HostIdentifier` newtypes with one constructor each | AGENTS.md typed key; #catalog-entry opaque identifier | `entry_key::identity_fields_are_typed` |
| RD-10 | warns when an `identifier` has no `:` | opaque; valid key; no diagnostic | #catalog-entry "open text format … MUST treat the value as opaque" | `entry_key::unrecognised_scheme_identifier_is_opaque_and_compared_exactly` (key half); no-diagnostic half in `entry_uniqueness` (S4) |
| RD-09 (key half) | `version: ""` is the same as absent for uniqueness | `Some("")` is a distinct key | #catalog-entry `version` is a string; presence, not content, decides (#multi-version-entries) | `entry_key::absent_empty_and_nonempty_versions_give_distinct_keys` |

`ai_catalog::media::MEDIA_TYPE_CATALOG` has the same value as `MEDIA_TYPE`, and both crates compare `type` exactly (agree).

## Consequences

- One constructor per identity value, one key and one media-type literal. A consumer that needs an entry's identity calls `key()` and does not rebuild it.
- Constructing a type wrapper never fails, so `parse` stays the one place that rejects shape, and `validate` the one place that judges content (empty values).
- `ArtifactType` consts cost nothing (`Cow::Borrowed`). A parsed type allocates once, as the `String` did.
- ADR-002's "Model structs" paragraph ("Identity values are `String` in S1") gets a revision-log entry in this slice that records the retyped fields. `Subject::new`'s signature changes, and S1/S2 tests move to the typed constructors in the same slice.
- Plan drift: PLAN §3 names only the entry and host identity fields. §5 adds the two Subject fields (same identity, #subject-object), and AMB-99 reads "non-empty" as ADR-005's diagnostic rather than a constructor check.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created for S3: `EntryIdentifier`, `EntryKey` (via `CatalogEntry::key`), `ArtifactType` with 10 known-type consts, `HostIdentifier`, `MEDIA_TYPE`, `WELL_KNOWN_PATH`; entry, host and subject identity fields retyped. AMB-98 and AMB-99 added. |
| 2026-11-17 | Accepted: `entry_key.rs` and `artifact_types.rs` green; every §5 `String` identity field deleted. Released in 0.0.4. |
