# ADR-010: Lambda well-known handler

- **Status:** Accepted (2026-11-17)
- **Date:** 2026-11-17
- **Implements:** under `crates/turul-ai-catalog-lambda/tests/`: `well_known_handler.rs`, `cold_start.rs`, `snapshot_refresh.rs`, `consumer_interop.rs`, `live_probe.rs` (`#[ignore]`). The crate's dependency rule is pinned by `crates/turul-ai-catalog-cli/tests/dependency_direction.rs` (ADR-011, the one dependency-direction test): `lambda_dependencies_are_exactly_the_allowed_set` (§1's sets).
- **Depends on:** ADR-002 (`parse`, `ParseError`), ADR-004 (`WELL_KNOWN_PATH`, `MEDIA_TYPE`), ADR-005 (`validate`, `Report`, `Severity`, `Conformance`), ADR-006 (Signature own-member checks), ADR-008 (`Fetched::new`, `accept_response`, `load`, `Source`, `FetchPolicy`: interop and live probe only), ADR-011 (`dependency_direction`), ADR-016 (`CatalogStore`, `CatalogId`, `assemble`, `import`), ADR-012 (M13 placement of header and signer-shape checks). Binding decisions: M2 (superseded in part by M19, ADR-016 §7), M4, M18, M19, M12, M13, M15, D11 (`publish = false`), D15 (`lambda_http`, `tokio` with `rt`, `macros`; dev-only `ai-catalog`, `ai-catalog-validate`), D18.
- **Spec:** https://ai-catalog.io, `specification/ai-catalog.md` as read in ADR-001 (last commit 2026-11-10). RFC 8615 (well-known URIs), RFC 9110 (HTTP semantics).

## Context

M2 adds `turul-ai-catalog-lambda`, which serves one catalog at `/.well-known/ai-catalog.json` on AWS Lambda. M19 and ADR-016 replace M2's compiled-in bundle: the catalog is assembled from a `CatalogStore` (`assemble`), at cold start and again when the snapshot is older than `max_age`. The repo owns the handler only: no IAM, SAM/CDK or table creation. Init fails on any assembly error, GET and HEAD on the well-known path serve the assembled bytes, other paths are 404 and other methods 405. M13 holds: an optional-signature defect never fails cold start, since entry Trust Manifests ride unchanged inside entry records.

Spec text this ADR implements:

- #well-known-uri: "To support automated discovery, hosts MAY serve an AI Catalog at the following well-known URI [[RFC8615]]: `/.well-known/ai-catalog.json`". "Use of the well-known URI is OPTIONAL."
- #location-independence: "When served over HTTP, the document SHOULD be served with the media type `application/ai-catalog+json`."
- #level-2-discoverable-catalog: "Includes a `host` object identifying the catalog operator"; "MAY be served at the well-known URI `/.well-known/ai-catalog.json` to enable automated domain-level discovery".
- #media-type-registration-application-ai-catalog-json: "Required parameters: N/A", "Optional parameters: N/A", "Encoding considerations: binary (UTF-8 encoded JSON [[RFC8259]])", "The document MUST contain `specVersion` and `entries` fields."
- #well-known-uri-registration-ai-catalog-json: "The well-known URI returns a JSON document with media type `application/ai-catalog+json` conforming to the AI Catalog schema".
- #level-2-discoverable-catalog, #host-info, #top-level-structure, #catalog-entry, #multi-version-entries (entry uniqueness) and #producer-behavior govern what the record gate and assembly accept (ADR-016).
- #dynamic-discovery: "Defining dynamic discovery behavior is out of scope for this specification."
- #verification-result (through M13): "Failure MUST NOT be treated as a verified endorsement. Consumers MAY retain or display unverified content".

## Decision

### 1. Crate shape and dependencies

- `turul-ai-catalog-lambda`, `publish = false` (D11). A library (`src/lib.rs`: `ServedCatalog`, `SnapshotCache`, `ColdStartError`, `handle`, `ALLOW`, `catalog_id`, `max_age`, `table_name` with feature `dynamodb`, and the three environment variable names) and a binary (`src/main.rs`: `main` only), so tests drive the handler in process (M4, "handler invoked in-process").
- Normal dependencies, all `workspace = true`: `turul-ai-catalog`, `turul-ai-catalog-storage` (default features: in-memory), `lambda_http`, `tokio` (features `rt`, `macros`) (D15). Optional: `axum` (feature `local`, M18); `aws-config` and `aws-sdk-dynamodb` (feature `dynamodb`, which also forwards to `turul-ai-catalog-storage/dynamodb`). Dev-dependencies: `turul-ai-catalog-client` (ADR-008), `ai-catalog` and `ai-catalog-validate` at `=0.4.0` (D15, M4), `futures`, `async-trait`. Nothing else. `src/` holds no `std::net` and no `std::fs` outside `src/bin/serve.rs`: the deployed function reads no file. The lambda never depends on the client or the trust crate.
- Request and response types are `lambda_http`'s re-exported `http` types; tests build requests with `lambda_http::http::Request::builder()`, so no `http` dev-dependency is added.

