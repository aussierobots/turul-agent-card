# ADR-001: Sources, conformance corpus, declared level, maintainer decisions and SDK interop

- **Status:** Proposed (S7 closes its scope; it stays Proposed until `sdk_interop` passes on CI)
- **Date:** 2026-11-17
- **Implements:** `crates/turul-ai-catalog/tests/spec_examples.rs` (parse half, S1; round-trip half, S2, for ADR-003 law 1), `crates/turul-ai-catalog/tests/sdk_interop.rs` (S7), `crates/turul-ai-catalog/tests/common/spec_corpus.rs` (S7: the one corpus list and wrapping rules), `crates/turul-ai-catalog/tests/fixtures/interop/` (S7).
- **Depends on:** ADR-002 (`parse`), ADR-003 (`to_canonical_vec`, the canonicalize law), ADR-005 and ADR-006 (`validate`, `Conformance`).

## Context

Every turul ADR cites the same sources at the same versions, and every claim of spec conformance is checked against one corpus. This ADR names the sources, the vendored corpus with its licences, how each spec example is fed to `parse`, the conformance level this workspace declares, the maintainer decisions in force, and where each disagreement with ai-catalog-rust is decided.

Spec anchors: #conformance-levels, #example-hierarchical-catalog. The examples used are those of every section listed in the corpus table.

## Decision

### 1. Sources (versions, never SHAs)

| Source | Version read | Role |
|---|---|---|
| AI Catalog specification, `specification/ai-catalog.md` in `github.com/Agent-Card/ai-catalog`, published at https://ai-catalog.io | revision whose last commit is dated 2026-11-10, read 2026-11-17 | Normative. Cited by heading anchor. |
| `specification/examples/` in the same repository | same revision | Normative examples |
| Working Group ADRs, `adr/` in the same repository | same revision | Intent only; the spec text wins (§5) |
| `ai-catalog-rust` (`github.com/Agent-Card/ai-catalog-rust`) | 0.4.0 (`rust-version = "1.95"`) | Reference implementation and fixtures, used only as a dev-dependency (M4, M5). Its divergences are recorded (§6), never followed silently. |

### 2. Corpus and licences (D9)

Two vendored directories, each with a `SOURCE.md` giving the source repository, path, version and licence, plus one row per file. Neither is published (`exclude = ["tests/fixtures"]`, S13).

- `crates/turul-ai-catalog/tests/fixtures/spec/`: CC-BY-4.0. The ai-catalog `LICENSE.md` licenses "files under `specification/`" under CC-BY-4.0, and `SOURCE.md` carries the attribution: source, licence and "unchanged".
- `crates/turul-ai-catalog/tests/fixtures/upstream/`: Apache-2.0 (the ai-catalog-rust `LICENSE`). These are the 13 files of ai-catalog-rust 0.4.0 `fixtures/`, byte-for-byte: `catalog.json`, `discoverable.json`, `future_member.json`, `invalid.json`, `manifest_shapes.json`, `minimal.json`, `nested_deep.json`, `nested_max.json`, `profile.json`, `spec-example.json`, `trust_clean.json`, `unsigned_trust.json`, `weak_signature.json`. `spec-example.json` is stale against `specification/examples/ai-catalog.json` and is classified as an upstream fixture, not a spec example (RD-35). `manifest_shapes.json` is a map of named manifest shapes, not a catalog. Accept/reject outcomes for these files are pinned by `sdk_interop` (§8).
- `crates/turul-ai-catalog/tests/fixtures/interop/`: authored in this repository (Apache-2.0), with a `SOURCE.md` row per file naming the RD or AMB id it exercises (§8.2). Added in S7.

