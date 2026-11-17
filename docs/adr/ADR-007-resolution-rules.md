# ADR-007: Display-name, description and version resolution; latest selection

- **Status:** Accepted (2026-11-17)
- **Date:** 2026-11-17
- **Implements:** `crates/turul-ai-catalog/tests/resolution.rs`, `crates/turul-ai-catalog/tests/latest_version.rs` (both land in S6).
- **Depends on:** ADR-002 (model), ADR-004 (`EntryIdentifier`, exact equality), ADR-005 (`Timestamp::parse` and its instant order). Binding decisions: M3 (one owning symbol for display-name, description and version resolution), M6 (no `semver` dependency).
- **Spec:** https://ai-catalog.io, `specification/ai-catalog.md` as read in ADR-001 (last commit 2026-11-10).

## Context

`displayName`, `description` and `version` are OPTIONAL, and the spec gives each a consumer resolution order whose step 2 is "the referenced artifact's own" value, "if the consumer has already fetched or cached the artifact". This crate never fetches and never reads artifact schemas (AGENTS.md: artifacts are opaque), so step 2 is a value the caller supplies, and the spec's "SHOULD NOT dereference an artifact at render time" holds by construction (AMB-13; the client half is ADR-009). #multi-version-entries asks clients that need the latest version to sort by SemVer "when parseable" or by `updatedAt`.

Spec anchors implemented: #resolving-an-artifact-s-display-name, #resolving-an-artifact-s-description, #resolving-an-artifact-s-version, #multi-version-entries (latest selection), #catalog-entry (`displayName`, `description`, `version` precedence).

## Decision

### 1. Functions (`resolve` module)

All four are pure, borrow from their inputs and never allocate a new value.

- `display_name<'a>(entry: &'a CatalogEntry, artifact_name: Option<&'a str>) -> &'a str`, in order: the entry's `displayName` when present; else `artifact_name` when `Some`; else the trailing segment of `entry.identifier`: the text after its last `:` or `/`, or the whole identifier when it has neither or that text is empty (AMB-72). `urn:air:example.com:mcp:weather` gives `weather`; `https://example.com/agents/x/` gives the whole identifier.
- `description<'a>(entry: &'a CatalogEntry, artifact_description: Option<&'a str>) -> Option<&'a str>`: the entry's `description`, else `artifact_description`, else `None`. There is no identifier fallback.
- `version<'a>(entry: &'a CatalogEntry, artifact_version: Option<&'a str>) -> Option<&'a str>`: the entry's `version`, else `artifact_version`, else `None`. An entry `version` that differs from the artifact's is still returned: it "is authoritative for catalog-level sorting and selection", and the mismatch is a publishing error that is never resolved "by silently preferring the artifact's value". Surfacing the mismatch (a MAY) is not built.
- `latest<'a>(catalog: &'a AiCatalog, identifier: &EntryIdentifier) -> Option<&'a CatalogEntry>` (§2).

"Present" means the member exists, whatever its text: `displayName: ""` is returned as `""`, and `Some("")` from the caller wins over step 3 (AMB-150). This is the presence rule that ADR-004 and ADR-005 apply to `version` (AMB-70). A caller that has no artifact value, or an artifact whose own name is absent ("step 2 yields no name"), passes `None`.

The returned text is raw. Display safety (`DisplayText`, AMB-75) is the client's, applied to whatever this returns (ADR-009).

### 2. Latest selection (AMB-12, AMB-151, AMB-152)

The group is `catalog.entries` whose `identifier` equals `identifier` exactly, in one catalog only; a nested catalog is a separate document (ADR-005 AMB-46).

1. Empty group: `None`. One entry: that entry, since nothing needs ordering.
2. If every entry in the group has a `version` that parses as SemVer 2.0.0, the entry of highest precedence. Two entries of equal precedence (for example `1.0.0+a` and `1.0.0+b`, since build metadata is ignored, SemVer §10) give `None`.
3. Otherwise, if every entry has an `updatedAt` that `Timestamp::parse` accepts, the entry with the latest instant. Equal instants give `None`.
4. Otherwise `None`.