### 2. Assembly and cold start (M13, M15, ADR-016 §7)

- `ServedCatalog::assemble(store: &dyn CatalogStore, catalog: &CatalogId) -> Result<ServedCatalog, ColdStartError>` calls `turul_ai_catalog_storage::assemble` and holds the canonical bytes. It is the only way to build a `ServedCatalog`.
- `ColdStartError` is `#[non_exhaustive]`, implements `Display` and `std::error::Error`: `Assemble(AssembleError)` and `Config(&'static str)`. Its `Display` names every error diagnostic's `code` and `path` for an `AssembleError::Invalid`, so the Lambda init log identifies the defect, and names the missing or malformed environment variable for `Config`.
- `SnapshotCache` holds an `Arc<ServedCatalog>` and the instant it was assembled; `current(store, catalog)` re-assembles when the age is at least `max_age` (`TURUL_AI_CATALOG_MAX_AGE_SECS`, whole seconds, default 60; `0` re-assembles every request) and keeps the last good snapshot when re-assembly fails, returning the failure beside it as `(Arc<ServedCatalog>, Option<ColdStartError>)` (AMB-407). The clock lives there, never in `handle`.
- Backend selection: built with feature `dynamodb`, `main` requires `TURUL_AI_CATALOG_TABLE` and `TURUL_AI_CATALOG_ID` and fails cold start naming a missing one; built without it, `main` uses an empty `InMemoryCatalogStore` and `TURUL_AI_CATALOG_ID` defaults to `default`.
- `main` is `#[tokio::main(flavor = "current_thread")]`: it builds the store, calls `ServedCatalog::assemble` before `lambda_http::run`, so a failure returns `Err` from `main`, the process exits non-zero and Lambda reports an init failure. On success its `service_fn` closure asks the cache for the current snapshot, writes a returned refresh failure to stderr, and returns `Ok(handle(&served, &request))`; the handler never fails.
- M13 and M15 at cold start: records are parsed when written (`parse_entry`), so a Signature object missing `signer`, `profile`, `issuedAt` or `jws` is refused at `import` and never reaches the store. `turul-ai-catalog` never reads a JWS header or the `did:web` shape of a `signer` (M12, ADR-012), so a malformed `jws` header or a non-root `did:web` signer gives no diagnostic and the catalog starts and is served. The lambda performs no signature verification.

### 3. Routing and responses (`handle`)

`handle(served: &ServedCatalog, request: &Request) -> Response<Body>` is pure: no I/O, no clock, no state. The route is decided by `request.uri().path()` and `request.method()` only:

| Path | Method | Status | Headers | Body |
|---|---|---|---|---|
| exactly `WELL_KNOWN_PATH` | `GET` | 200 | `Content-Type: application/ai-catalog+json` (`MEDIA_TYPE`) | the assembled bytes |
| exactly `WELL_KNOWN_PATH` | `HEAD` | 200 | the same `Content-Type` | empty |
| exactly `WELL_KNOWN_PATH` | any other | 405 | `Allow: GET, HEAD` (`ALLOW`) | empty |
| any other path | any | 404 | none | empty |

- The path and media type come only from `turul_ai_catalog::identity::{WELL_KNOWN_PATH, MEDIA_TYPE}` (ADR-004). No string literal for either appears in the lambda's `src/` (`dependency_direction`).
- `Content-Type` is exactly `application/ai-catalog+json`, with no `charset` or other parameter (AMB-111).
- The body is the `ServedCatalog` bytes unchanged: the canonical bytes `assemble` produced, not re-serialised by the handler (D18).
- Path is compared exactly; the query string is ignored; `Accept` is ignored (AMB-112, AMB-115).
- The path check precedes the method check, so `POST /other` is 404 and `POST /.well-known/ai-catalog.json` is 405 (AMB-113).
- The handler adds no other header (no `Cache-Control`, `ETag`, CORS or `Content-Length`). Framing headers are the runtime's. No feature beyond D18 is built.