Spec-example extraction rule: `examples/ai-catalog.json` is copied unchanged to `tests/fixtures/spec/examples/ai-catalog.json`. Each fenced ```` ```json ```` block of `ai-catalog.md` becomes `tests/fixtures/spec/blocks/NN-<heading-anchor>.json`. `NN` is the block's ordinal in the spec (01–18) and the anchor is that of the nearest heading above the block; the generic `## Examples` heading under #unrecognized-members is written `unrecognized-members-examples`. The content is the lines between the fences, joined with `\n`, plus a final `\n`, with no other change.

### 3. How each spec example reaches `parse` (AMB-66, AMB-83)

The test builds the input by byte concatenation, `prefix ++ fragment bytes ++ suffix`, so the reader sees the example's own bytes. Wrapper entry `E` = `"identifier":"urn:air:example.com:fixture:wrapper","type":"application/mcp-server-card+json","url":"https://example.com/wrapper.json"`.

| Rule | Prefix | Suffix |
|---|---|---|
| `doc` | — | — |
| `entry` | `{"specVersion":"1.0","entries":[` | `]}` |
| `host` | `{"specVersion":"1.0","host":` | `,"entries":[]}` |
| `trustSchema` | `{"specVersion":"1.0","entries":[{E,"trustManifests":[{"contributor":"did:web:example.com","trustSchema":` | `}]}]}` |
| `attestation` | as `trustSchema` with `"attestations":[` | `]}]}]}` |
| `provenance` | as `trustSchema` with `"provenance":[` | `]}]}]}` |
| `entry-merge` | the fragment's first `{` is replaced by `{E,`, then rule `entry` | — |

| Fixture | Anchor | Kind | Rule | `parse` outcome |
|---|---|---|---|---|
| `examples/ai-catalog.json` | — | document | `doc` | parses |
| `blocks/01-top-level-structure.json` | #top-level-structure | document | `doc` | parses |
| `blocks/02-host-info.json` | #host-info | Host Info | `host` | parses; `host.display_name` = `Acme Enterprise AI` |
| `blocks/03-multi-version-entries.json` | #multi-version-entries | document | `doc` | parses |
| `blocks/04-catalog-projection.json` | #catalog-projection | entry | `entry` | parses |
| `blocks/05-optional-members.json` | #optional-members | entry | `entry` | parses; two manifests |
| `blocks/06-trust-schema-object.json` | #trust-schema-object | Trust Schema | `trustSchema` | parses |
| `blocks/07-attestation-object.json` | #attestation-object | Attestation | `attestation` | parses; `size` = 245760 |
| `blocks/08-provenance-link-object.json` | #provenance-link-object | Provenance Link | `provenance` | parses |
| `blocks/09-signed-payload-and-jws-construction.json` | #signed-payload-and-jws-construction | signed payload | exempt | not parsed: a payload, not a stored object (its `signature` lacks `jws` by construction). It is the S2 canonical vector and the T1 payload golden. |
| `blocks/10-nested-catalog-entries.json` | #nested-catalog-entries | document | `doc` | parses |
| `blocks/11-nested-catalog-entries.json` | #nested-catalog-entries | entry | `entry` | parses |
| `blocks/12-format-and-key-naming.json` | #format-and-key-naming | document | `doc` | **refused**: `ArtifactSourceMissing` at `/entries/0` (AMB-83) |
| `blocks/13-policy-urls-extension.json` | #policy-urls-extension | entry excerpt (no `identifier`) | `entry-merge` | parses; the policy-urls value is kept |
| `blocks/14-unrecognized-members-examples.json` | #examples (under #unrecognized-members) | entry | `entry` | parses; `costCenter`, `approvedBy`, `reviewDate` kept |
| `blocks/15-unrecognized-members-examples.json` | #examples (under #unrecognized-members) | entry | `entry` | parses; `lifecycle`, `supersededBy` kept |
| `blocks/16-example-multi-artifact-catalog-with-nested-catalog.json` | #example-multi-artifact-catalog-with-nested-catalog | document | `doc` | parses |
| `blocks/17-example-hierarchical-catalog.json` | #example-hierarchical-catalog | document | `doc` | parses; direct and nested entries |
| `blocks/18-example-dual-protocol-agent-mcp-a2a.json` | #example-dual-protocol-agent-mcp-a2a | entry | `entry` | parses |

