# ADR-015: Signature acceptance and time, coverage, authority and the per-entry trust report (trust crate)

- **Status:** Accepted (T4 landed every test file below; every outcome in the trust crate is produced through `verify_signature` and `verify_resolved`; no catalog-wide trust verdict exists in the workspace)
- **Date:** 2026-11-17
- **Implements:** under `crates/turul-ai-catalog-trust/tests/`: `signature_acceptance.rs`, `profile_selection.rs`, `entry_coverage.rs`, `publisher_authorization.rs`, `catalog_endorsement.rs`, `manifest_selection.rs`, `trust_report.rs`.
- **Depends on:** ADR-005 (`validate`, `Report`, `Code`, `Timestamp` with `from_system_time` and instant `Ord`), ADR-006 (`validate_manifest`, `TrustManifest::signer_matches_contributor`, `Subject::binds`, `DID_WEB_V1`), ADR-008 (`FetchPolicy`), ADR-009 (`ResolvedCatalog` tree and `base`), ADR-012 (`DidWebRoot`, `UrnAir`, `header::{inspect, policy}`, `Kid`, `HeaderError`), ADR-013 (`manifest_payload`, `catalog_payload`, `verify_artifact`, `PayloadError`, `IntegrityError`), ADR-014 (`did_web::resolve`, `DidDocument`, `select_key`, `PublicJwk`, `ResolveError`, `KeySelectionError`). Binding decisions: D2 (Level 3 never claimed), D8, M10, M12, M13, M15. JWS API: `turul-jwt-validator` 0.3.4 `detached::{verify_detached, DetachedJwsError}`.
- **Spec:** `target/upstream/ai-catalog/specification/ai-catalog.md` (last commit 2026-11-10).

## Context

The spec separates authentication, acceptance, coverage and authority, and makes each a separate MUST:

