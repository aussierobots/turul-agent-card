# ADR-006: Trust Manifest structure and Manifest Validity

- **Status:** Accepted (2026-11-17)
- **Date:** 2026-11-17
- **Implements:** under `crates/turul-ai-catalog/tests/`: `manifest_validity.rs`, `subject_binding.rs`, `signature_object.rs`.
- **Depends on:** ADR-005 (`Report`, `Diagnostic`, `Code`, strict scope AMB-111, `Timestamp`, `DigestValue::parse`, `extensions::is_valid_key`, the `uri` module; S5 adds `uri::is_absolute_uri` there, per ADR-005's owner table), ADR-004 (`EntryIdentifier`, `ArtifactType`, exact equality), ADR-002 (model, M15 `MissingMember`). Binding decisions: M12 (the format crate never decodes `jws`), M13 and M15 (below), D2.
- **Spec:** https://ai-catalog.io, `specification/ai-catalog.md` as read in ADR-001 (last commit 2026-11-10).

## Context

The spec constrains Trust Manifests beyond their CDDL: "The normative prose additionally constrains contributor URIs, manifest substance, subject binding, timestamp validity, and signature verification" (#cddl-schema). The first four are document rules this ADR assigns to the format crate. Signature verification, and everything the spec makes a condition of *relying* on a signature (profile support, header, `did:web` shape, key, time window, artifact bytes), is the trust crate's (ADR-012 to ADR-015), which composes the predicates defined here and never re-implements them.

Spec anchors implemented: #trust-manifest, #contributor-identity, #independent-contributions-and-updates (structure), #manifest-validity, #optional-members (`subject`), #subject-object, #signature-object (own members), #profile-selection (identifier form only), #signature-acceptance-and-time (timestamp syntax and instant order only), #entry-release-coverage (structural binding only), #level-1-minimal-catalog (Manifest Validity at any level), #level-3-trusted-catalog (structure of additional contributions), #the-did-web-publisher-profile (the release-only manifest is valid).

### M13 and M15

- M13: a malformed or failing optional signature is a warning-severity non-endorsement, never a document error. It covers JWS header, `did:web` signer-shape and verification defects only (M15). None of those is decided here. `validate` never reads the content of `jws` (M12), never applies `did:web` rules to `signer`, and reads no clock, so a malformed header, `signer: "did:web:Example.com"`, an empty `jws` or an expired signature gives no document diagnostic; the trust crate reports each as a `Reason` (ADR-012, ADR-015).
- M15: a Signature object missing `signer`, `profile`, `issuedAt` or `jws` is a `ParseError` (ADR-002). Every other own-member check below keeps its severity (error, except `profile-unrecognised`): they are document MUSTs on the Signature object, not header, signer-shape or verification defects, and an error makes the document `NotMet` and fails lambda cold start (D18).

## Decision

### 1. Scope and entrypoints

- `validate_manifest(&TrustManifest) -> Report` is the one owner of manifest-scope and signature-scope diagnostics. Pointers are relative to the manifest (the manifest itself is the empty pointer); `Report::conformance` is `None` (ADR-005). It validates a standalone manifest, which "lets the signed manifest travel independently of the entry" (#optional-members), and ADR-015 calls it for `SignedObject::Manifest`.
- `validate` calls the same crate-private function for every manifest of every entry, at every catalog level, with the manifest's pointer as prefix and the strictness its enclosing document passes down (ADR-005 AMB-111: a manifest inside a signed catalog root, or inside the `data` of a digested entry, is strict even when unsigned).
- The signature-scope checks (§3) are one crate-private function, called for a manifest's `signature` by the manifest checks and for the catalog root's `signature` by `validate` (AMB-133). A root signature has no `contributor` or `subject` rule.
- Entry-scope binding (§4) needs the containing entry, so only `validate` reports it (AMB-130). Hence, for every manifest `m` at pointer `P`: `validate`'s diagnostics at or under `P`, leaving out `subject-mismatch` and with I-JSON severities as `validate_manifest` would give them for an unsigned scope, equal `validate_manifest(m)` with `P` prefixed (S5 criterion, refined).

### 2. Manifest checks (`validate_manifest`)

| Code | Severity | Pointer | Rule (spec) |
|---|---|---|---|
| `contributor-not-absolute-uri` | error | `/contributor` | not an RFC 3986 `absolute-URI` (#contributor-identity "MUST be an absolute URI"; AMB-71: no fragment) |
| `manifest-not-substantive` | error | `` (the manifest) | none of `signature`, `trustSchema`, non-empty `attestations`, non-empty `provenance`, non-empty `extensions` (#manifest-validity; AMB-32: `subject`, `additionalSignatures` and unknown members do not count). This is also "Empty manifests MUST be omitted" |
| `manifest-subject-missing` | error | `/subject` | `signature` present, `subject` absent (#manifest-validity, #optional-members "REQUIRED when `signature` is present") |
| `extension-key-invalid` | error | `/extensions/<key>` | ADR-005 §7 rule |
| `digest-invalid`, `digest-weak`, `digest-algorithm-unknown` | ADR-005 §5 | `/subject/digest`, `/attestations/k/digest`, `/provenance/k/sourceDigest` | `DigestValue::parse` |
| `number-not-i-json`, `number-inexact`, `string-not-i-json` | ADR-005 §5 | the value | `number-not-i-json` and `string-not-i-json` are strict when the manifest carries `signature` (its payload: the manifest minus `signature.jws` and `additionalSignatures`) or the enclosing document says so; `number-inexact` is always a warning (M20) |
| `signer-contributor-mismatch` | error | `/signature/signer` | `signature.signer` ≠ `contributor` (#contributor-identity "MUST exactly equal"; "case-sensitively, without URI normalization") |

`TrustManifest::signer_matches_contributor(&self) -> bool` is the one equality: `true` exactly when `signature` is present and `signer == contributor` as decoded strings, byte for byte. `validate_manifest` reports a mismatch through it; ADR-015 composes it into Entry Release Coverage. No other member is checked: `trustSchema`, `attestation.uri`, `governanceUri`, `statementUri` and friends are opaque text, and attestations and provenance are never fetched or verified (M9; stale-attestations-2/3 OOS).

### 3. Signature own-member checks (#signature-object, #profile-selection, #signature-acceptance-and-time)

| Code | Severity | Pointer | Rule |
|---|---|---|---|
| `signer-not-absolute-uri` | error | `…/signature/signer` | not an RFC 3986 `absolute-URI` (#signature-object `signer`; AMB-71) |
| `profile-empty` | error | `…/signature/profile` | `""` (#signature-object "A non-empty string"; AMB-25) |
| `profile-unrecognised` | warning | `…/signature/profile` | neither exactly a member of `RESERVED_PROFILES` nor an RFC 3986 `absolute-URI` (AMB-26) |
| `timestamp-invalid` | error | `…/signature/issuedAt`, `…/signature/expiresAt` | ADR-005 §4 grammar (#signature-acceptance-and-time "MUST validate timestamp syntax") |
| `signature-expiry-order` | error | `…/signature/expiresAt` | both timestamps valid and `expiresAt ≤ issuedAt` as `Timestamp` instants (#signature-object "MUST denote an instant later than `issuedAt`"; "compare timestamps as instants, not strings") |

- `validate::DID_WEB_V1 = "did-web-v1"` and `validate::RESERVED_PROFILES = [DID_WEB_V1]`. A profile is compared exactly and case-sensitively, with no URI normalisation (#profile-selection), so `DID-WEB-V1` is a warning and `HTTPS://example.com/p` is not `https://example.com/p`. Any other non-empty value is a warning, never an error: a later 1.x minor may reserve it (#compatibility-rules), and an unsupported profile only means the signature does not count (#profile-selection), which ADR-015 decides.
- When either timestamp is invalid, only `timestamp-invalid` is reported; no order is computed (AMB-135).
- `jws` is checked only as CDDL `text` (ADR-002). Its content, and an empty `jws`, are never a document diagnostic (M12, M13; AMB-132).
- Unknown members inside `signature` are kept and ignored (ADR-002 AMB-25).

### 4. Subject binding (#subject-object, #entry-release-coverage)

- `Subject::binds(&self, &CatalogEntry) -> bool` is the one structural binding: `identifier == entry.identifier` (`EntryIdentifier`, exact), `artifact_type == entry.artifact_type` (`ArtifactType`, exact, ADR-004 AMB-98), `Some(&digest) == entry.digest.as_ref()` compared as exact strings, never through `DigestValue` (AMB-131: `SHA256:…` ≠ `sha256:…`, and an entry without `digest` never binds), and `version == entry.version` as `Option<String>`, so both absent or both present and byte-equal, with `""` present (RD-09). It compares one subject with one entry, so fields from different manifests can never combine.
- `Subject::for_entry(&CatalogEntry) -> Option<Subject>` is its inverse: `None` when the entry has no `digest`, otherwise the entry's `identifier`, `type`, `digest` and `version` (present exactly when the entry's is). Law: `Subject::for_entry(e).is_none_or(|s| s.binds(e))`, and `Some` whenever `e.digest` is. The signer (ADR-013) builds subjects only through it.
- `validate`, for each manifest with a `subject`, signed or not (the #subject-object MUSTs apply to every subject; RD-17), reports one `subject-mismatch` error per failing member at `…/subject/identifier`, `…/subject/type`, `…/subject/digest` or `…/subject/version` (the absent member's pointer when one side lacks `version`).
- `entry-digest-missing` is an error at `/entries/i/digest` when the entry has no `digest` and any of its manifests carries `signature` (#subject-object "which MUST be present when an entry carries a signed Trust Manifest"; #entry-release-coverage "The entry MUST contain `digest`"). Such a manifest also gets `subject-mismatch` at `…/subject/digest`; both are kept because they break different MUSTs.
- Binding here is structural. Reliance additionally needs a verified signature and artifact integrity, in the AMB-20 order, which ADR-015 owns (entry-release-coverage-7).

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Manifest-scope and signature-scope diagnostics | `validate::validate_manifest` (crate-private manifest and signature checks inside) | `validate_manifest`; `validate` per manifest and for the root `signature` |
| Signer = contributor | `TrustManifest::signer_matches_contributor` | `validate_manifest`; trust `entry_release_coverage` (ADR-015) |
| Structural subject binding | `Subject::binds` | `validate`; trust `entry_release_coverage` |
| Subject for an entry | `Subject::for_entry` | trust `sign_manifest` (ADR-013) |
| Reserved profiles | `validate::{DID_WEB_V1, RESERVED_PROFILES}` | `validate_manifest`; trust profile selection (ADR-015) |

The diagnostic vocabulary, `Report`, `Timestamp`, `DigestValue`, the extension-key rule, the I-JSON scope and the RFC 3986 grammar are ADR-005's; this ADR calls them. No golden bytes or schema digest belong here (D17).

## Requirement → test

Test paths are under `crates/turul-ai-catalog/tests/`.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| contributor-identity-1 | `contributor` "MUST be an absolute URI [[RFC3986]]" | `manifest_validity::contributor_must_be_an_rfc_3986_absolute_uri` |
| contributor-identity-5 | `signature.signer` "MUST exactly equal `contributor`" | `signature_object::signer_must_equal_contributor` |
| contributor-identity-6 | "compare … case-sensitively, without URI normalization" | `signature_object::signer_and_contributor_compare_without_normalisation` (`did:web:Acme.com` vs `did:web:acme.com`, `%41` vs `A`) |
| independent-contributions-and-updates-1 | "MAY contain multiple manifests from the same contributor, including … different verification profiles" | `manifest_validity::several_manifests_from_one_contributor_are_valid` |
| independent-contributions-and-updates-2 | "A contributor MAY publish an updated Trust Manifest for the same release" | `manifest_validity::updated_manifest_for_the_same_release_is_valid` |
| manifest-validity-1 | "MUST contain at least one of …" | `manifest_validity::manifest_without_substance_is_an_error` (each substantive member alone passes; `subject`, `additionalSignatures`, unknown members, `[]` and `{}` alone fail) |
| manifest-validity-2 | "A manifest containing a `signature` MUST also contain `subject`" | `manifest_validity::signed_manifest_without_subject_is_an_error` |
| manifest-validity-3 | "A signed manifest MAY endorse only the artifact release identified by that subject" | `manifest_validity::release_only_signed_manifest_is_valid` |
| manifest-validity-4 | "Empty manifests MUST be omitted" | `manifest_validity::manifest_with_only_contributor_is_an_error` |
| optional-members-1 | `subject` "REQUIRED when `signature` is present"; "travel independently" | `manifest_validity::signed_manifest_without_subject_is_an_error`, `manifest_validity::standalone_manifest_is_validated_alone` |
| subject-object-2 | `subject.identifier` "MUST exactly equal the containing entry's `identifier`" | `subject_binding::identifier_mismatch_is_an_error` |
| subject-object-3 | `subject.type` "MUST exactly equal the containing entry's `type`" | `subject_binding::type_mismatch_is_an_error` |
| subject-object-4 | `subject.digest` "MUST exactly equal the containing entry's `digest`, which MUST be present when an entry carries a signed Trust Manifest" | `subject_binding::digest_mismatch_is_an_error`, `subject_binding::signed_manifest_on_entry_without_digest_is_an_error` |
| subject-object-5 | `subject.version` "present exactly when `entry.version` is present, and its value MUST exactly equal" | `subject_binding::version_presence_and_value_must_match` |
| signature-object-7 | `expiresAt` "MUST denote an instant later than `issuedAt`" | `signature_object::expires_at_not_later_than_issued_at_is_an_error` (equal instants; `+10:00` vs `Z`) |
| profile-selection-4 | "Independently defined profiles MUST use absolute URI identifiers" | `signature_object::profile_is_reserved_or_absolute_uri` (`https://example.com/profiles/v1` valid; `did-web-v2` warning; `""` error) |
| profile-selection-5 | "Profile identifiers MUST be compared exactly and case-sensitively" | `signature_object::profile_is_compared_exactly` (`DID-WEB-V1` warning) |
| signature-acceptance-and-time-2 | "MUST validate timestamp syntax and compare timestamps as instants" | `signature_object::signature_timestamps_are_validated_and_compared_as_instants` |
| entry-release-coverage-1 | "verified Signature object whose `signer` exactly equals the manifest's `contributor`" (structural half) | `signature_object::signer_must_equal_contributor` |
| entry-release-coverage-2 | signed `subject` `identifier`, `type`, `digest` "exactly equal the corresponding entry fields" | `subject_binding::for_entry_binds_its_entry`, `subject_binding::each_member_mismatch_breaks_binding` |
| entry-release-coverage-3 | "The entry MUST contain `digest`" | `subject_binding::signed_manifest_on_entry_without_digest_is_an_error`, `subject_binding::entry_without_digest_has_no_subject` |
| entry-release-coverage-4 | `version` both absent, or both present and equal | `subject_binding::version_presence_and_value_must_match` |
| entry-release-coverage-5 | comparisons "MUST be case-sensitive, without URI normalization" | `subject_binding::binding_compares_exact_strings` |
| entry-release-coverage-6 | "MUST NOT combine fields from different manifests" | `subject_binding::fields_from_two_manifests_never_combine` |
| level-1-minimal-catalog-2 | each manifest "when present at any level, MUST satisfy Manifest Validity" | `manifest_validity::manifest_validity_applies_in_inline_nested_catalogs` |
| level-3-trusted-catalog-2 | "Additional contributors MAY provide their own Trust Manifests" | `manifest_validity::manifests_from_several_contributors_are_valid` |
| level-3-trusted-catalog-6 | "MAY include signed attestations, provenance, and manifest extensions" | `manifest_validity::signed_manifest_with_attestations_provenance_and_extensions_is_valid` |
| the-did-web-publisher-profile-6 | "MAY issue a signed manifest containing only its contributor identity, subject, and signature" | `manifest_validity::release_only_signed_manifest_is_valid` |

Decisions owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| RD-09 (binding half; key half ADR-004, uniqueness half ADR-005) | `""` is a present version for binding | `subject_binding::version_presence_and_value_must_match` (`""` vs absent mismatches, `""` vs `""` binds) |
| RD-12 | RFC 3986 `absolute-URI` grammar, not a regex | `manifest_validity::contributor_must_be_an_rfc_3986_absolute_uri` (`urn:x:<bad>`, `urn:x:a"b`, `did:web:é.example`, `urn:x:%zz`, `http://[::1/x`, `a:b#c` refused; `foo:` accepted), `signature_object::signer_must_be_an_rfc_3986_absolute_uri` |
| RD-17 | binding checked whenever `subject` is present (agree) | `subject_binding::unsigned_manifest_subject_is_bound_too` |
| RD-50 | subject compared even when the entry has no `digest` | `subject_binding::digest_mismatch_is_an_error` (entry without `digest`) |
| RD-51 | empty `jws` and empty subject members are not emptiness errors | `signature_object::jws_content_and_did_web_shape_are_never_document_diagnostics`, `subject_binding::binding_compares_exact_strings` |
| RD-52 | `profile-unrecognised` warning | `signature_object::profile_is_reserved_or_absolute_uri` |
| AMB-20 (structural half; order and composition ADR-015) | binding is one predicate per manifest | `subject_binding::for_entry_binds_its_entry` |
| AMB-25 (`profile` half) | empty `profile` is an error | `signature_object::profile_is_reserved_or_absolute_uri` |
| AMB-26 | `RESERVED_PROFILES`, absolute-URI valid, else warning | `signature_object::profile_is_reserved_or_absolute_uri`, `signature_object::profile_is_compared_exactly` |
| AMB-32 | substance members | `manifest_validity::manifest_without_substance_is_an_error` |
| AMB-71 (`contributor`, `signer`, `profile`) | `absolute-URI`, no fragment | `manifest_validity::contributor_must_be_an_rfc_3986_absolute_uri` (`did:web:x#k` refused), `signature_object::signer_must_be_an_rfc_3986_absolute_uri` |
| AMB-130 | entry-scope binding is `validate`'s; the S5 agreement law | `manifest_validity::validate_reports_validate_manifest_diagnostics_for_every_corpus_manifest`, `manifest_validity::standalone_manifest_is_validated_alone` |
| AMB-131 | subject digest compared as exact text | `subject_binding::binding_compares_exact_strings` |
| AMB-132 | `jws` content, empty `jws`, `did:web` shape and time window are never document diagnostics (M12, M13) | `signature_object::jws_content_and_did_web_shape_are_never_document_diagnostics`, `signature_object::expired_signature_has_no_document_diagnostic` |
| AMB-133 | root signature gets the signature-scope checks | `signature_object::root_signature_members_are_checked` |
| AMB-134 | strictness passed down to manifests | `manifest_validity::unsigned_manifest_in_a_signed_catalog_is_strict` |
| AMB-135 | invalid timestamp suppresses the order check | `signature_object::invalid_timestamp_gives_no_order_diagnostic` |
| Law | `Subject::for_entry(e).binds(e)`; `None` without `digest` | `subject_binding::for_entry_binds_its_entry` (with and without `version`), `subject_binding::entry_without_digest_has_no_subject` |

New ids (not in PLAN §6): RD-50 to RD-52 and AMB-130 to AMB-135, from a block reserved for this ADR.

- AMB-130: a standalone manifest has no entry, yet #subject-object binds a subject to "the containing entry". The binding rule therefore lives with the entry, in `validate`, and `validate_manifest` stays entry-free so that ADR-015 can use it for a standalone manifest. The S5 criterion "validate reports exactly validate_manifest's diagnostics" holds with `subject-mismatch` and inherited strictness set aside (§1).
- AMB-131: #subject-object and #entry-release-coverage say "exactly equal" and "decoded JSON strings"; normalising the algorithm case would be URI-style normalisation of a signed value.
- AMB-132: M12 and M13 as amended by M15.
- AMB-133: #signature-object: "A Trust Manifest or catalog root MAY contain one `signature` object", with one member list for both.
- AMB-134: ADR-005 AMB-111; a catalog signature "includes complete nested Trust Manifests" (#signed-payload-and-jws-construction).
- AMB-135: an order between an invalid and a valid timestamp is undefined.

## Divergences from ai-catalog-rust 0.4.0

| Id | ai-catalog-rust | This crate | Spec | Pinned by |
|---|---|---|---|---|
| RD-12 | `^[A-Za-z][A-Za-z0-9+.\-]*:\S+$` for `contributor` and `signer` | RFC 3986 `absolute-URI` | #contributor-identity, #signature-object | `manifest_validity::contributor_must_be_an_rfc_3986_absolute_uri`, `signature_object::signer_must_be_an_rfc_3986_absolute_uri` |
| RD-17 | binding checked whenever `subject` is present | same (agree) | #subject-object | `subject_binding::unsigned_manifest_subject_is_bound_too` |
| RD-50 | `subject_mismatches` skips `identifier`, `type` and `digest` when either side is `""`, and `digest` when the entry has none | every member compared; no entry `digest` never binds | #subject-object "MUST exactly equal" | `subject_binding::digest_mismatch_is_an_error` |
| RD-51 | `""` in `signer`, `profile`, `issuedAt`, `jws` and subject members is `*-member-missing`; `jws` content is checked in `validate` | `profile-empty`; `""` `signer` is `signer-not-absolute-uri`; `""` `issuedAt` is `timestamp-invalid`; `jws` never inspected here; subject `""` only compared | #signature-object (only `profile` is "non-empty"); M12, M13 | `signature_object::jws_content_and_did_web_shape_are_never_document_diagnostics` |
| RD-52 | no diagnostic for an unknown non-URI `profile` | `profile-unrecognised` warning | #profile-selection "MUST use absolute URI identifiers" | `signature_object::profile_is_reserved_or_absolute_uri` |
| RD-16 (document half; ADR-012 / ADR-015 decide) | `did:web` shape and time window are `validate` warnings | no document diagnostic | M12, M13 | `signature_object::jws_content_and_did_web_shape_are_never_document_diagnostics`, `signature_object::expired_signature_has_no_document_diagnostic` |

ai-catalog-rust agrees on: `manifest-not-substantive` with the same substance members, a missing `subject` beside `signature`, `signer` = `contributor` exact, the entry-digest requirement for a signed manifest, and `expiresAt` compared as an instant.

## Consequences

- One structural binding (`Subject::binds`), one signer equality and one manifest validator. ADR-013 signs only subjects from `Subject::for_entry`; ADR-015 relies on a manifest only after these predicates hold, and adds what the format crate cannot know (keys, header, time, bytes).
- Every check here is a document error or warning; none depends on network, clock or cryptography, so the lambda's cold-start gate (D18) and the CLI exit code see the same result for the same bytes.
- A signed manifest that breaks a structural MUST (no `subject`, signer ≠ contributor, subject mismatch, no entry `digest`) makes the document `NotMet`. M15 keeps this: these are not header, signer-shape or verification defects.
- Plan drift recorded: PLAN S5 states the `validate` / `validate_manifest` agreement without the entry-scope and inherited-strictness exceptions (AMB-130, AMB-134). PLAN §3 lists `profile-selection-4/5` and `signature-acceptance-and-time-2` without naming the codes; they are §3 here.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created for S5: `validate_manifest`, manifest and signature own-member checks, `TrustManifest::signer_matches_contributor`, `Subject::binds`, `Subject::for_entry`, `DID_WEB_V1`, `RESERVED_PROFILES`, entry-scope binding diagnostics. M13 / M15 scope recorded. RD-50 to RD-52 and AMB-130 to AMB-135 added. |
| 2026-11-17 | Depends-on corrected: `uri::is_absolute_uri` is added in S5 inside ADR-005's `uri` module, not pre-existing. |
| 2026-11-17 | §2 `sourceDigest` check kept (#digest-format "Digests in this specification use the format `algorithm:hex-value`" governs; CDDL `text` is the shape only). Spec block 16's truncated `"sha256:99998888..."` is therefore `digest-invalid` at `/entries/2/data/entries/2/trustManifests/0/provenance/0/sourceDigest` and the document is `NotMet`: `conformance_level::multi_artifact_example_has_placeholder_source_digest`. S4 I-JSON fixtures migrated to valid ADR-006 manifests and signatures. |
| 2026-11-17 | Accepted at S5 close (0.0.6): `manifest_validity.rs`, `subject_binding.rs` and `signature_object.rs` are present and green (`cargo test -p turul-ai-catalog`, 210 passed), and every manifest-scope and signature-scope `Code` is emitted only from `validate.rs` through `validate_manifest` and its crate-private helpers. |
| 2026-11-17 | M20 (ADR-003 AMB-94, ADR-005 §5): `number-inexact` is a warning in a signed manifest too; only `number-not-i-json` and `string-not-i-json` follow the strict context. The row's section reference corrected to ADR-005 §5. |