AMB-83 (new): the #format-and-key-naming example's only entry has `identifier`, `type`, `tags` and `extensions` but neither `url` nor `data`. #catalog-entry says "A Catalog Entry MUST contain exactly one of the following members", and the example is illustrative ("For example, a catalog entry with `extensions` …"). The MUST wins, so the example is refused and the test pins that refusal. Adding a `url` to make it parse would fabricate content (RD-01). It is a defect in the spec example. D20 / M11 do not list it, so it is reported to the maintainer through this ADR and no upstream draft is written.

Validation outcomes (`validate`, ADR-005) are added per fixture by S4. The two placeholder `jws` values (`…..detached-jws-signature`) are opaque text to the format crate, which never decodes them (M12). In the trust crate they pass `header::inspect` and never verify (S14).

### 4. Declared conformance level (#conformance-levels; D2)

This workspace declares **Level 2 (Discoverable Catalog)**: `validate` reports `Met(Minimal)` or `Met(Discoverable)` for a document (ADR-005), the producer API can emit `host`, and the lambda serves the well-known URI (M2, ADR-010). Level 3 is never claimed (D2), although `turul-ai-catalog-trust` implements the Level 3 consumer checks (ADR-012 to ADR-015). "Implementations MUST satisfy all requirements of their declared level" is pinned by `conformance_level` (ADR-005).

### 5. WG ADRs that the spec text overrides

- 0001 counts the enterprise root as level 1 of 4; the spec says only "A depth limit of 4 is RECOMMENDED". The WG reading is adopted: root = 1, so at most 4 catalog levels (AMB-45, ADR-005).
- 0010: "AI Card" is not defined in the spec.
- 0011: the spec path is `/.well-known/ai-catalog.json`.
- 0012: superseded by `extensions` and open objects.
- 0014: the spec adds `agent-plugins+zip` and `+gzip`.
- 0019, 0025: the Subject has no `url` and uses `type`.
- 0028a: privacy and terms URLs live in the policy-urls extension; `additionalSignatures` is in the CDDL.

No ADR implements a WG ADR's text against the spec.

### 6. Disagreements with ai-catalog-rust 0.4.0

Each disagreement has one RD id, one deciding ADR and a pinning test. The full rationale is in the deciding ADR and in PLAN §6a. `sdk_interop` asserts, per corpus file, that the two implementations agree on accept/reject, or that §8.3 names the RD id and spec anchor that decides it.

| RD | Deciding ADR | Pinning test file(s) |
|---|---|---|
| RD-01, RD-02, RD-04, RD-14, RD-38, RD-39 | ADR-002 | `required_members`, `artifact_source`, `duplicate_member_names`, `spec_version_gate`, `strict_json` |
| RD-03 | ADR-003 | `canonical_serialisation` |
| RD-06, RD-08 (inline), RD-09, RD-11, RD-12, RD-13 (format) | ADR-005 / ADR-006 | `conformance_level`, `nested_inline_catalogs`, `entry_uniqueness`, `extension_keys`, `manifest_validity`, `signature_object`, `digest_format`, `subject_binding` |
| RD-10, RD-36 | ADR-004 | `entry_key` |
| RD-17 | ADR-006 | `subject_binding` |
| RD-18 | ADR-007 / ADR-009 | `resolution`, `filter` |
| RD-28..RD-34, RD-08 (`url`) | ADR-008 / ADR-009 | `well_known`, `response_acceptance`, `safe_fetching`, `nested_traversal` |
| RD-15, RD-16 | ADR-012 / ADR-015 | `jws_header`, `did_web_signer`, `signature_acceptance` |
| RD-07, RD-19..RD-27, RD-13 (integrity) | ADR-013 / ADR-015 | `artifact_integrity`, `trust_report`, `profile_selection`, `urn_air`, `signature_acceptance`, `sdk_trust_interop` |
| RD-35 | ADR-001 | `sdk_interop` (fixture classification) |
| RD-05, RD-37 | — | informational, not built |

