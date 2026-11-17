# ADR-009: Resolved catalog tree: traversal, cycles, filtering, display text and JSON output

- **Status:** Accepted (2026-11-17, S9 close, 0.0.11)
- **Date:** 2026-11-17
- **Implements:** `crates/turul-ai-catalog-client/tests/`: `nested_traversal.rs`, `filter.rs`, `display_text.rs`, `json_output.rs`, `schema_digest.rs` (including the ADR-003 owner `crates/turul-ai-catalog/tests/support/schema_digest.rs` by `#[path]`), and the goldens in `tests/fixtures/json_output/` with `SOURCE.md`. Loopback tests use ADR-008's `tests/support/loopback.rs`.
- **Depends on:** ADR-008 (`load` fetch half, `fetch`, `accept_response`, `resolve_reference`, `LoadError`, `LoadWarning`, `FetchPolicy`), ADR-005 (`validate_at`, `NestingDepth`, `MAX_NESTING_DEPTH`, `Report`, `Diagnostic`, `Conformance`, diagnostic codes), ADR-007 (`resolve::display_name`, `resolve::description`, `resolve::version`), ADR-004 (`ArtifactType::AI_CATALOG` and its exact comparison, AMB-98; `EntryIdentifier`), ADR-003 (`Json::to_canonical_vec`; the schema-digest algorithm and its owner), ADR-002 (`parse`). Binding decisions: M3 (one owner each for traversal, resolution and filtering; front ends render only the client's tree), M16 (`FetchPolicy::max_requests`, `Skipped(Budget)`, one budget `LoadWarning`; owner: this ADR), D4 (JCS is the only serialisation), D17 (goldens from an independent producer; schema digest), D15 (dev-dependency `sha2`).
- **Spec:** https://ai-catalog.io, `specification/ai-catalog.md` as read in ADR-001 (last commit 2026-11-10).

## Context

`load` (ADR-008) returns a root catalog. This ADR makes it a tree: `url`-nested catalogs are fetched under the depth limit and cycle tracking (#nested-catalog-entries, #nested-catalog-depth-and-circular-references), the number of requests per `load` is capped by M16's request budget, each entry gets resolved display values without any artifact fetch (#resolving-an-artifact-s-display-name and its siblings), and the tree is filtered, rendered as text through one sanitiser (#embedded-content-safety) and serialised as one JSON output. The CLI and TUI (ADR-011) render only this tree (M3). The JSON output is a derived artifact under D17.

Spec anchors implemented: #nested-catalog-entries (the `url` half; the inline half is ADR-005), #nested-catalog-depth-and-circular-references, #resolving-an-artifact-s-display-name, #resolving-an-artifact-s-description, #resolving-an-artifact-s-version (render-time rule and caller side; the order is ADR-007), #multi-version-entries (matching a specific version), #embedded-content-safety (displayed text), #safe-fetching (what is never fetched), #policy-urls-extension (never fetched), #trust-layers (output without a trust verdict), #catalog-entry (`tags` "for filtering and discovery").

## Decision

### 1. The tree

`ResolvedCatalog` (`#[non_exhaustive]`, public fields):

| Field | Meaning |
|---|---|
| `document: AiCatalog` | the parsed document |
| `report: Report` | `validate_at(&document, depth)`; inline nested `data` is validated inside it (ADR-005) |
| `base: Option<String>` | `Fetched::url()`, the final URL after redirects; `None` for a file root (ADR-008) |
| `depth: NestingDepth` | `NestingDepth::ROOT` for the root; one more per `url` level |
| `warnings: Vec<LoadWarning>` | retrieval warnings for this document (`media-type-generic`, ADR-008); on the root only, after those, the one `request-budget` warning when any child is `Skipped(Budget)` (§2c) |
| `rows: Vec<EntryRow>` | one per entry, in document order (§3) |
| `children: Vec<Nested>` | one per followed entry (§2), in entry order |

`Nested { entry: usize, child: Child }`, where `entry` is the index in `document.entries`. `Child::Catalog(Box<ResolvedCatalog>)` or `Child::Skipped { url: String, reason: SkipReason }`. `SkipReason` (`#[non_exhaustive]`) is `Budget`, `Depth`, `Cycle`, `Unresolvable` or `Fetch(LoadError)`. `url` is `resolve_reference(node.base, url)` whenever that succeeds, whatever the reason (so a `Budget` or `Depth` skip, decided before resolution in §2, still carries the resolved URL), and the entry's `url` text as written when it does not.

### 2. Traversal (`load`, traversal half)

After the fetch half has built a node, `load` visits its entries depth-first in entry order, one request at a time (AMB-196). For each entry it:

1. Follows the entry only when `artifact_type == ArtifactType::AI_CATALOG`, compared exactly (ADR-004 AMB-98), and its source is `ArtifactSource::Url`. No other entry produces a child or a request. An inline nested catalog (`data`) is validated in the parent's `report` (ADR-005) and is not a child (AMB-191, AMB-193).
2. If the request budget is spent (§2c), the result is `Skipped(Budget)`, with no request.
3. Computes the child depth from `NestingDepth`. A child beyond `MAX_NESTING_DEPTH` (4; the root is 1, AMB-45) is `Skipped(Depth)`, with no request.
4. Resolves `url` with `resolve_reference(node.base, url)` (ADR-008, AMB-73). `Unresolvable`, for example a relative `url` under a file root, is `Skipped(Unresolvable)`, with no request.
5. If the resolved URL is in the current path set (§2a), the result is `Skipped(Cycle)`, with no request.
6. Fetches through the one hop loop of `fetch`, counting against the load's budget (§2c), then `accept_response` (ADR-008). A budget spent inside the child's redirect chain is `Skipped(Budget)`. Any other error is `Skipped(Fetch(LoadError))`. If the child's final URL is in the path set, the result is `Skipped(Cycle)`: a redirect back to an ancestor.
7. Builds the child node with `validate_at(&child, child_depth)` and recurses with the path set extended.

A failed child never fails `load` or changes another node. Only a root failure is `Err` (ADR-008). A skip is a traversal outcome, never a document diagnostic: the parent's `report` and conformance describe the parent document only (#nested-catalog-entries: "Each sub-catalog is an independent AI Catalog document"; AMB-46, AMB-192).

#### 2a. Cycle key (AMB-73, AMB-190)

The path set holds, for the current node and each of its ancestors, the URL requested for it and its `base`. A file root contributes nothing. URLs are compared as the strings `resolve_reference` and `fetch` serialise, exactly, with no further normalisation. The set holds the current path only, not every URL fetched so far: #nested-catalog-depth-and-circular-references rejects a URL "already fetched in the current traversal path". So two sibling entries naming the same catalog both load, and A → B → A is `Skipped(Cycle)` at the second A. A fragment is kept in the key. `…/a.json#x` and `…/a.json` are distinct keys, and the depth limit bounds any chain built from such variants.

#### 2b. What is never fetched

`load` requests only the root and the followed entries of §2. It never requests an artifact `url` (so display values never come from an artifact, §3), `host.logoUrl`, `host.documentationUrl`, `attestation.uri`, `statementUri`, `registryUri`, `sourceId`, `governanceUri`, or a policy-urls value. The trust crate fetches artifacts and DID documents only for verification (ADR-013, ADR-014), never `load`.

#### 2c. Request budget (M16, AMB-198)

`FetchPolicy` (ADR-008 §1) gains `max_requests: u32`, default 256. It is read only by `load`. A single `fetch` call is bounded by `1 + max_redirects` already and ignores it, so DID-document and artifact fetches (ADR-013, ADR-014) are unchanged.

- One budget per `load` call, starting at `max_requests`. It counts HTTP requests sent: the root's and every child's, each redirect hop included. A request is counted immediately before it is sent, after the hop's scheme check and address vetting, so a hop refused there (`Scheme`, `Resolve`, `DisallowedAddress`) costs nothing. A `Source::File` root costs nothing.
- The count lives in the one hop loop of `fetch` (crate-private `fetch::RequestBudget`). There is no second request path and no second redirect loop: public `fetch` runs the same loop with no budget, and `load` runs it with its budget.
- When a hop needs a request and the budget is 0, the hop is not sent. For a child, the result is `Skipped(Budget)`; requests already sent for that child stay counted. Once the budget is spent, every further followed entry is `Skipped(Budget)` (§2 step 2), whatever its depth or URL, as M16 says.
- For the root, the result is `Err(LoadError::Budget { limit })`, with `limit` = `max_requests`. This happens only when `max_requests` is 0 or smaller than the root's own redirect chain. A root failure is `Err` (ADR-008 §6), and no tree exists to carry a warning.
- After traversal, if any child is `Skipped(Budget)`, `load` appends exactly one `LoadWarning::RequestBudget { limit: u32, skipped: usize }` to the root's `warnings`: `limit` is `max_requests`, `skipped` is the number of `Skipped(Budget)` children in the whole tree. Its code is `request-budget`, its severity is warning (as every `LoadWarning`), and its `Display` names both numbers. No warning is added when nothing is skipped for the budget. It is not a document diagnostic: no node's `report` or conformance changes (AMB-192).
- The budget is spent in traversal order (AMB-196, depth-first, entry order), so which children are skipped is deterministic for a given set of responses.

### 3. Entry rows (AMB-13)

`EntryRow { index, name: String, description: Option<String>, version: Option<String> }` is built once per entry by `load`: `resolve::display_name(entry, None)`, `resolve::description(entry, None)` and `resolve::version(entry, None)` (ADR-007). The caller-supplied artifact value is always `None`, because the client never fetches an artifact to resolve a value (display-name-2, description-3, version-6: "SHOULD NOT dereference an artifact at render time solely to obtain" it). These three calls are the only resolution call sites in the client, and the front ends read `rows`. Rows hold the resolved text unchanged; text renderings sanitise it at output (§5).

### 4. `Filter` (AMB-195)

`Filter` (`#[non_exhaustive]`, public fields, `Default` matches everything) has `identifier: Option<EntryIdentifier>`, `version: Option<String>`, `artifact_type: Option<ArtifactType>` and `tag: Option<String>`. An entry matches when every criterion that is set holds:

- `identifier`: the entry's identifier equals it (ADR-004, exact).
- `version`: the entry has a `version` exactly equal to it. An entry without `version` never matches, and `""` matches only `""` (AMB-70). So `identifier` + `version` is the #multi-version-entries "match on both" rule.
- `artifact_type`: exact equality (AMB-98).
- `tag`: the entry's `tags` contains it exactly.

`ResolvedCatalog::filter(&self, &Filter) -> ResolvedCatalog` returns the same tree with only the matching rows kept in every node. Documents, reports, warnings and children, skipped ones included, are unchanged, so a nested catalog whose own entry row does not match still shows its matching descendants. Each node is filtered in its own scope (AMB-46). There is no "latest" criterion: latest selection is `resolve::latest` (ADR-007), and nothing in the client needs it as a filter today.

### 5. `DisplayText` (AMB-75, embedded-content-safety-1)

`DisplayText::new(&str) -> DisplayText` replaces every character of Unicode General Category `Cc` (U+0000..U+001F and U+007F..U+009F, a set fixed by the Unicode stability policy, so no table is needed) with U+FFFD and keeps every other character unchanged. `as_str` and `Display` expose it, and `new` is the only constructor. `"\u{1b}[2J"` becomes `"\u{FFFD}[2J"`. Every string a text rendering shows (rows, identifiers, types, host, publisher and diagnostic paths, skipped URLs, and `data` as canonical JSON text, ADR-011) goes through it. The MUST ("treat it as untrusted input") wins over #catalog-entry's "render it as given" (SHOULD) only for `Cc`, which is what lets a terminal be driven. Bidirectional and other `Cf` controls are kept (AMB-197). JSON output is not a text rendering: it carries the original strings, and JCS escapes every `Cc` as `\u00hh` or a short escape (ADR-003), so it is inert.

### 6. JSON output (AMB-194, D4, D17)

`ResolvedCatalog::to_json(&self) -> Json` never fails. Its bytes are `Json::to_canonical_vec` (ADR-003), which refuses only a string holding a noncharacter (AMB-93), and then nothing is written. Large repeated records are columnar, and repeated vocabularies are dictionary-encoded through one top-level legend:

```
{ "legend": { "code": [..], "severity": [..], "type": [..] },
  "root": Node }
Node = {
  "source": <base> | null,
  "conformance": "minimal" | "discoverable" | "not-met" | "undetermined",
  "entries": { "index": [int], "identifier": [str], "type": [int], "version": [str|null],
               "name": [str], "description": [str|null], "url": [str|null] },
  "diagnostics": { "severity": [int], "code": [int], "path": [str] },
  "children": [ { "entry": int, "catalog": Node }
              | { "entry": int, "url": str, "skipped": "budget" | "depth" | "cycle" | "unresolvable" | "fetch" } ] }
```

- Each legend array lists the distinct values used anywhere in the output, ascending by code point. `type`, `severity` and `code` columns hold indices into it. `conformance` maps ADR-005's `Met(Minimal)`, `Met(Discoverable)`, `NotMet` and `Undetermined`.
- `entries` holds the rows that are kept (after `filter`), in document order. `index` is the entry's position in the document, so diagnostic paths such as `/entries/3/digest` stay meaningful. `name`, `description` and `version` are the row values of §3. `url` is the entry's `url` as written, or `null` for `data`. `data` content is not included.
- `diagnostics` lists the node's `warnings` first (path `""`), then `report` diagnostics in report order. Client codes (`media-type-generic`) and ADR-005 codes share the `code` legend. No ADR-005 code has the text `media-type-generic`.
- A file root's `source` is `null`, never a local path, so output does not depend on the machine.
- Member names are exactly: `legend`, `code`, `severity`, `type`, `root`, `source`, `conformance`, `entries`, `index`, `identifier`, `version`, `name`, `description`, `url`, `diagnostics`, `path`, `children`, `entry`, `catalog` and `skipped`. No member carries a trust verdict, a signature or Trust Manifest content (trust-layers-2: without Layer 2, manifest content "SHOULD [be treated] as advisory, not authoritative"; AMB-60). Verification results belong to the trust crate's `TrustReport` (ADR-015).
- A `Fetch` skip carries no error detail in JSON, because error `Display` text is not a stable contract. The typed `SkipReason::Fetch(LoadError)` carries it for text renderings.
- The `request-budget` warning appears in the root's `diagnostics` as severity `warning`, code `request-budget`, path `""`. Its `limit` and `skipped` numbers are not in JSON (AMB-199): `skipped` equals the number of `"skipped": "budget"` children in the output, and `limit` is the caller's own policy. Text renderings show both through `Display`.

Goldens, in `crates/turul-ai-catalog-client/tests/fixtures/json_output/`, are hand-authored canonical bytes. Each is checked in S9 with `python3 -c 'import json,sys; sys.stdout.write(json.dumps(json.load(open(sys.argv[1])), ensure_ascii=False, separators=(",",":"), sort_keys=True))'`, which equals JCS here because every name is ASCII and every number is a small integer. The command and each file's origin are recorded in `SOURCE.md`. Our writer never produced them:

| Files | Covers |
|---|---|
| `file-root.catalog.json`, `file-root.jcs` | `Source::File` root: an entry with `displayName`, one without (the identifier segment), one with `description`, one with `version`, an inline nested catalog (a row, not a child), a relative nested `url` (`Skipped(Unresolvable)`), an entry with a signed Trust Manifest, and at least one warning and one error diagnostic (ADR-005 codes) |
| `loopback-tree/root.json`, `loopback-tree/child.json`, `loopback-tree/missing` (served as 404), `loopback-tree/extra.json`, `loopback-tree.jcs` | a loopback root served as `application/json` (`media-type-generic`), a `Child::Catalog` node, a cycle back to the root, a 404 child (`fetch`), and, loaded with `max_requests: 3`, a last root entry `extra.json` that is `Skipped(Budget)` after the root, `child.json` and `missing` have spent the budget, so the root's `diagnostics` hold `media-type-generic` then `request-budget`. `extra.json` is never requested. The golden writes the origin as `http://loopback.test`. The test substitutes the server's actual origin into the golden text before comparing; the origin occurs only inside string values, so member order is unaffected. |

Schema digest (D17): the ADR-003 §6 algorithm, through its one owner, over `crates/turul-ai-catalog-client/tests/fixtures/json_output/`. Recorded digest: `876ff3ff75dbb40f15255807f068614105a6cd56680901f3bf06f2c2e770dc99`, here, in the constant `RECORDED` of `tests/schema_digest.rs` and in the revision log, together. Any change to a golden changes all three.

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Resolved tree | `client::ResolvedCatalog`, `Nested`, `Child`, `SkipReason`, `EntryRow` | `client::load` |
| Traversal (depth, cycles, child validation) | `client::load` (traversal half), using `NestingDepth` and `validate_at` (ADR-005) | `load` |
| Request budget | `FetchPolicy::max_requests` (read only by `load`), counted in the one hop loop of `fetch` through crate-private `fetch::RequestBudget` | `load` |
| Budget warning | `client::LoadWarning::RequestBudget { limit, skipped }`, code `request-budget` | `load` |
| Cycle key | the URL string from `client::resolve_reference` (ADR-008) and `Fetched::url()` | `load` |
| Entry display values | `EntryRow`, built from `resolve::{display_name, description, version}` with `None` | `load` |
| Filtering | `client::Filter` → `ResolvedCatalog::filter` | itself; CLI `list`, TUI (ADR-011) |
| Display sanitisation | `client::DisplayText::new` | every text rendering (ADR-011) |
| JSON output | `ResolvedCatalog::to_json` → `Json::to_canonical_vec` (ADR-003) | CLI `--format json` (ADR-011) |
| JSON goldens | `crates/turul-ai-catalog-client/tests/fixtures/json_output/` with `SOURCE.md` | `tests/json_output.rs` |
| Schema digest of the goldens | §6 recorded line (S9), using ADR-003's owner `crates/turul-ai-catalog/tests/support/schema_digest.rs` | `schema_digest::json_output_goldens_match_recorded_digest` |

## Requirement → test

Test paths are under `crates/turul-ai-catalog-client/tests/`.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| nested-catalog-depth-and-circular-references-3 | "SHOULD track the set of catalog URLs visited … and reject any catalog URL that has already been fetched in the current traversal path" | `nested_traversal::catalog_on_the_current_path_is_skipped_as_cycle` (A → B → A; the log holds A and B once each), `nested_traversal::redirect_back_to_an_ancestor_is_a_cycle`, `nested_traversal::sibling_references_to_one_catalog_are_not_cycles` |
| multi-version-entries-6 | "Clients that need a specific version SHOULD match on both `identifier` and `version`" | `filter::identifier_and_version_match_exactly` (`2.0` does not match `2.0.0`; an entry without `version` never matches a version; `""` matches only `""`) |
| resolving-an-artifact-s-display-name-2 | "SHOULD NOT dereference an artifact at render time solely to obtain a name" | `nested_traversal::rows_are_resolved_without_fetching_artifacts` (an entry without `displayName`, whose `url` serves an A2A card with `name`, gets the identifier segment; the log has no request for the card) |
| resolving-an-artifact-s-description-3 | same, for a description | `nested_traversal::rows_are_resolved_without_fetching_artifacts` (`description` stays `None`) |
| resolving-an-artifact-s-version-6 | same, for a version | `nested_traversal::rows_are_resolved_without_fetching_artifacts` (`version` stays `None`) |
| safe-fetching-5 | "SHOULD prefer inline `data:` attestations and Data-URI logos to avoid leaking verification activity" | `nested_traversal::logo_and_evidence_urls_are_never_requested` (loopback `logoUrl`, `documentationUrl`, `attestation.uri`, `statementUri`, `registryUri`, `sourceId`, `governanceUri`: none in the log) |
| policy-urls-extension-4 | "Consumers fetching those documents MUST follow Safe Fetching" | `nested_traversal::policy_urls_are_never_requested` (no request, so none needs Safe Fetching; any future fetch could only go through ADR-008's `fetch`) |
| embedded-content-safety-1 | `data` content: "consumers MUST treat it as untrusted input" | `display_text::control_characters_become_replacement_characters` (all 65 `Cc`; `"\u{1b}[2J"`), `display_text::no_output_contains_a_control_character` (every scalar value), `display_text::other_characters_are_kept` (U+00A0, U+200B, U+202E, U+2028, U+FFFD, combining marks, emoji) |
| trust-layers-2 | Layer 0/1 consumers "SHOULD treat Trust Manifest content as advisory, not authoritative" | `json_output::output_carries_no_trust_verdict` (member names are exactly the §6 set in every golden; no `contributor`, `signer` or `jws` value of the fixture's manifest appears in the bytes) |

Rows owned by ADR-005 whose `url` half is pinned here:

| Requirement | Test function |
|---|---|
| nested-catalog-entries-1 (references "via `url`") | `nested_traversal::only_nested_catalog_entries_are_followed` (an A2A `url` entry, an inline `data` catalog and `Application/AI-Catalog+JSON` with `url` are not requested) |
| nested-catalog-entries-2 (reused `identifier` allowed) | `nested_traversal::each_fetched_catalog_has_its_own_report` |
| nested-catalog-entries-3, -4, nested-catalog-depth-and-circular-references-1, -2 (depth limit, 4) | `nested_traversal::five_level_url_chain_stops_at_level_four` (levels 1 to 4 are requested; level 4's child is `Skipped(Depth)` and never requested) |
| nested-catalog-entries-5 ("SHOULD document their limit") | the limit 4 is stated here, in ADR-005 and in the rustdoc of `load` |

Decisions owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| RD-08 (`url` half; inline half ADR-005) | `url` nesting followed with depth and cycle tracking, each child validated | `nested_traversal::five_level_url_chain_stops_at_level_four`, `nested_traversal::catalog_on_the_current_path_is_skipped_as_cycle`, `nested_traversal::each_fetched_catalog_has_its_own_report` |
| RD-18 (`Filter` half; resolution half ADR-007) | exact identifier + version match | `filter::identifier_and_version_match_exactly` |
| RD-33 | nested resolution exists | `nested_traversal::relative_url_resolves_against_the_fetching_document` |
| AMB-13 | no artifact is fetched to render | `nested_traversal::rows_are_resolved_without_fetching_artifacts` |
| AMB-45 (`url` half) | each `url` level adds 1 from the root's 1; limit 4; inline nesting in a child counts from the child's depth | `nested_traversal::five_level_url_chain_stops_at_level_four`, `nested_traversal::inline_nesting_inside_a_fetched_child_counts_from_its_depth` |
| AMB-46 (`url` half) | each child is an independent document with its own report and uniqueness scope | `nested_traversal::each_fetched_catalog_has_its_own_report` (one identifier in parent and child gives no uniqueness diagnostic) |
| AMB-60 (output half) | the client output carries no trust verdict | `json_output::output_carries_no_trust_verdict` |
| AMB-73 (cycle-key and traversal half; resolution owned by ADR-008) | relative `url` against the fetching document's final URL; none from a file root; keys compared exactly | `nested_traversal::relative_url_resolves_against_the_fetching_document`, `nested_traversal::relative_url_from_a_file_root_is_unresolvable`, `nested_traversal::base_is_the_final_fetched_url`, `nested_traversal::catalog_on_the_current_path_is_skipped_as_cycle` |
| AMB-75 | `Cc` → U+FFFD; everything else as given | the three `display_text` functions |
| AMB-190 | path set = requested and final URLs of the current path | `nested_traversal::redirect_back_to_an_ancestor_is_a_cycle`, `nested_traversal::sibling_references_to_one_catalog_are_not_cycles` |
| AMB-191 | only exact `AI_CATALOG` `url` entries are followed | `nested_traversal::only_nested_catalog_entries_are_followed` |
| AMB-192 | a child failure is a skip, not a diagnostic and not an error | `nested_traversal::failed_child_is_skipped_and_siblings_continue` (a 404 child gives `Skipped(Fetch(Accept(Status(404))))`, the next sibling loads, the parent's report is unchanged) |
| AMB-193 | `url` entries inside inline `data` are not followed | `nested_traversal::url_entries_inside_inline_data_are_not_followed` |
| AMB-194 | JSON output schema, columnar with sorted legends, deterministic | `json_output::file_root_output_equals_golden`, `json_output::loopback_tree_output_equals_golden`, `json_output::output_is_deterministic` (two loads give equal bytes; reading and re-canonicalising the bytes is the identity) |
| AMB-195 | filter: conjunction of exact criteria; prunes rows only | `filter::criteria_are_conjunctive`, `filter::filter_prunes_rows_only`, `filter::default_filter_keeps_everything` |
| AMB-196 | depth-first, entry order, sequential | `nested_traversal::traversal_order_is_depth_first_in_entry_order` (loopback log order) |
| AMB-197 | `Cf` kept; JSON carries raw strings | `display_text::other_characters_are_kept`, `json_output::control_characters_are_escaped_not_replaced` |
| M16 | `FetchPolicy::max_requests` (default 256) caps requests per `load`, redirects included; then every further child is `Skipped(Budget)`; one warning names the limit and the count | `nested_traversal::default_request_budget_is_256`, `nested_traversal::request_budget_counts_every_request_redirects_included` (`max_requests: 4`; root, a plain child and a child redirecting once use 4; the next three children are `Skipped(Budget)`; the loopback log has 4 requests), `nested_traversal::spent_budget_skips_every_further_child` (after the budget is spent, a cycle candidate and a child beyond the depth limit are both `Skipped(Budget)`), `nested_traversal::one_budget_warning_names_limit_and_skipped_count` (exactly one `RequestBudget { limit: 4, skipped: 3 }`, last in the root's `warnings`, none in a child's; `Display` contains `4` and `3`; every `report` equals the unbudgeted load's) |
| AMB-198 | budget accounting (§2c) | `nested_traversal::budget_spent_inside_a_redirect_chain_skips_that_child` (`max_requests: 2`; the child's first hop redirects; `Skipped(Budget)`, log length 2), `nested_traversal::root_beyond_the_budget_is_a_load_error` (`max_requests: 0` gives `Err(LoadError::Budget { limit: 0 })` with an empty log; `max_requests: 1` with a root that redirects once gives `Budget { limit: 1 }` and a log of 1), `nested_traversal::refused_hops_cost_no_budget` (a child whose `url` is `http://169.254.169.254/` is `Skipped(Fetch(..))` and the next child still loads with `max_requests: 2`), `nested_traversal::file_root_costs_no_request` (`max_requests: 1`; a file root's one absolute loopback child loads), `nested_traversal::no_budget_warning_without_a_budget_skip`, `nested_traversal::fetch_alone_ignores_the_request_budget` (`fetch` with `max_requests: 0` makes its request) |
| AMB-199 | JSON: `"skipped": "budget"` and code `request-budget`; no numbers | `json_output::loopback_tree_output_equals_golden` |
| Schema digest (D17) | ADR-003 algorithm over the goldens | `schema_digest::json_output_goldens_match_recorded_digest` |

New ids (not in PLAN §6), AMB-190 to AMB-199. The block AMB-190 to AMB-199 is reserved for this ADR, so ids added by ADRs written concurrently cannot collide. Unused numbers stay unused.

- AMB-190: the spec says "current traversal path", so siblings are not cycles. A redirect into an ancestor is the same catalog by another name, and `base` is its retrieval URL (ADR-008 AMB-184). Exact string comparison follows AMB-73, and `resolve_reference` already serialises the URL canonically (scheme and host case, default port, dot segments).
- AMB-191: #nested-catalog-entries defines the mechanism by `type` `application/ai-catalog+json`, and ADR-004 compares `type` exactly. Following any other entry would fetch an artifact (AMB-13).
- AMB-192: #nested-catalog-entries calls each sub-catalog "an independent AI Catalog document". A remote failure says nothing about the parent's conformance. Failing the whole load would let one dead link hide the catalog.
- AMB-193: an inline nested catalog's `url` entries would need its `data` mapped to an `AiCatalog`, and that mapping, `parse::document`, is crate-private to the format crate (ADR-002). Parsing its canonical bytes again would be a second path to the model, and it fails on non-I-JSON numbers. Decision: such entries are not followed. Their documents are still validated, to the depth limit, by `validate_at` (ADR-005). Trade-off: a `url` catalog referenced from inside an inline catalog is never fetched; the inline catalog appears only as its parent's row, with its content in `data`. Lifting this needs ADR-002 to expose its value → model mapping publicly, which is ADR-002's decision.
- AMB-194: §6. Columnar layout and legends follow the workspace rule for large repeated records.
- AMB-195: §4. `tags` exist "for filtering and discovery" (#catalog-entry), and `type` exists so clients can "select, filter, and route entries without parsing artifact content" (#design-goals). Exact matching follows ADR-004.
- AMB-196: deterministic output and request order. Parallel fetching is not needed today.
- AMB-198: M16 decides the budget, its default, the `Skipped(Budget)` outcome and the one warning; #nested-catalog-depth-and-circular-references names only depth and cycles, and #safe-fetching frames fetching as "a server-side request forgery (SSRF) and denial-of-service surface". What M16 leaves open is decided here. Counting at the send, after vetting, makes the budget equal what a server observes (the loopback log), and a refused hop is not a request. Counting inside the one hop loop is the only way to include redirects without a second redirect loop or a hop count leaking through `Fetched`. The root counts because M16 caps requests "per `load`"; a root that cannot be fetched within the cap is a load failure, since there is no tree to skip it in. `fetch` alone ignores the field because M16 scopes it to `load` and a single call is already bounded.
- AMB-199: the JSON `diagnostics` columns are severity, code and path (AMB-194). Adding a value column for one warning would widen every row; the count is derivable from the output and the limit is an input. Display text stays out of JSON, as for `Fetch` skips.
- AMB-197: `Cf` characters (for example U+202E) cannot drive a terminal. Replacing them would change legitimate text in scripts that need them. Visual spoofing of identifiers is #identifier-typosquatting, which is out of scope. JCS escapes `Cc` in JSON, so JSON never needs `DisplayText`.

## Divergences from ai-catalog-rust 0.4.0

| Id | ai-catalog-rust | This crate | Spec | Pinned by |
|---|---|---|---|---|
| RD-08 (`url` half) | no cycle check; `url`-nested catalogs are not fetched or validated | depth 4, path-set cycle check, each child validated with `validate_at` | #nested-catalog-depth-and-circular-references SHOULD | `nested_traversal::catalog_on_the_current_path_is_skipped_as_cycle`, `nested_traversal::five_level_url_chain_stops_at_level_four` |
| RD-18 (`Filter` half) | `get_by_id` returns the first entry with the identifier | `Filter` matches identifier and version exactly and returns every match | #multi-version-entries SHOULD | `filter::identifier_and_version_match_exactly` |
| RD-33 | `discover` returns the root only; no nested resolution | `load` builds the tree | #nested-catalog-entries | `nested_traversal::relative_url_resolves_against_the_fetching_document` |

ai-catalog-rust has no display sanitiser or JSON listing output, so §5 and §6 have no counterpart. It fetches no nested catalog (RD-33), so the request budget (§2c) has none either.

## Consequences

- One tree, built once by `load`. The CLI and TUI hold no traversal, resolution, filtering, sanitising or serialising code. ADR-011's `dependency_direction` and `render` tests and S12's "`list --format json` equals `to_json` canonical bytes" check this from the front-end side.
- Costs, accepted: a remote child is fetched again for each sibling that names it; `url` entries inside inline catalogs are not followed (AMB-193); a `fetch` skip has no detail in JSON; the request budget is shared by the whole tree in traversal order, so an early deep subtree can leave later siblings `Skipped(Budget)`, which is deterministic and named by the warning.
- Plan drift recorded:
  - PLAN §2 lists `ResolvedCatalog` fields as "document, report, base, children, entry rows". This ADR adds `depth` (for `validate_at` and the limit) and `warnings` (ADR-008 `LoadWarning`), and fixes the row and child shapes (§1, §3).
  - PLAN S9 lists `resolve_reference` as S9 code. It lands in S8 (ADR-008), and S9 uses it.
  - PLAN §2 lists `SkipReason{Depth, Cycle, Unresolvable, Fetch}`; M16 adds `Budget`. §2c also adds `FetchPolicy::max_requests` and `LoadWarning::RequestBudget` to ADR-008's types and `LoadError::Budget` for the root; ADR-008's revision log records it.
  - M16 says the CLI prints the budget warning to stderr and the TUI shows it. That is front-end behaviour owned by ADR-011, which renders `ResolvedCatalog::warnings` through `DisplayText`; ADR-011's owner adds the stderr rule and its test in S11/S12.
  - PLAN §4 maps nested-catalog-entries-1..5 to ADR-005 with the `url` half "pinned in client tests/nested_traversal.rs". This ADR names those test functions and leaves the rows with ADR-005.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created for S9: the `ResolvedCatalog` tree, `url` traversal with depth and path-set cycle tracking, entry rows without artifact fetches, `Filter`, `DisplayText`, columnar JSON output with legends, goldens and the schema-digest slot. AMB-190 to AMB-197 added in the reserved block AMB-190 to AMB-199. Request budget escalated as an open question. |
| 2026-11-17 | M16 answered the open question: §2c request budget (`FetchPolicy::max_requests`, default 256, counted per request sent in the one hop loop, redirects included), `SkipReason::Budget`, `LoadError::Budget` for the root, one `LoadWarning::RequestBudget { limit, skipped }` (`request-budget`) on the root; JSON `"skipped": "budget"`; loopback-tree golden gains `extra.json` under `max_requests: 3`. AMB-198 and AMB-199 added; the open-question paragraph is removed. |
| 2026-11-17 | S9 tests and goldens landed: `nested_traversal.rs`, `filter.rs`, `display_text.rs`, `json_output.rs`, `schema_digest.rs` and `tests/fixtures/json_output/` (hand-authored, `SOURCE.md`); `Implements:` updated. Schema digest recorded: `876ff3ff75dbb40f15255807f068614105a6cd56680901f3bf06f2c2e770dc99`. §1 clarified: a skipped child's `url` is the resolved URL whenever `resolve_reference` succeeds, including `Budget` and `Depth` skips decided before resolution; the text as written only when it does not resolve. |
| 2026-11-17 | Accepted at S9 close (0.0.11): `nested_traversal.rs`, `filter.rs`, `display_text.rs`, `json_output.rs` and `schema_digest.rs` are present and green (workspace: 329 tests pass); the digest `876ff3ff…dc99` is recorded in §6 and in `RECORDED`; traversal, filtering, `DisplayText` and `to_json` each live only in `turul-ai-catalog-client` (`tree.rs`, `display.rs`), and no front end exists yet that could bypass them (CLI and TUI land in S11/S12 through these symbols). |