The result never depends on array order: a tie is `None`, not the first or last match (#independent-contributions-and-updates gives the same rule for manifests; RD-18). The crate-private `semver` module is the one SemVer 2.0.0 implementation: the full §2 grammar (no `v` prefix, no leading zero in a numeric identifier, non-empty dot-separated pre-release and build identifiers of `[0-9A-Za-z-]`), and §11 precedence. Numeric identifiers are compared by digit-string length, then lexically, so any length compares without overflow; a pre-release version has lower precedence than its release; numeric identifiers sort below alphanumeric ones; a longer pre-release set wins when all preceding identifiers are equal. It exposes `parse` and a precedence comparison and nothing else (precedence only, PLAN §2).

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Display name | `resolve::display_name` | itself; client rows (ADR-009), TUI (ADR-011) |
| Description | `resolve::description` | itself; client, TUI |
| Version | `resolve::version` | itself; client, TUI |
| Latest entry of an identifier | `resolve::latest` | itself; client `Filter` (ADR-009) |
| SemVer grammar and precedence | crate-private `semver` | `resolve::latest` |
| Instant order of `updatedAt` | `timestamp::Timestamp` (ADR-005) | `resolve::latest` |

No golden bytes or schema digest belong here (D17).

## Requirement → test

Test paths are under `crates/turul-ai-catalog/tests/`.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| resolving-an-artifact-s-display-name-1 | "SHOULD resolve one in the following order": entry `displayName`, artifact name, identifier segment | `resolution::display_name_resolves_in_spec_order` (`urn:air:example.com:mcp:weather` → `weather`, `urn:air:anonymous.modelcontextprotocol.io:mcp:brave-search` → `brave-search`) |
| resolving-an-artifact-s-description-1 | "It SHOULD resolve one in the following order" | `resolution::description_resolves_in_spec_order` |
| resolving-an-artifact-s-description-2 | "no identifier-derived fallback; a consumer SHOULD simply render the entry without one" | `resolution::missing_description_stays_none` |
| resolving-an-artifact-s-version-1 | "To obtain a version, a consumer SHOULD resolve one in the following order" | `resolution::version_resolves_in_spec_order` |
| resolving-an-artifact-s-version-3 | unversioned entries: "SHOULD fall back to `updatedAt`" | `latest_version::non_semver_group_selects_by_updated_at` |
| resolving-an-artifact-s-version-5 | mismatch "SHOULD NOT [be resolved] by silently preferring the artifact's value" | `resolution::entry_version_wins_over_a_differing_artifact_version` |
| multi-version-entries-5 | latest: "sort … by `version` (when parseable as a semantic version) or by `updatedAt`, and select the most recent" | `latest_version::highest_semver_precedence_is_latest` (`2.0.0` vs `2.1.0` → `2.1.0`; `1.0.0-rc.1` < `1.0.0`; `1.10.0` > `1.9.0`), `latest_version::non_semver_group_selects_by_updated_at` |
| catalog-entry-12 | `displayName` "takes precedence … SHOULD render it as given even when it differs" | `resolution::entry_display_name_wins_over_a_differing_artifact_name` |
| catalog-entry-15 | entry `description` "takes precedence … SHOULD render it as given even when it differs" | `resolution::entry_description_wins_over_a_differing_artifact_description` |

Decisions owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| RD-18 (resolution half; filter half ADR-009) | resolution helpers exist; latest never uses array order | `latest_version::latest_never_depends_on_array_order` (each group also reversed), `resolution::display_name_resolves_in_spec_order` |
| AMB-12 | semver path, then `updatedAt`, then `None`; ties `None` | `latest_version::highest_semver_precedence_is_latest`, `latest_version::non_semver_group_selects_by_updated_at`, `latest_version::equal_precedence_or_instant_gives_none`, `latest_version::group_without_an_order_gives_none` |
| AMB-13 (resolution half; client half ADR-009) | step 2 is caller-supplied; nothing is fetched | `resolution::artifact_values_come_only_from_the_caller` |
| AMB-72 | identifier segment after the last `:` or `/`; whole identifier when none or empty | `resolution::identifier_segment_fallback` (`a:b/c` → `c`, `plain` → `plain`, `x:` → `x:`, `""` → `""`) |
| AMB-150 | presence, not content, decides each step | `resolution::empty_present_values_win` |
| AMB-151 | one-entry group returns that entry; empty group `None` | `latest_version::single_entry_group_is_its_own_latest` |
| AMB-152 | an invalid `updatedAt` or a missing one leaves the group unordered | `latest_version::group_without_an_order_gives_none` |
| SemVer grammar | SemVer 2.0.0 §2, §9–§11 | `latest_version::non_semver_versions_fall_back` (`v1.0.0`, `01.0.0`, `1.0`, `1.0.0-`, `1.0.0+` are not SemVer), `latest_version::semver_numeric_identifiers_of_any_length_compare` |

New ids (not in PLAN §6): AMB-150 to AMB-152, from a block reserved for this ADR.

- AMB-150: each step says "if present" (#resolving-an-artifact-s-display-name: "`displayName` on the entry, if present"); the spec defines no emptiness exception, and #catalog-entry calls a present `displayName` "the authoritative value for display". A blank result is a publishing choice, not a resolution failure.
- AMB-151: #multi-version-entries says "sort entries sharing the same `identifier` … and select the most recent"; a group of one has a most recent entry without any order.
- AMB-152: "when parseable" and AMB-12's "every entry has `updatedAt`" need a comparable instant for every entry; an unparsable `updatedAt` is already a `timestamp-invalid` error (ADR-005) and gives no instant.

## Divergences from ai-catalog-rust 0.4.0

| Id | ai-catalog-rust | This crate | Spec | Pinned by |
|---|---|---|---|---|
| RD-18 | `AiCatalog::get_by_id` returns the first entry with the identifier; no display-name, description, version or latest helpers | `resolve::*`; `latest` decides by SemVer or `updatedAt` and never by array order | #resolving-an-artifact-s-display-name, #resolving-an-artifact-s-description, #resolving-an-artifact-s-version, #multi-version-entries consumer SHOULDs | `latest_version::latest_never_depends_on_array_order`, `resolution::display_name_resolves_in_spec_order` |

## Consequences

- The client, TUI and CLI show names, descriptions and versions only through these four functions (M3); ADR-009's `dependency_direction` and source scans keep it so.
- Step 2 values from an artifact can be supplied only by a caller that already holds the artifact, so no render path fetches one.
- A group mixing SemVer and non-SemVer versions, or lacking `updatedAt` on any entry, has no latest entry. Callers that need one must apply their own policy and say so.
- `semver` is crate-private precedence code, not a dependency (M6); it has no other caller.
- Plan drift recorded: PLAN §3 gives `display_name`, `description` and `version` the second parameter `Option<&str>`; the lifetimes above tie the result to the entry and the caller's value, so no `String` is returned.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created for S6: `resolve::{display_name, description, version, latest}`, crate-private `semver`. AMB-150 to AMB-152 added. |
| 2026-11-17 | Accepted at S6 close (0.0.7): `resolution.rs` and `latest_version.rs` are present and green (`cargo test -p turul-ai-catalog`, 230 passed), and `resolve` with crate-private `semver` is the only resolution and latest-selection code in the workspace (no client, TUI or CLI crate exists yet; each will call `resolve`, M3). |