`sdk_interop` adds a cross-implementation pin for RD-01, RD-02, RD-03, RD-04, RD-06, RD-08, RD-12, RD-13, RD-14, RD-15, RD-16 and RD-35 (§8.4). S15's upstream drafts cite those functions.

### 7. Maintainer decisions in force

The binding text is `docs/plan/DECISIONS.md`; the accepted asks are in PLAN §0b. Listed here by id only, so there is one definition:

| Id | Effect |
|---|---|
| M1 | Workspace `turul-agent-card`; format crate `turul-ai-catalog`, with no network or async dependency |
| M2 | `turul-ai-catalog-lambda` serves one bundled catalog at the well-known path (D18); the bundle is superseded by M19, and the Lambda serves a catalog assembled from a store (ADR-016) |
| M3 | `turul-ai-catalog-client` owns consumer logic; the CLI and TUI are thin (D10, D12, D13) |
| M4 | Interop tests at every boundary; ai-catalog-rust is a dev-dependency only |
| M5 | Own model (D1) |
| M6 | Zero runtime dependencies in the format crate; strict reader; lexeme-keeping `Json` (D3) |
| M7 | D2, D4, D9, D11, D14, D15, D16, D17, D18 accepted |
| M8 | D19 done |
| M9 | D5–D8 in scope; the spec decides unclear design |
| M10 | JWS cryptography only through `turul-jwt-validator` |
| M11 | Upstream issue texts drafted, never posted (D20) |
| M12 | JWS header and `did:web` signer-shape checks live in the trust crate; the format crate never decodes JWS or base64url |
| M13 | A failing optional signature is a warning-severity non-endorsement |
| M14 | `turul-jwt-validator` feature line (D21) |
| M15 | A Signature object missing a CDDL-required member is a `ParseError` (D23) |

### 8. SDK interop (M4, S7)

#### 8.1 Dependencies and outcome definitions

`[workspace.dependencies]` gains `ai-catalog = "=0.4.0"`, `ai-catalog-validate = "=0.4.0"` and `ai-catalog-trust = "=0.4.0"` (D15). Only `turul-ai-catalog` lists them, under `[dev-dependencies]` with `workspace = true` (M4, M5). The exact pin matters: every expected outcome in §8.3 is a fact about 0.4.0, and a new upstream version is a new slice that re-runs the table. MSRV stays 1.95 (D14). No other crate gains a dependency.

For input bytes `b` (a corpus file, or a spec fragment wrapped by its §3 rule):

- **ours accepts** `b` when `parse(b)` is `Ok(c)` and `validate(&c).has_errors()` is false. Warnings do not reject (ADR-005).
- **theirs accepts** `b` when `ai_catalog::parse_slice(b)` is `Ok(t)` and `ai_catalog_validate::validate(&t).is_valid` is true (default `Options`).
- **agree** means both accept or both reject. A difference in which code or which stage (parse or validate) rejects is not a disagreement; the codes are not compared, because the two vocabularies are independent and ours is the owner (ADR-005).

Block 09 stays exempt (§3): it is a payload, not a document.

#### 8.2 Fixtures of ours (`tests/fixtures/interop/`)

Each file is the smallest document that exercises its id. Digests are the `sha256` of `test` already used in `trust_clean.json`; signatures reuse that fixture's structurally valid `did-web-v1` JWS values, so ai-catalog-validate's JWS-structure checks pass and only the property under test differs.