### 4. What is served (AMB-110, AMB-114)

- Any assembled catalog without an error diagnostic is served, `Met(Minimal)` included, and an empty store serves `{"entries":[],"specVersion":"1.0"}`. `host` is not required.
- Assembly validates the whole document, including inline nested catalogs in `data` (through `validate`, ADR-005). Nested catalogs referenced by `url` are other documents: the lambda does not fetch them, so they are not validated at cold start.

### 5. Interop (M4: lambda ↔ consumers) and live probe

- `consumer_interop`: for a minimal catalog and the spec example, each `import`ed into an in-memory store, the GET response is accepted by `turul_ai_catalog_client::accept_response(&Fetched::new(status, content_type, body))` with no `media-type-generic` warning, and by `ai_catalog::parse_slice(body)` followed by `ai_catalog_validate::validate` with an empty `errors`. Both agree on values: `specVersion`, `host.displayName`, `host.identifier`, and per entry in order `identifier`, `type`, `version`, `url`.
- `live_probe` (`#[ignore]`): needs `TURUL_AI_CATALOG_LIVE_URL` (the deployed handler's full well-known URL) and `AI_CATALOG_CLI` (path to the official `ai-catalog` binary from `github.com/Agent-Card/ai-catalog-cli`). It fails, naming the missing variable, when either is unset. With both set: `turul_ai_catalog_client::load(&Source::Url(url), &FetchPolicy::default())` succeeds, its document has no error diagnostic, and `<cli> validate <url>` exits 0.

### 6. Content

A deployment's content is its store, loaded by `import` or by registering writers (ADR-016). M17's producer commands still author catalog files, and a curated deployment `import`s one. The Lambda ships no default catalog.

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Served catalog bytes | `turul_ai_catalog_storage::assemble` (ADR-016 §4) | `ServedCatalog::assemble` |
| Cold-start gate | `ServedCatalog::assemble` (any `AssembleError` fails) | `main`; tests |
| Cold-start error | `ColdStartError::{Assemble(AssembleError), Config(&'static str)}` | `ServedCatalog::assemble`, `catalog_id`, `max_age`, `table_name` |
| Snapshot freshness | `SnapshotCache` | `main`'s `service_fn` closure |
| Route and response | `handle(&ServedCatalog, &Request) -> Response<Body>` | `main`'s `service_fn` closure |
| Well-known path | `turul_ai_catalog::identity::WELL_KNOWN_PATH` (ADR-004) | `handle` |
| Media type | `turul_ai_catalog::identity::MEDIA_TYPE` (ADR-004) | `handle` |
| `Allow` value | `turul_ai_catalog_lambda::ALLOW = "GET, HEAD"` | `handle` |

The served bytes are produced by `assemble` (ADR-016), whose determinism is proved there. The handler serves those bytes without a writer.

## Requirement → test

Test paths are under `crates/turul-ai-catalog-lambda/tests/`.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| well-known-uri-1 | hosts "MAY serve an AI Catalog at … `/.well-known/ai-catalog.json`" | `well_known_handler::get_serves_the_assembled_catalog_at_the_well_known_path` |
| location-independence-2 | "SHOULD be served with the media type `application/ai-catalog+json`" | `well_known_handler::content_type_is_the_ai_catalog_media_type_without_parameters` (GET and HEAD) |
| level-2-discoverable-catalog-1 | a Discoverable Catalog "MAY be served at the well-known URI" | `well_known_handler::discoverable_catalog_is_served_at_the_well_known_path` (a catalog with `host`, `Met(Discoverable)`, is served with status 200) |

Keyword sentences in the cited sections owned elsewhere or out of scope:

| Requirement | Owner |
|---|---|
| well-known-uri-2 (clients "SHOULD attempt to retrieve this well-known URL") | ADR-008, client `tests/well_known.rs` |
| well-known-uri-3 ("SHOULD use the entries' `url` values to retrieve individual artifacts") | ADR-013, trust `tests/artifact_integrity.rs` |
| well-known-uri-4 ("Use of the well-known URI is OPTIONAL") | OOS: statement (PLAN §4) |
| location-independence-1 ("MAY be served from any URL") | OOS: hosting statement (PLAN §4) |
| media-type-registration-application-ai-catalog-json-1 ("MUST contain `specVersion` and `entries`") | ADR-002, `tests/required_members.rs`; the assembled document always carries both, pinned by `cold_start::empty_store_serves_a_valid_empty_catalog` |

Maintainer decisions and plan ambiguities owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| D18 (served bytes) | the body is the `ServedCatalog` bytes, the canonical bytes `assemble` produced | `well_known_handler::get_body_is_the_canonical_assembled_bytes` (a fixture with indentation, unsorted members and `1.0` is served in canonical form) |
| D18 (HEAD) | same status and headers as GET, empty body | `well_known_handler::head_returns_the_get_headers_and_no_body` |
| D18 (404) | every other path is 404 | `well_known_handler::other_paths_are_not_found` |
| D18 (405) | every other method on the path is 405 | `well_known_handler::other_methods_are_not_allowed_with_an_allow_header` |
| D18 (cold start) | any assembly error fails init; warnings do not | `cold_start::invalid_store_fails_cold_start` (a cross-entry error; an unreachable store), `cold_start::warnings_do_not_fail_cold_start` (a `digest-algorithm-unknown` warning; a fifth nesting level, `Undetermined`), `cold_start::empty_store_serves_a_valid_empty_catalog` |
| D18 (init log) | `ColdStartError` names each error code and pointer | `cold_start::cold_start_error_names_each_error_code_and_pointer` |
| M2 / M19 | handler only; nothing fetched or read at run time; the Lambda serves the assembled document | `dependency_direction::lambda_dependencies_are_exactly_the_allowed_set`, `dependency_direction::sources_have_no_network_filesystem_or_process_access` (ADR-011), `cold_start::missing_table_configuration_fails_cold_start` (feature `dynamodb`), `cold_start::catalog_id_and_refresh_interval_are_validated` |
| AMB-407 | a stale snapshot is re-assembled; a failed re-assembly keeps the last good one and reports the failure | `snapshot_refresh::stale_snapshot_is_reassembled`, `snapshot_refresh::failed_reassembly_keeps_the_last_good_snapshot`, `snapshot_refresh::cross_entry_error_keeps_the_last_good_snapshot_and_is_reported`, `snapshot_refresh::fresh_snapshot_is_not_reassembled` |
| M4 (lambda ↔ consumers) | our client and ai-catalog-rust accept the response and agree on values | `consumer_interop::client_and_sdk_accept_the_served_catalog_and_agree_on_values` |
| M4 (live probe) | opt-in probe of a deployment | `live_probe::deployed_handler_serves_a_catalog_both_clients_accept` (`#[ignore]`), `live_probe::live_probe_requires_its_environment` (`#[ignore]`; fails naming the missing variable) |
| M13 | a malformed `jws` header or a non-root `did:web` signer never fails cold start | `cold_start::signature_defects_do_not_fail_cold_start` (header `{}`, header `{"alg":"none"}`, `signer` `did:web:Example.com` with `contributor` changed to the same value so #contributor-identity's "MUST exactly equal" holds; each is imported, starts and GET serves it) |
| M15 | a Signature object missing a required member never reaches the store | `parse::parse_entry` refuses it at the record gate (ADR-002, ADR-016); the lambda sees no such record |
| AMB-56 (server half; client half owned by ADR-008) | the lambda serves the well-known URI | `well_known_handler::get_serves_the_assembled_catalog_at_the_well_known_path` |
| AMB-110 | a catalog without `host` is served | `well_known_handler::minimal_catalog_without_host_is_served` |
| AMB-111 | `Content-Type` carries no parameter | `well_known_handler::content_type_is_the_ai_catalog_media_type_without_parameters` |
| AMB-112 | exact path; query ignored; no normalisation | `well_known_handler::query_string_does_not_change_the_response`, `well_known_handler::near_miss_paths_are_not_found` (`/.well-known/ai-catalog.json/`, `/.well-known/AI-catalog.json`, `/.well-known/ai-catalog`, `/ai-catalog.json`, `/.well-known/ai%2Dcatalog.json`) |
| AMB-113 | path before method; `Allow: GET, HEAD` on 405 | `well_known_handler::other_methods_are_not_allowed_with_an_allow_header` (`POST`, `PUT`, `DELETE`, `OPTIONS`), `well_known_handler::other_paths_are_not_found` (`POST /other` is 404) |
| AMB-114 | `url`-nested catalogs are not fetched or validated at cold start | `cold_start::url_nested_children_are_not_fetched_at_cold_start` (a catalog whose child `url` is `https://127.0.0.1/x.json` starts) |
| AMB-115 | `Accept` ignored; never 406 | `well_known_handler::accept_header_does_not_change_the_response` (`Accept: text/html`) |
| AMB-116 (superseded by ADR-016) | served bytes are canonical; a number that is not a finite double is refused at the record gate, never served | `well_known_handler::get_body_is_the_canonical_assembled_bytes`, `cold_start::non_i_json_numbers_are_refused_before_they_can_be_served` |

New ids (not in PLAN §6). The block AMB-110 to AMB-119 is used so that ids added concurrently by ADR-004 to ADR-009 (from AMB-98) cannot collide.

- AMB-110: #level-2-discoverable-catalog attaches "MAY be served at the well-known URI" to Level 2, and #well-known-uri says "hosts MAY serve an AI Catalog" with no level condition. A permission on Level 2 is not a prohibition on Level 1, and the requirement that distinguishes Level 2 is the `host` object, which `validate` reports as `Met(Discoverable)` (ADR-005). Decision: the lambda serves any assembled catalog without error diagnostics; requiring `host` would be a gate D18 does not name. The store's content decides its own level.
- AMB-111: #location-independence names the media type, and the IANA registration lists "Required parameters: N/A" and "Optional parameters: N/A" with "binary (UTF-8 encoded JSON)". A `charset` parameter is not defined for this type (RFC 8259 §11 likewise defines none for `application/json`). Decision: exactly `application/ai-catalog+json`, written as `MEDIA_TYPE`.
- AMB-112: RFC 8615 registers a path, and RFC 3986 paths are case-sensitive. Decision: `request.uri().path()` must equal `WELL_KNOWN_PATH` byte for byte. No percent-decoding, case folding or trailing-slash tolerance: a client following #well-known-uri sends the literal path, and each tolerance is a rule with no spec basis. The query string is not part of the path and is ignored, because #dynamic-discovery places content that varies by query parameters out of scope, so every caller gets the same bytes. Trade-off: the path is the one `lambda_http` presents; a deployment front end that adds a prefix (for example an API Gateway REST stage) makes the handler answer 404. Deployment wiring is not this repo's (M2); `live_probe` is the check.
- AMB-113: D18 fixes 404 and 405 without an order. Decision: path first, since a resource that does not exist has no method set (RFC 9110 §15.5.5). RFC 9110 §15.5.6: "The origin server MUST generate an Allow header field in a 405 (Method Not Allowed) response", so 405 carries `Allow: GET, HEAD`. HEAD carries the GET header fields and no content (RFC 9110 §9.3.2).
- AMB-114: D18 says "parses and validates it once at cold start", and #nested-catalog-entries makes a `url`-nested catalog a separate document. Fetching it would need the client at run time, which M2 and D18 do not grant. Decision: cold start validates the assembled catalog and its inline `data` catalogs only. Trade-off: a broken remote child is found by consumers (ADR-009 `Skipped`), not at deploy.
- AMB-115: the spec defines no content negotiation for the well-known URI. Decision: `Accept` and every other request header are ignored; the response never depends on them.
- AMB-116: superseded by ADR-016. The served body is the canonical bytes of the assembled document, so ADR-003's `NotIJson` refusal applies at the record gate and at assembly, and the body is never a verbatim file.

## Divergences from ai-catalog-rust 0.4.0

ai-catalog-rust has no server or handler, so no behaviour is compared directly. Two RD rows shape the interop test:

| Id | ai-catalog-rust | This crate | Spec | Pinned by |
|---|---|---|---|---|
| RD-06 | `ai_catalog_validate` reports `Minimal` both for a document with errors and for one without `host` | cold start fails only on errors; a `Met(Minimal)` catalog is served (AMB-110) | #conformance-levels | `consumer_interop::client_and_sdk_accept_the_served_catalog_and_agree_on_values` compares `errors` emptiness, never level |
| RD-30 | `ai-catalog-discover` ignores the response media type | the lambda always sends `application/ai-catalog+json`; our client checks it (ADR-008, AMB-74) | #location-independence | `well_known_handler::content_type_is_the_ai_catalog_media_type_without_parameters`; the SDK half of `consumer_interop` cannot observe `Content-Type`, so the client half is the media-type check |

## Consequences

- One HTTP serving path in the workspace. The served bytes are exactly what `assemble` produced from the store.
- A store that assembles to a document failing `validate` cannot deploy: Lambda init fails and the function serves nothing; a later refresh failure keeps the last good snapshot and is logged (AMB-407). A cross-entry error (an unversioned entry beside versioned entries of one identifier) passes every write and is caught only here, so one such put fails every new instance's init until the record is deleted (ADR-016). A catalog whose only signature defects are header or signer-shape defects deploys (M13), and the lambda says nothing about them; verification is `turul-ai-catalog-trust`'s, run by consumers.
- `url`-nested children and the deployment's path mapping are not checked by `cargo test`; consumers (ADR-009) and `live_probe` cover them.
- Plan drift recorded:
  - PLAN §2 lists the lambda's dev-dependencies as "client, ai-catalog"; S10's success criterion ("`ai_catalog::parse` + `validate` both accept") needs `ai-catalog-validate` too, which D15 already names as a workspace dev-dependency.
  - The lambda's dependency and source rules are pinned in the CLI crate's `dependency_direction.rs`, the one dependency-direction test (ADR-011), rather than in a lambda test file.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created for S10: crate shape, `BUNDLE`, `ServedCatalog::load` cold-start gate, `ColdStartError`, `handle` routing table, M13 and M15 at cold start, lambda ↔ consumers interop, opt-in live probe. AMB-110 to AMB-116 added. |
| 2026-11-17 | M17: `catalog.json` is produced by the CLI producer commands, not maintainer-authored. §6 adds the default content (Level 2, the spec example's three entries and host) and the recorded commands; `bundle_provenance::lambda_bundle_is_reproduced_by_the_recorded_commands` (CLI crate) and `cold_start::bundled_catalog_is_discoverable` added; owner table, status line and plan drift updated; S10 depends on S12. AMB-117 and AMB-118 added. |
| 2026-11-17 | S12 test dispute: `signature_defects_do_not_fail_cold_start` changes `contributor` with `signer` (otherwise ADR-006 `signer-contributor-mismatch`, an error, fails cold start); `dependency_direction::lambda_dependencies_are_exactly_the_allowed_set` moves to S10, which creates the crate. |
| 2026-11-17 | S10 landed: crate, handler and tests green. `live_probe` runs the official CLI as `<cli> validate <url>`; not exercised (needs a deployment and the `ai-catalog-cli` binary, whose version is therefore unrecorded). |
| 2026-11-17 | M18: adds the local development server `turul-ai-catalog-serve` (`src/bin/serve.rs`, feature `local`, `axum` optional). It adapts HTTP requests to `handle` and loads `--file` through `ServedCatalog::load`; routing, headers and the validation gate stay single-owned. `dependency_direction` now expects `axum` as the only optional dependency and exempts `src/bin/serve.rs` alone from the no-file/no-network rule. The deployed binary is unchanged. |
| 2026-11-17 | Accepted. Every test file above and `catalog.json` are present and green; `bundle_provenance::lambda_bundle_is_reproduced_by_the_recorded_commands` reproduces the bundle byte for byte; `handle` is the only code that answers a catalog request over HTTP (`src/bin/serve.rs` adapts requests to it and loads through `ServedCatalog::load`). `live_probe` stays `#[ignore]` and is not an acceptance condition. Implements line updated to the landed files. |
| 2026-11-17 | M20 (ADR-003 AMB-94): `9007199254740993` is now `number-inexact`, not `number-not-i-json`, so the AMB-116 cold-start test uses `1e400`, a number that is not a finite double. No handler behaviour changed. |
| 2026-11-17 | M19 (ADR-016 §7): the Lambda serves the document `assemble` builds from a `CatalogStore`. `BUNDLE`, `catalog.json` and `ServedCatalog::load` are deleted; cold start is `ServedCatalog::assemble`; `ColdStartError` is `{Assemble, Config}`; `SnapshotCache` refreshes a snapshot older than `max_age` and keeps the last good one on failure; feature `dynamodb` selects the DynamoDB backend. `bundle_provenance.rs`, AMB-117 and AMB-118 are deleted, AMB-116 is superseded, and the cold-start tests for parse failures and error diagnostics move to the record gate (`records.rs`, `parse_entry`). M18's `--file` goes through `import` and `assemble`. `consumer_interop` and `live_probe` no longer compare against a bundle. |
| 2026-11-17 | `SnapshotCache::current` returns `(Arc<ServedCatalog>, Option<ColdStartError>)`: a failed refresh still serves the last good snapshot, and `main` writes the failure to stderr. Consequences state that a cross-entry error accepted by the store fails every cold start (ADR-016). |