- "Consumers MUST validate the Signature object's structure, profile selection, payload, JWS, and selected signer profile before accepting an endorsement … compare timestamps as instants … MUST NOT accept an endorsement before `issuedAt` or at or after `expiresAt` … MAY allow a small clock-skew tolerance … An invalid, unsupported, or expired signature MUST NOT count as a verified endorsement. Other manifests are evaluated independently … MUST limit authentication claims based on a Trust Manifest to its signed contents and successfully verified subject bindings" (#signature-acceptance-and-time).
- "Consumers MUST select the verification procedure named by `profile`, and MUST require that it is both supported and permitted by local policy. They MUST NOT infer another profile … or fall back … The selected profile MUST authenticate the exact `signer` value … Consumers MUST also apply the coverage and authority checks required for the intended endorsement" (#profile-selection); "Consumers MUST NOT authenticate an identity using an unsupported profile" (#verification-procedures).
- Entry Release Coverage needs a verified signature whose `signer` equals `contributor`, a subject equal to the entry, and artifact bytes that match `entry.digest` (#entry-release-coverage). Publisher authorization: the `urn:air` publisher component, under the root-`did:web` domain rule, must give exactly the signer; "Subdomain, suffix, and organizational-ownership heuristics MUST NOT be used" (#the-did-web-publisher-profile). Catalog endorsement needs "A separate profile or configured policy [that] MUST identify authorized operators" (#catalog-authorization).
- "Failure MUST NOT be treated as a verified endorsement. Consumers MAY retain or display unverified content" (#verification-result). Updates: "SHOULD prefer the manifest with the most recent acceptable signature, comparing authenticated `issuedAt` instants … MUST NOT infer an ordering from array order" (#independent-contributions-and-updates).

ai-catalog-rust 0.4.0 reduces this to a catalog-wide `Trusted` level where one failure blocks everything (RD-07). D2 forbids claiming Level 3, and M13 makes every signature defect a non-endorsement of that signature only. So this ADR's output is one outcome per signature, never a level.

Spec anchors implemented: #verification-procedures, #profile-selection, #signature-acceptance-and-time, #verification-result, #entry-release-coverage (reliance), #jws-algorithm-and-key-requirements (verification), #the-did-web-signer-profile (profile scope), #the-did-web-publisher-profile, #catalog-authorization, #publisher-and-policy-metadata, #independent-contributions-and-updates (selection), #contributor-identity (no inferred authentication), #level-3-trusted-catalog (consumer MUSTs), #trust-layers, #stale-attestations.

## Decision

### 1. Inputs

- `TrustPolicy { profiles: Vec<String>, clock_skew: Duration, operators: Vec<DidWebRoot>, fetch: FetchPolicy }`, `Default` = `profiles: [DID_WEB_V1]`, `clock_skew: 0` (AMB-24, the strictest reading), `operators: []` (AMB-34), `fetch: FetchPolicy::default()`. Stricter local policy is expressed by these fields, never by another profile name (#profile-selection: "MAY impose stricter local acceptance requirements without defining another profile").
- `SignedObject<'a>::{Manifest(&'a TrustManifest), Catalog(&'a AiCatalog)}`, with `signature()`. It is the one carrier of "kind of containing object" for `header::policy` (ADR-012) and for the payload (ADR-013).
- `now: SystemTime`, supplied by the caller (AMB-334).

### 2. `verify_signature`

`verify_signature(object: SignedObject<'a>, doc: &DidDocument, policy: &TrustPolicy, now: SystemTime) -> Result<AcceptedSignature<'a>, Reason>` runs these steps in this order and returns the first failure (AMB-331). Steps 0 to 4 are the crate-private `precheck(object, policy) -> Result<(DidWebRoot, Kid), Reason>`, shared with `verify_resolved` and `verify`, so no key, document or network work happens for a signature that fails them.

| Step | Check | Owner (layer) | Failure |
|---|---|---|---|
| 0 | the object has a `signature` | — | `Reason::Unsigned` |
| 1 | no error diagnostic in the signed object: `validate_manifest(m)` for `Manifest` (so a standalone manifest is covered), `validate(c)` for `Catalog` (AMB-332) | ADR-005, ADR-006 | `Reason::Invalid(Code)`, the first error's code in report order |
| 2 | `profile` equals `DID_WEB_V1` exactly, else unsupported; and is in `policy.profiles`, else disallowed. No fallback, no inference from `signer`, header or anything else | here | `Reason::Unsupported(profile)`, `Reason::Disallowed` |
| 3 | `DidWebRoot::parse(signer)` | ADR-012 | `Reason::SignerShape` |
| 4 | `header::inspect(object, signature)`: structure, `alg`, `typ`, `b64`, `crit`, forbidden members (the validator's `inspect_detached` under `header::policy(object)`), then `kid` = signer + `#` + fragment | ADR-012, validator | `Reason::Header(HeaderError)` |
| 5 | `manifest_payload` / `catalog_payload` | ADR-013 | `Reason::Payload(PayloadError)` |
| 6 | `doc.id()` equals the signer root | ADR-014 | `Reason::Resolve(Some(ResolveError::IdMismatch))` |
| 7 | `select_key(doc, &kid)` | ADR-014, validator (`check_public_jwk`) | `Reason::Key(KeySelectionError)` |
| 8 | `verify_detached(jws, payload, key, &header::policy(object))`; its returned header is ignored | validator | `Reason::Signature(DetachedJwsError)` |
| 9 | time, on the now-authenticated `issuedAt` / `expiresAt`: with `late = Timestamp::from_system_time(now + skew)` and `early = Timestamp::from_system_time(now − skew)`, require `issuedAt ≤ late` and, when present, `early < expiresAt` (= `issuedAt − skew ≤ now < expiresAt + skew`); a shifted instant that `SystemTime` or `Timestamp` cannot represent is replaced by `now` unshifted, which is stricter, never looser | here, `Timestamp` (ADR-005) | `Reason::NotYetValid`, `Reason::Expired` |

`Ok(AcceptedSignature { object, signer: DidWebRoot, kid: Kid, issued_at: Timestamp, expires_at: Option<Timestamp> })`: the profile is always `did-web-v1`, and the object it authenticates travels with it, so its fields can never be combined with another manifest's (#entry-release-coverage).

Layering: each rule has one owner. Header rules are the validator's alone and are decided by `inspect_detached` (step 4) before any resolution or key work; `verify_detached` (step 8) re-runs the same `check_header` path over the same bytes with the same policy, so it can only agree. `kid` is checked once, in `header::inspect`. Signature bytes and JWK content are the validator's only. Time is checked after the cryptographic check because only then are `issuedAt` and `expiresAt` authenticated ("comparing authenticated `issuedAt` instants"), and the timestamps' syntax was already decided in step 1.

Agreement: the negative-header fixtures S14 pins for `header::inspect` are each rejected by `verify_signature` as `Reason::Header` with the same variant. A catalog signature whose header is `{"typ":"ai-catalog+jws",…,"typ":"ai-catalog-trust-manifest+jws"}`, and the reverse on a manifest, are each `Reason::Header(Jws(InvalidHeader))` (duplicate member), so no signature is replayed across object kinds.

### 3. Coverage, authority and endorsement

- `entry_release_coverage(accepted: &AcceptedSignature, entry: &CatalogEntry, integrity: &Result<(), IntegrityError>) -> Coverage`, in AMB-20 order: the accepted object is a manifest (`Uncovered(NotAManifest)` otherwise); `Subject::binds(entry)` (ADR-006: identifier, type, digest, version both absent or equal) else `Uncovered(SubjectMismatch)`; `integrity` is `Ok` else `Uncovered(Integrity(e))`. `Coverage::{Covered, Uncovered(CoverageGap)}`. The first bullet of #entry-release-coverage ("`signer` exactly equals the manifest's `contributor`") is already guaranteed for every accepted manifest signature: `signer-contributor-mismatch` is an ADR-006 error that `validate_manifest` reports through `signer_matches_contributor`, so step 1 rejects it as `Reason::Invalid`. Coverage does not repeat the check (one rule, one place).
- `integrity` is `verify_artifact(entry, node.base, &policy.fetch)` (ADR-013), computed at most once per entry and only when some manifest of that entry has an accepted signature that passes the subject check (AMB-335). An entry nothing relies on is never fetched.
- `publisher_authorized(identifier: &EntryIdentifier, signer: &DidWebRoot) -> bool`: `UrnAir::parse(identifier)` succeeds, `DidWebRoot::parse("did:web:" + publisher)` succeeds (the root-`did:web` domain rule, AMB-77, nothing case-folded, AMB-27), and that DID equals `signer` exactly. No subdomain, suffix or ownership heuristic exists.
- `catalog_endorsed(accepted: &AcceptedSignature, policy: &TrustPolicy) -> bool`: the accepted object is a catalog and its signer is in `policy.operators`. `host.identifier` is never consulted (AMB-34).
- `ManifestReport::endorsement() -> Endorsement::{None, Contributor, Publisher}`: `Contributor` = accepted and `Covered`; `Publisher` = `Contributor` and `publisher_authorized` (#the-did-web-publisher-profile: publisher authorization plus release binding). An unsigned manifest is `None` whatever its `contributor` says (#contributor-identity).

### 4. Selection among updates

`select_current(entry: &EntryReport<'a>) -> Vec<(&'a str, Selection<'a>)>`: per `contributor` (exact string), among that entry's manifests whose endorsement is at least `Contributor` (so all of one release), the one with the latest authenticated `issued_at` instant is `Selection::Current(&ManifestReport)`; two or more sharing the latest instant are `Selection::Tie(Vec<&ManifestReport>)`, whatever their array order or content (AMB-336). The chosen manifest is the parsed one, unchanged; nothing is merged (#independent-contributions-and-updates "preserve the chosen manifest and its signature rather than merge").

### 5. The trust report

- `verify_resolved(tree: &'a ResolvedCatalog, docs: &DidDocuments, policy: &TrustPolicy, now: SystemTime) -> TrustReport<'a>`: for every catalog node of the tree, the root, then each `Child::Catalog` in `children` order, recursively (ADR-009): the node's catalog signature outcome and `catalog_endorsed`; for every entry, every manifest's outcome (in array order, which carries no meaning), its coverage when accepted, `publisher_authorized`, and the entry's integrity result when computed. A `Child::Skipped` child has no node and so no outcome. An inline nested catalog (`data`) is not a node (ADR-009 AMB-191, AMB-193), so its inner entries get no outcome and are never endorsed (AMB-337); the containing entry itself is verified like any other, its digest covering the `data` JCS bytes.
- `DidDocuments` maps each signer `DidWebRoot` to `Result<DidDocument, ResolveError>`. For a signature that passes `precheck`, a signer absent from the map is `Reason::Resolve(None)` and a failed one is `Reason::Resolve(Some(e))`.
- `verify(tree: &'a ResolvedCatalog, policy: &TrustPolicy, now: SystemTime) -> TrustReport<'a>` is: collect the distinct signers of the signatures that pass `precheck` (AMB-334), `did_web::resolve(root, &policy.fetch)` once each, then `verify_resolved`; nothing else. Blocking; rustdoc says to call it through `spawn_blocking` from async code (§2 rules). T4 drives `verify_resolved` with pre-built documents (ADR-014 seam); `verify` never runs against a real `did:web` host in CI (named gap).
- Types: `TrustReport<'a> { catalogs: Vec<CatalogReport<'a>> }`; `CatalogReport<'a> { catalog: &'a AiCatalog, base: Option<&'a str>, signature: SignatureOutcome<'a>, endorsed: bool, entries: Vec<EntryReport<'a>> }`; `EntryReport<'a> { entry: &'a CatalogEntry, integrity: Option<Result<(), IntegrityError>>, manifests: Vec<ManifestReport<'a>> }`; `ManifestReport<'a> { manifest: &'a TrustManifest, signature: SignatureOutcome<'a>, coverage: Option<Coverage>, publisher_authorized: bool }`; `SignatureOutcome<'a>::{Unsigned, Rejected(Reason), Accepted(AcceptedSignature<'a>)}`. All `#[non_exhaustive]`. `Reason::{Unsigned, Invalid(Code), Unsupported(String), Disallowed, SignerShape, Header(HeaderError), Payload(PayloadError), Resolve(Option<ResolveError>), Key(KeySelectionError), Signature(DetachedJwsError), NotYetValid, Expired}`.
- Independence (RD-07): every outcome is computed from its own signature, its own document lookup and its own entry; no outcome reads another manifest's or another entry's outcome, and the report has no catalog-wide verdict.
- M13: no `Reason` is ever an `Err` of `verify` / `verify_resolved` (they return a report), and neither reads or changes `validate`'s conformance, so a signature defect never changes a document's level, lambda cold start or CLI exit code.
- What is authenticated (AMB-35, AMB-50, #publisher-and-policy-metadata, #signature-acceptance-and-time "limit authentication claims"): an accepted manifest signature authenticates that manifest's signed contents (everything in its payload) and, when covered, its binding to that entry release. Entry `publisher`, entry `extensions` (policy URLs included) and entry unknown members are outside every manifest payload; they are covered only by an accepted and endorsed catalog signature, which `CatalogReport::endorsed` reports and which is never reported as a publisher endorsement. Policy-URL documents are never fetched or authenticated. The report exposes no accessor that presents entry metadata as manifest-authenticated.
- Unverified content is retained: the report borrows the tree and never alters or drops anything; a caller may still display a rejected manifest (#verification-result MAY). No retry is built (MAY; the `Reason::Resolve` value lets a caller retry).

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Acceptance of one signature | `verify_signature` (pre-key steps: crate-private `precheck`) | itself; `verify_resolved` |
| Local acceptance policy | `TrustPolicy` (`Default` per §1) | all of the below |
| Kind of signed object | `SignedObject` | `verify_signature`; ADR-012 `header::policy`; ADR-013 payload |
| Accepted signature | `AcceptedSignature`, built only by `verify_signature` | `entry_release_coverage`, `catalog_endorsed`, `select_current` |
| Entry Release Coverage | `entry_release_coverage` | `verify_resolved` |
| Publisher authorization | `publisher_authorized` | `verify_resolved`; `ManifestReport::endorsement` |
| Catalog endorsement | `catalog_endorsed` | `verify_resolved` |
| Current manifest per contributor | `select_current` | itself |
| Per-signature, per-entry report | `verify_resolved(&ResolvedCatalog, &DidDocuments, &TrustPolicy, SystemTime) -> TrustReport` | `verify_resolved`; `verify` |
| Resolution plus report | `verify(&ResolvedCatalog, &TrustPolicy, SystemTime)` = `did_web::resolve` per distinct prechecked signer + `verify_resolved` | `verify` |
| Rejection reasons | `Reason` | — |

No derived artifact or golden belongs to this ADR; the report is a Rust value and is not serialised (the client's JSON output carries no trust verdict, ADR-009).

## Requirement → test

Test paths are under `crates/turul-ai-catalog-trust/tests/`. All outcomes are driven through `verify_signature` and `verify_resolved` with pre-built `DidDocument`s holding the RFC 7515 A.3 public key (ADR-013 fixtures) and signatures made by ADR-013's signer. Ids number requirement statements per section as in PLAN §4.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| verification-procedures-3 | "MUST NOT authenticate an identity using an unsupported profile" | `profile_selection::absolute_uri_profile_is_unsupported_and_never_authenticates` |
| signature-object-5 | "MUST NOT infer authentication or authority from [`signer`] alone" | `signature_acceptance::matching_signer_with_failing_jws_is_not_authenticated` |
| profile-selection-1 | "MUST select the verification procedure named by `profile`, and MUST require that it is both supported and permitted by local policy" | `profile_selection::did_web_v1_is_selected_by_its_exact_name`, `profile_selection::policy_without_did_web_v1_is_disallowed` |
| profile-selection-2 | "MUST NOT infer another profile … or fall back" | `profile_selection::case_variant_profile_is_unsupported_without_fallback` (`DID-WEB-V1` on an otherwise valid `did:web` ES256 signature) |
| profile-selection-3 | "Such a signature MUST NOT count as a verified endorsement; other manifests are evaluated independently" | `profile_selection::unsupported_or_disallowed_signature_never_counts` (a second, valid manifest on the entry stays endorsed) |
| profile-selection-7 | "MAY impose stricter local acceptance requirements without defining another profile" | `profile_selection::stricter_local_policy_keeps_the_profile_name` (zero skew, empty operators; the accepted signature is still `did-web-v1`) |
| profile-selection-8 | "The selected profile MUST authenticate the exact `signer` value" | `profile_selection::did_web_v1_authenticates_the_exact_signer` (a document for `did:web:other.example` handed for `did:web:example.com` is `Resolve(Some(IdMismatch))`) |
| profile-selection-9 | "MUST also apply the coverage and authority checks required for the intended endorsement" | `trust_report::endorsement_applies_coverage_and_authority` |
| signature-acceptance-and-time-1 | "MUST validate the Signature object's structure, profile selection, payload, JWS, and selected signer profile before accepting" | `signature_acceptance::error_diagnostic_in_signed_object_rejects` (an uppercase-hex attestation digest gives `Invalid(digest-invalid)`), `signature_acceptance::checks_run_in_the_documented_order` (one fixture per step, each failing only that step) |
| signature-acceptance-and-time-3 | "MUST NOT accept an endorsement before `issuedAt` or at or after `expiresAt`" | `signature_acceptance::accepted_at_issued_at`, `signature_acceptance::rejected_one_second_before_issued_at`, `signature_acceptance::rejected_at_expires_at`, `signature_acceptance::offsets_compare_as_instants` (`+10:00` and `Z` forms of one instant) |
| signature-acceptance-and-time-4 | "MAY allow a small clock-skew tolerance" | `signature_acceptance::clock_skew_widens_both_bounds` (2 s accepts one second before `issuedAt` and one second after `expiresAt`) |
| signature-acceptance-and-time-5 | "An invalid, unsupported, or expired signature MUST NOT count as a verified endorsement" | `signature_acceptance::invalid_unsupported_or_expired_never_counts` |
| signature-acceptance-and-time-6 | "Other manifests are evaluated independently; consumers MAY apply local requirements" | `signature_acceptance::other_manifests_are_evaluated_independently`; no contributor filter is built (MAY): every outcome names its manifest and signer, so callers filter |
| signature-acceptance-and-time-7 | "MUST limit authentication claims … to its signed contents and successfully verified subject bindings" | `trust_report::manifest_authentication_covers_only_signed_contents` |
| verification-result-1 | "Failure MUST NOT be treated as a verified endorsement" | `trust_report::header_and_shape_defects_are_outcomes_not_errors` (each defect gives a non-endorsement while `verify_resolved` returns a report and `validate`'s conformance is unchanged) |
| verification-result-2 | "MAY retain or display unverified content, retry …, or reject" | `trust_report::unverified_content_is_retained` (the report borrows the unchanged tree; the rejected manifest is reachable) |
| entry-release-coverage-7 | "MUST also verify the artifact bytes against the matching digest" | `entry_coverage::digest_mismatch_leaves_a_verified_manifest_uncovered` |
| jws-algorithm-and-key-requirements-2 | "MUST support ES256 and MUST reject any other `alg` value when applying that profile" | `signature_acceptance::es256_is_verified`, `signature_acceptance::other_alg_is_rejected_before_any_lookup` (`ES384` → `Header(Jws(AlgNotAllowed))` with an empty `DidDocuments`) |
| jws-algorithm-and-key-requirements-6 | "A mismatch MUST fail verification under this profile" | `signature_acceptance::kid_mismatch_fails_before_any_lookup` (`Header(KidMismatch)`) |
| the-did-web-signer-profile-3 | "MAY support additional profiles by agreement" | not built (MAY); `profile_selection::only_did_web_v1_is_supported` pins that every other name is `Unsupported` |
| the-did-web-signer-profile-4 | "MUST NOT silently apply these rules to a different mechanism" | `profile_selection::did_web_rules_apply_only_under_did_web_v1` (an absolute-URI profile with a broken header is `Unsupported`, never `Header` or `SignerShape`) |
| the-did-web-publisher-profile-1 | "The entry MUST use the standard `urn:air` syntax" | `publisher_authorization::non_urn_air_identifier_is_never_authorized`, `publisher_authorization::urn_air_with_two_components_is_never_authorized` |
| the-did-web-publisher-profile-2 | publisher component "MUST satisfy the root `did:web` domain restrictions" | `publisher_authorization::publisher_failing_the_domain_rule_is_never_authorized` (`urn:air:Example.com:…`, `urn:air:2130706433:…`, `urn:air:127.1:…`, `urn:air:0x7f.0.0.1:…`) |
| the-did-web-publisher-profile-3 | "The authenticated signer identity MUST exactly match `did:web:` followed by that component" | `publisher_authorization::publisher_signer_is_authorized` (`did:web:example.com` for `urn:air:example.com:agent:billing`), `publisher_authorization::assessor_is_endorsed_but_not_publisher_authorized` (`did:web:assessor.example`) |
| the-did-web-publisher-profile-4 | "Subdomain, suffix, and organizational-ownership heuristics MUST NOT be used" | `publisher_authorization::subdomain_signer_is_not_authorized` (`did:web:sub.example.com`), `publisher_authorization::suffix_signer_is_not_authorized` (`did:web:example.com.evil`) |
| the-did-web-publisher-profile-5 | "The signature MUST satisfy Entry Release Coverage" | `publisher_authorization::publisher_authorization_needs_release_coverage` (an entry `version` added after signing leaves the publisher signature uncovered and not publisher-endorsed) |
| catalog-authorization-1 | "A separate profile or configured policy MUST identify authorized operators before a consumer accepts a signature as a catalog endorsement" | `catalog_endorsement::verified_signature_with_empty_operators_is_not_endorsed`, `catalog_endorsement::operator_signature_is_endorsed`, `catalog_endorsement::host_identifier_is_never_consulted` |
| publisher-and-policy-metadata-1 | "MUST distinguish publisher-authenticated metadata from metadata supplied or endorsed by another entity" | `trust_report::entry_metadata_is_outside_manifest_authentication` (changing entry `publisher`, `extensions` or an unknown member after signing leaves the manifest outcome unchanged and breaks the catalog signature) |
| independent-contributions-and-updates-3 | "SHOULD prefer the manifest with the most recent acceptable signature, comparing authenticated `issuedAt` instants" | `manifest_selection::latest_authenticated_issued_at_is_current`, `manifest_selection::rejected_later_manifest_is_never_current` |
| independent-contributions-and-updates-4 | "MUST NOT infer an ordering from array order" | `manifest_selection::equal_issued_at_is_a_tie_in_either_array_order` |
| independent-contributions-and-updates-5 | "SHOULD preserve the chosen manifest and its signature rather than merge" | `manifest_selection::current_manifest_is_returned_unchanged` |
| contributor-identity-4 | "MUST NOT infer authentication from its spelling or mere presence" | `trust_report::unsigned_did_web_contributor_is_not_endorsed` (`contributor` `did:web:example.com`, no signature → `Unsigned`, `Endorsement::None`) |
| level-3-trusted-catalog-1 | "MUST authenticate signers, verify signatures and endorsement times, enforce Entry Release Coverage, and verify artifact content … before relying" | `trust_report::reliance_needs_signer_time_coverage_and_integrity` (each one missing gives `Endorsement::None`) |
| level-3-trusted-catalog-3 | "MUST verify each contributor's signature and the subject binding to the same entry release" | `trust_report::each_contributor_is_verified_on_its_own` |
| level-3-trusted-catalog-4 | "MUST distinguish the initial interoperable `did:web` profiles from other profiles supported by private agreement" | `trust_report::did_web_outcomes_are_distinguished_from_unsupported_profiles` (`Accepted` is always `did-web-v1`; others are `Rejected(Unsupported(name))`) |
| trust-layers-1 | "SHOULD implement at least Layer 2 (signature verification)" | `trust_report::reliance_needs_signer_time_coverage_and_integrity` (Layer 2 = `entry_release_coverage` over an accepted signature, AMB-60) |
| stale-attestations-1 | "Check acceptable contributor endorsement times; unsigned entry `updatedAt` is only an advisory listing timestamp" | `signature_acceptance::updated_at_is_never_read` (an entry `updatedAt` before `issuedAt` or after `expiresAt` changes no outcome) |

Further T4 behaviour, pinned without a requirement id of its own:

| Rule | Test function |
|---|---|
| duplicate `typ` cannot replay a signature across kinds (§2 agreement) | `signature_acceptance::duplicate_typ_cannot_replay_across_kinds` |
| every S14 negative header gives the variant S14 pins | `signature_acceptance::every_header_fixture_is_rejected_with_its_inspect_variant` |
| signer-shape defect, e.g. `did:web:Example.com`, is `SignerShape` with no lookup | `signature_acceptance::signer_shape_is_rejected_before_any_lookup` |
| standalone manifest verifies through `validate_manifest` and gets no coverage | `signature_acceptance::standalone_manifest_verifies_without_coverage` |
| spec-example placeholder signatures pass the header and never verify (AMB-66) | `signature_acceptance::spec_example_signatures_never_verify` (`Signature(_)`) |
| signer missing from `DidDocuments` | `trust_report::signer_without_document_is_resolve_none` |
| uncovered cases | `entry_coverage::signer_not_contributor_is_rejected_and_uncovered` (`Rejected(Invalid(signer-contributor-mismatch))`, no coverage), `entry_coverage::version_added_after_signing_is_uncovered`, `entry_coverage::catalog_signature_is_never_entry_coverage` |
| nested catalogs | `trust_report::nested_catalog_entries_get_outcomes` (a `url` child), `trust_report::inline_nested_catalog_entries_get_no_outcome` (the containing entry is verified; its inner manifests are never endorsed) |
| one failure never affects another manifest or entry | `trust_report::failing_manifest_leaves_other_endorsement_intact`, `trust_report::failing_entry_leaves_other_entries_intact` |
| `verify` composition | `trust_report::verify_is_resolve_then_verify_resolved` (source scan) |

Decisions owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| RD-07 | one outcome per manifest per entry; no catalog-wide verdict; no Level 3 | `trust_report::failing_manifest_leaves_other_endorsement_intact`, `trust_report::failing_entry_leaves_other_entries_intact` |
| RD-16 (time half) | the time window is acceptance here, never a document diagnostic | `signature_acceptance::rejected_at_expires_at` |
| RD-19..RD-27 (verify) | fetches through `client::fetch`; every catalog of the tree; `did-web-v1` only; `urn:air` arity (AMB-78); clock skew (AMB-24) | `trust_report::nested_catalog_entries_get_outcomes`, `profile_selection::only_did_web_v1_is_supported`, `publisher_authorization::urn_air_with_two_components_is_never_authorized`, `signature_acceptance::clock_skew_widens_both_bounds` |
| RD-81 (publisher half) | IP spellings never authorise a publisher | `publisher_authorization::publisher_failing_the_domain_rule_is_never_authorized` |
| RD-84 | an error diagnostic in the signed object rejects the signature | `signature_acceptance::error_diagnostic_in_signed_object_rejects` |
| AMB-05 | "publisher-authorized" = `publisher_authorized` over a covered signature; delegation OOS | `publisher_authorization::publisher_signer_is_authorized` |
| AMB-20 | coverage order: acceptance (signer = contributor inside it), binding, integrity, then authority | `entry_coverage::integrity_is_checked_only_after_the_subject` |
| AMB-22, AMB-54, AMB-55 | verification MUSTs bind this crate only; no level is reported | `trust_report::report_has_no_conformance_level` (type-level: no level member) |
| AMB-24 | `clock_skew` default 0 | `signature_acceptance::rejected_one_second_before_issued_at`, `signature_acceptance::clock_skew_widens_both_bounds` |
| AMB-27 | the Publisher Profile has no `profile` value; nothing is case-folded | `publisher_authorization::publisher_failing_the_domain_rule_is_never_authorized` |
| AMB-34 | operators are `TrustPolicy::operators`, empty by default | `catalog_endorsement::verified_signature_with_empty_operators_is_not_endorsed` |
| AMB-35, AMB-50 | what a manifest or catalog signature authenticates (§5) | `trust_report::manifest_authentication_covers_only_signed_contents`, `trust_report::entry_metadata_is_outside_manifest_authentication` |
| AMB-60 | Layer 1 = `verify_artifact`, Layer 2 = coverage | `trust_report::reliance_needs_signer_time_coverage_and_integrity` |
| AMB-331 | step order of §2; the first failure is the reason | `signature_acceptance::checks_run_in_the_documented_order` |
| AMB-332 | "structure" = no error diagnostic in the signed object | `signature_acceptance::error_diagnostic_in_signed_object_rejects`, `signature_acceptance::standalone_manifest_verifies_without_coverage` |
| AMB-333 | time after cryptography; unrepresentable shifted bounds fall back to `now` | `signature_acceptance::clock_skew_widens_both_bounds`, `signature_acceptance::unrepresentable_skew_is_never_looser` (`Duration::MAX`) |
| AMB-334 | `now: SystemTime`; `verify` resolves only prechecked signers | `trust_report::verify_is_resolve_then_verify_resolved` |
| AMB-335 | integrity at most once per entry, only when relied on | `entry_coverage::integrity_is_fetched_once_and_only_when_relied_on` (loopback request log: one request for two covered manifests, none for an entry with only rejected ones) |
| AMB-336 | equal latest instants → `Tie`, whatever the content | `manifest_selection::equal_issued_at_is_a_tie_in_either_array_order` |
| AMB-337 | the tree is ADR-009's; inline nested catalogs are not nodes, so their inner entries get no outcome | `trust_report::nested_catalog_entries_get_outcomes`, `trust_report::inline_nested_catalog_entries_get_no_outcome` |

New ids (not in PLAN §6), from the blocks RD-81.. / AMB-331..:

- RD-84: `ai_catalog_validate::verify` validates the document but does not consult the result when checking a signature, so a signed manifest with, for example, an uppercase-hex attestation digest still verifies. #signature-acceptance-and-time requires validating the structure before accepting; here such a signature is `Reason::Invalid`.
- AMB-331: the spec lists what must be validated, not an order. The order of §2 puts the checks that need no network first, so a defective signature never causes a fetch (#safe-fetching: these inputs "may be attacker-controlled before authentication"), and each signature gets one deterministic reason.
- AMB-332: "the Signature object's structure" is read as the signed object's validity: the payload is the whole object, so an error diagnostic anywhere in it means the signed content is not a valid manifest or catalog. A warning (including `nested-depth-limit`) does not reject.
- AMB-333: §2 step 9 and its reason.
- AMB-334: PLAN passes `now: Timestamp`. Applying a skew needs instant arithmetic, which `Timestamp` (ADR-005) does not offer; `SystemTime` has checked arithmetic and `Timestamp::from_system_time` is already planned, so the clock is a `SystemTime` and no new `Timestamp` API is needed. `verify` resolves only signers whose signature passes `precheck`, which PLAN's ADR-015 row already requires ("decided by `inspect_detached` before any resolution").
- AMB-335: #verifying-artifact-integrity puts retrieval after authentication and coverage (step 1), and every fetch is an SSRF surface. Fetching once per entry, only when an accepted manifest would rely on it, follows that order and leaves outcomes unchanged.
- AMB-336: "Equal issuance times with different content require local selection policy". Equal content would make either choice equivalent, but deciding equality needs a payload comparison that gives the caller nothing a `Tie` does not; `Tie` is returned in both cases and array order is never used.
- AMB-337: "every entry of every catalog in the tree" depends on what ADR-009's `ResolvedCatalog` holds, and ADR-009 makes only `url` children nodes (AMB-191, AMB-193). The trust crate does not walk `data` itself (one traversal, ADR-009). The spec makes reliance conditional on verification (#level-3-trusted-catalog "before relying on signed claims"), so an entry inside an inline nested catalog that gets no outcome is simply never relied on: nothing is endorsed wrongly. The containing entry's own manifests are verified, with the `data` JCS bytes as the artifact. Named gap: inner manifests of inline nested catalogs are never verified. If ADR-009 later makes inline catalogs nodes, they are covered with no change here.

## Divergences from ai-catalog-rust 0.4.0

| Id | ai-catalog-rust | This crate | Spec | Pinned by |
|---|---|---|---|---|
| RD-07 | `ai_catalog_validate::verify` reports `ConformanceLevel::Trusted` when some entry is publisher-endorsed and none fails; one failing publisher manifest removes it for the whole catalog | one outcome per manifest per entry; no level (D2) | #independent-contributions-and-updates; #level-3-trusted-catalog "Unverified additional contributions do not by themselves invalidate" | `trust_report::failing_manifest_leaves_other_endorsement_intact` |
| RD-19..RD-27 | caller `Fetch`; root entries only; `did-web-v1` only; `urn:air` publisher taken before the first `:` with no arity check; no clock skew | `client::fetch`; every catalog node; `did-web-v1` only (agree); `UrnAir` arity (AMB-78); skew per AMB-24 | #safe-fetching, #nested-catalog-entries, #profile-selection, #the-did-web-publisher-profile, #signature-acceptance-and-time | `trust_report::nested_catalog_entries_get_outcomes`, `publisher_authorization::urn_air_with_two_components_is_never_authorized` |
| RD-81 | IP spellings such as `2130706433` accepted as publisher domains | never authorised | #the-did-web-publisher-profile | `publisher_authorization::publisher_failing_the_domain_rule_is_never_authorized` |
| RD-84 | signature verified regardless of validation errors | `Reason::Invalid(Code)` | #signature-acceptance-and-time | `signature_acceptance::error_diagnostic_in_signed_object_rejects` |

Agreements: `issuedAt ≤ now < expiresAt` with no skew (ai-catalog-validate checks time before resolution; here after verification, which changes the reason reported, never acceptance); `Unverifiable` for an unsupported profile, as `Unsupported` here; catalog authorization by a configured operator list; coverage requires signer = contributor, subject = entry and an entry digest.

## Consequences

- The only trust output in the workspace is `TrustReport`: per signature, per manifest, per entry, per catalog node. Nothing in the format crate, the client, the lambda or the front ends reports trust (ADR-009, ADR-011), and Level 3 is never claimed (D2).
- A signature can fail for many reasons, and only the first in §2 order is reported. That keeps each outcome deterministic, at the cost of hiding later defects of an already-rejected signature.
- Every `verify` call fetches each distinct prechecked signer's DID document and each relied-on artifact once, with no cache (ADR-014).
- Plan drift recorded:
  - PLAN passes `Timestamp` as the clock; this ADR uses `SystemTime` (AMB-334). PLAN's T4 criteria ("`now` = `issuedAt`", "one second before") are unchanged in meaning.
  - PLAN's `verify` is "`did_web::resolve` once per distinct signer … nothing else"; it resolves only signers that pass `precheck` (AMB-334).
  - PLAN names `Reason::Resolve` for a missing or failed signer; it carries `Option<ResolveError>` (`None` = missing), and `Reason::{Unsigned, Payload, Key, Signature, NotYetValid, Expired}`, `SignatureOutcome`, `Coverage`, `CoverageGap`, `Endorsement` and `Selection` are named here.
  - PLAN T4 says the duplicate-`typ` replay is rejected with "`Reason::Header` (duplicate member)"; as released that is `Header(Jws(InvalidHeader))`.
  - PLAN's coverage formula is "verified signature ∧ `signer_matches_contributor` ∧ `Subject::binds` ∧ `verify_artifact`". ADR-006 makes `signer-contributor-mismatch` an error, so step 1 already rejects such a signature and coverage does not call `signer_matches_contributor` again; PLAN T4's "signer ≠ contributor leaves the manifest uncovered" still holds (the signature is rejected, so there is no coverage). ADR-006's owner table lists `entry_release_coverage` as a caller of `signer_matches_contributor`; that row is stale and is ADR-006's to correct.
  - PLAN T4's "Nested-catalog entries get outcomes too" holds for `url` children; inline nested catalogs are not ADR-009 nodes, so their inner entries get none (AMB-337, named gap).

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created ahead of T4 from PLAN §3 (ADR-015 row), §4, T4, D2, D8, M13, RD-07, RD-16, RD-19..RD-27, AMB-05, AMB-20, AMB-22, AMB-24, AMB-27, AMB-34, AMB-35, AMB-50, AMB-54, AMB-55, AMB-60 and validator 0.3.4 `verify_detached`. RD-84 and AMB-331 to AMB-337 added (ranges no other ADR used). Aligned with ADR-005 (`Timestamp::from_system_time`), ADR-006 (`signer-contributor-mismatch` is an error) and ADR-009 (only `url` children are nodes). |
| 2026-11-17 | T4 landed. Accepted. Realisation details: `TrustPolicy` and every report type are `#[non_exhaustive]`; `DidDocuments` is the type alias `HashMap<DidWebRoot, Result<DidDocument, ResolveError>>`; `Selection<'r, 'a>` carries two lifetimes (the borrow of the `EntryReport` and the tree); `ResolveError` is `Clone` so a failed document can be reported once per signature; the integrity result passed to `entry_release_coverage` is a placeholder `Ok(())` when no manifest of the entry bound its subject (never read in that case). |