| File | Exercises | Content |
|---|---|---|
| `unknown-members-every-kind.json` | #unrecognized-members, ADR-003 law (R2-06) | an unknown `zz` member on all 11 object kinds (catalog, host, signature (on the catalog and on the manifest), entry, policy-urls extension value, publisher, manifest, subject, trust schema, attestation, provenance), every other member valid |
| `empty-arrays.json` | RD-03 | empty `tags`, `trustManifests`, `attestations`, `provenance` and `verificationMethods` |
| `data-null.json` | RD-02, AMB-09 | one entry with `"data": null` |
| `url-empty.json` | RD-02 | one entry with `"url": ""` |
| `nested-signed-manifest.json` | #nested-catalog-entries, R2-06 | an entry whose `data` is a catalog holding an entry with a digest and a signed Trust Manifest |
| `missing-entries.json` | RD-01 | `{"specVersion":"1.0"}` |
| `duplicate-member.json` | RD-04 | a duplicate member name inside `data` |
| `spec-version-plus.json` | RD-14 | `"specVersion": "+1.0"` |
| `lone-surrogate.json` | AMB-82 | `"\ud800"` in an `identifier` |
| `digest-uppercase.json` | RD-13 | an entry `digest` with uppercase hex |
| `contributor-not-rfc3986.json` | RD-12 | `contributor` `urn:acme|team` (`|` is outside RFC 3986, inside `\S+`) |

`SOURCE.md` lists these 11 files; `every_interop_fixture_is_listed` fails on a file missing from it or from §8.3.

#### 8.3 Per-file outcomes (observed with ai-catalog-rust 0.4.0)

`sdk_interop` holds this table once, as `(file, ours, theirs, decided_by)`, and asserts both observed outcomes equal the row. A row whose outcomes differ must carry an RD id; the test fails on a disagreement without one, and on an RD row whose outcomes agree (a stale divergence).

| File | Ours | Theirs | Decided by |
|---|---|---|---|
| `spec/examples/ai-catalog.json`, blocks 01–08, 10, 11, 13–15, 17, 18 | accept | accept | agree |
| `spec/blocks/12-format-and-key-naming.json` | reject (parse, `ArtifactSourceMissing`) | reject (`EntryArtifact`) | agree (AMB-83) |
| `spec/blocks/16-…-nested-catalog.json` | reject (`digest-invalid`, placeholder `sha256:99998888...`) | reject (`DigestInvalid`) | agree (ADR-006) |
| `upstream/catalog.json`, `discoverable.json`, `future_member.json`, `minimal.json`, `nested_max.json`, `spec-example.json`, `trust_clean.json`, `unsigned_trust.json` | accept | accept | agree |
| `upstream/invalid.json` | reject (parse, `MissingMember` at `/host/displayName`) | reject | agree |
| `upstream/manifest_shapes.json` | reject (parse, `SpecVersionInvalid`) | reject | agree (not a catalog, §2) |
| `upstream/weak_signature.json` | reject (`digest-weak`) | reject | agree; conformance differs (RD-06) |
| `upstream/nested_deep.json` | accept (`nested-depth-limit` warning, `Undetermined`) | reject (`NestedDepth`) | **RD-08**, #nested-catalog-entries "SHOULD impose a maximum nesting depth … A depth limit of 4 is RECOMMENDED" |
| `upstream/profile.json` | accept | reject (`SignatureB64`, `SignatureMalformed`, `SignatureForm`) | **RD-15**, #signed-payload-and-jws-construction, #verification-result, M12, M13 |
| `interop/unknown-members-every-kind.json`, `empty-arrays.json`, `nested-signed-manifest.json` | accept | accept | agree |
| `interop/url-empty.json` | reject (`url-empty`) | reject (`EntryArtifact`) | agree |
| `interop/lone-surrogate.json` | reject (parse, `LoneSurrogate`) | reject (parse) | agree (AMB-82) |
| `interop/data-null.json` | accept | reject (`EntryArtifact`) | **RD-02**, #catalog-entry (`data` "any JSON value") |
| `interop/missing-entries.json` | reject (parse, `MissingMember` at `/entries`) | accept | **RD-01**, #top-level-structure MUST |
| `interop/duplicate-member.json` | reject (parse, `DuplicateMember`) | accept | **RD-04**, #signed-payload-and-jws-construction |
| `interop/spec-version-plus.json` | reject (parse, `SpecVersionInvalid`) | accept | **RD-14**, #version-format |
| `interop/digest-uppercase.json` | reject (`digest-invalid`) | accept | **RD-13**, #digest-format |
| `interop/contributor-not-rfc3986.json` | reject (`contributor-not-absolute-uri`) | accept | **RD-12**, #contributor-identity "`contributor` MUST be an absolute URI [[RFC3986]]" |

Interpretation recorded: the spec defines conformance, not implementation agreement, so where 0.4.0 departs from the spec text the expected row follows the spec and names the RD; nothing in our model or validator is changed to match 0.4.0.

#### 8.4 Laws and pins

For every corpus input that both implementations parse (`parse` and `parse_slice` both `Ok`), whatever the validation outcome:

1. **Ours → theirs.** `ai_catalog::parse_slice(to_canonical_vec(&c)?) == parse_slice(b)` (their `PartialEq`).
2. **Theirs → ours.** `parse(ai_catalog::to_string(&t)?) == parse(b)` (our `PartialEq`), except the rows decided by RD-02 (`data-null.json`: their writer drops `data`, so ours refuses with `ArtifactSourceMissing`) and RD-03 (`empty-arrays.json`: their writer drops the empty arrays). Those two are asserted as the observed inequality, in their own tests.
3. **Canonical bytes (ADR-003 law).** For every corpus input `b` that `parse` accepts and whose numbers are all finite doubles, `to_canonical_vec(&parse(b)?)? == ai_catalog_trust::canonicalize(b)?.into_bytes()`. `b` is always the original (or §3-wrapped) bytes, never our output, so a member that `parse` drops fails the law. The set of corpus files excluded for a number that is not a finite double (neither implementation writes one) is empty at this revision; a file our writer refuses with `NotIJson` and that is not in the exclusion list fails the test. Both write every finite number in its RFC 8785 form (ADR-003 M20; RD-40 closed), pinned by `numbers_canonicalise_as_ai_catalog_trust_does`.

RD and AMB pins, each a function of `sdk_interop` that asserts both implementations' observed behaviour:

| Id | Spec anchor | Function |
|---|---|---|
| RD-01 | #top-level-structure | `missing_required_member_is_refused_by_us_and_defaulted_by_them` |
| RD-02 | #catalog-entry | `null_data_is_an_artifact_source_and_their_writer_drops_it` |
| RD-03 | #signed-payload-and-jws-construction (JCS of the object as given: dropping an empty array changes the signed bytes) | `empty_arrays_survive_our_round_trip_and_not_theirs` |
| RD-04 | #signed-payload-and-jws-construction | `duplicate_member_is_refused_by_us_and_last_wins_for_them` |
| RD-06 | #conformance-levels | `document_with_errors_is_not_met_where_they_report_minimal` (`weak_signature.json`: ours `Conformance::NotMet`, theirs `ConformanceLevel::Minimal` with `is_valid == false`) |
| RD-08 | #nested-catalog-entries | `over_depth_inline_nesting_is_a_warning_for_us_and_an_error_for_them` |
| RD-12 | #contributor-identity | `contributor_outside_rfc3986_is_refused_by_us_only` |
| RD-13 | #digest-format | `uppercase_digest_hex_is_refused_by_us_only` |
| RD-14 | #version-format | `plus_signed_spec_version_is_refused_by_us_only` |
| RD-15 | #signed-payload-and-jws-construction, #verification-result | `jws_structure_is_never_a_document_error` (`profile.json` entries 11–13) |
| RD-16 | #the-did-web-signer-profile, #verification-result | `did_web_signer_shape_is_never_a_document_diagnostic` (`profile.json`: they warn `ProfileSigner`, `ProfileAlgorithm`, `ProfileKeyId`, `ProfileHeader`; our report has no diagnostic at any `signature/signer` or `signature/jws` pointer) |
| RD-35 | — (no OCI format) | `stale_upstream_spec_example_is_an_upstream_fixture` (its bytes differ from `spec/examples/ai-catalog.json`; it is listed under `upstream/` only) |
| AMB-82 | #signed-payload-and-jws-construction (I-JSON) | `both_refuse_an_unpaired_surrogate_escape` |
| RD-40 (closed: agreement) | #signed-payload-and-jws-construction (JCS) | `numbers_canonicalise_as_ai_catalog_trust_does` (every finite number in its RFC 8785 form, `9007199254740993` as `9007199254740992`; `1e400` refused by both) |

#### 8.5 One corpus list

The fixture list and the §3 wrapping rules move from `tests/spec_examples.rs` to `tests/common/spec_corpus.rs` (`pub const SPEC_FIXTURES`, `pub fn wrapped(name) -> Vec<u8>`), and both `spec_examples` and `sdk_interop` use them; the copy in `spec_examples.rs` is deleted in the same slice. `sdk_interop` reads the upstream and interop directories by listing them, so a vendored file without a §8.3 row fails `every_corpus_file_has_an_outcome_row`.

## Requirement → test

Test paths are under `crates/turul-ai-catalog/tests/`.

| Requirement / decision | Spec text (short) | Test function |
|---|---|---|
| example-hierarchical-catalog-1 | "A catalog MAY contain both direct artifact entries and nested catalog entries" | `spec_examples::hierarchical_example_mixes_direct_and_nested_entries` |
| AMB-66 | full documents parse | `spec_examples::full_document_examples_parse` |
| AMB-66 | fragments parse under their rule | `spec_examples::fragment_examples_parse_when_wrapped` |
| AMB-66 | each fixture has exactly one class; only block 09 is exempt | `spec_examples::every_spec_fixture_is_classified` |
| AMB-83 | #format-and-key-naming example refused | `spec_examples::format_and_key_naming_example_lacks_an_artifact_source` |
| ADR-003 law 1 | every parsed spec example round-trips through the canonical writer (block 12 refused at parse; block 09 is a payload vector) | `spec_examples::every_parsed_spec_example_round_trips` |
| conformance-levels-1 | declared level (§4) | `conformance_level` (ADR-005, S4) |
| M4 / D9 | every corpus file has a §8.3 row | `sdk_interop::every_corpus_file_has_an_outcome_row`, `sdk_interop::every_interop_fixture_is_listed` |
| M4 | accept/reject agrees, or the row names an RD id | `sdk_interop::accept_reject_agrees_or_is_a_recorded_divergence` |
| M4 | ours → theirs: equal values (§8.4 law 1) | `sdk_interop::our_canonical_bytes_parse_equal_in_ai_catalog` |
| M4 | theirs → ours: equal values, except RD-02/RD-03 (§8.4 law 2) | `sdk_interop::their_serialisation_parses_equal_in_ours` |
| ADR-003 law (R2-06) | `to_canonical_vec(parse(b)) == ai_catalog_trust::canonicalize(b)` on original bytes | `sdk_interop::canonical_bytes_equal_ai_catalog_trust_canonicalize` |
| #unrecognized-members | unknown members on all 11 kinds cross both implementations | `sdk_interop::unknown_members_on_every_kind_cross_both_ways` |
| #nested-catalog-entries | inline nested catalog with a signed manifest crosses both ways | `sdk_interop::nested_catalog_with_signed_manifest_crosses_both_ways` |
| RD-01, -02, -03, -04, -06, -08, -12, -13, -14, -15, -16, -35, AMB-82 | §8.4 pins | the functions named in §8.4 |

## Owning symbols and entrypoints

| Artifact | Owner | Entrypoint |
|---|---|---|
| Spec-example corpus | `crates/turul-ai-catalog/tests/fixtures/spec/` with `SOURCE.md` | `tests/spec_examples.rs` |
| Upstream corpus | `crates/turul-ai-catalog/tests/fixtures/upstream/` with `SOURCE.md` | `tests/sdk_interop.rs` |
| Interop corpus | `crates/turul-ai-catalog/tests/fixtures/interop/` with `SOURCE.md` | `tests/sdk_interop.rs` |
| Spec fixture list and wrapping rules | §3, implemented once as `SPEC_FIXTURES` and `wrapped` in `tests/common/spec_corpus.rs` | used by `tests/spec_examples.rs` and `tests/sdk_interop.rs` |
| Expected cross-implementation outcomes | §8.3, held once as the outcome table in `tests/sdk_interop.rs` | `accept_reject_agrees_or_is_a_recorded_divergence` |

No derived artifact with golden bytes belongs to this ADR.

## Consequences

- Plan drift recorded: PLAN S1's criterion "Every spec example parses" does not hold for block 12. The spec MUST decides, and the refusal is pinned (AMB-83). PLAN AMB-66 lists the policy-urls excerpt as exempt; here it is parsed under `entry-merge`, which keeps its bytes and matches AMB-66's "its `extensions` value is wrapped instead".
- A new spec revision changes §1's date, the corpus and the extraction ordinals in one slice, together with this ADR's revision log.
- A new ai-catalog-rust version changes §1, the `=0.4.0` pins, the vendored `upstream/` files and §8.3 in one slice. An RD that the new version fixes makes its row agree, so the test fails until the row and the RD are removed.
- `unknown-members-every-kind.json` covers PLAN S7's 11 object kinds with 12 `zz` members: the Signature kind appears both on the catalog and on the manifest, because a signed manifest is the case R2-06 names.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created for S1: sources, licences, corpus classification and wrapping rules, declared level, maintainer decisions, RD index. AMB-83 added. S2 adds the round-trip half of `spec_examples`; S4 adds validation outcomes; S7 adds `sdk_interop` and closes this ADR. |
| 2026-11-17 | S1 review: the upstream corpus (§2) is vendored in S1 as D9 requires: the 13 ai-catalog-rust 0.4.0 `fixtures/` files byte-for-byte under `tests/fixtures/upstream/` with `SOURCE.md`. Workspace `rust-version` and the CI MSRV job set to 1.95 (D14, M7). |
| 2026-11-17 | S2: round-trip half of `spec_examples` named (`every_parsed_spec_example_round_trips`, ADR-003 law 1). RD-40 (ai-catalog-trust number canonicalisation) is decided in ADR-003 and pinned by `i_json_numbers`; S7 excludes those documents from the canonicalize law. |
| 2026-11-17 | S7: §8 SDK interop added. `=0.4.0` dev-dependencies on `ai-catalog`, `ai-catalog-validate`, `ai-catalog-trust` (D15). Accept/reject defined (§8.1). Interop corpus of 11 fixtures (§8.2). Per-file outcomes observed against 0.4.0 (§8.3): disagreements are RD-01, RD-02, RD-04, RD-08, RD-12, RD-13, RD-14, RD-15; blocks 12 and 16 are refused by both. Cross round-trip and canonicalize laws (§8.4), with RD-02 and RD-03 as the only theirs → ours inequalities; the RD-40 exclusion set is empty. Spec fixture list and wrapping rules move to `tests/common/spec_corpus.rs` (§8.5). |
| 2026-11-17 | M20 (ADR-003 AMB-94): our writer is exactly RFC 8785, so RD-40 is closed: `serde_jcs` 0.2 also writes integers through `f64`, and both give the same bytes for every finite number. §6 drops RD-40; §8.4 law 3 excludes only numbers that are not finite doubles; `sdk_interop::numbers_canonicalise_as_ai_catalog_trust_does` added. |
| 2026-11-17 | §7 M2 marked superseded in part by M19: the Lambda serves a store-assembled catalog, not a bundle (ADR-016). |
