# ADR-010: Lambda well-known handler

- **Status:** Accepted (2026-11-17)
- **Date:** 2026-11-17
- **Implements:** under `crates/turul-ai-catalog-lambda/tests/`: `well_known_handler.rs`, `cold_start.rs`, `consumer_interop.rs`, `live_probe.rs` (`#[ignore]`); `crates/turul-ai-catalog-lambda/catalog.json`; and `crates/turul-ai-catalog-cli/tests/bundle_provenance.rs` (§6, AMB-118). The crate's dependency rule is pinned by `crates/turul-ai-catalog-cli/tests/dependency_direction.rs` (ADR-011, the one dependency-direction test): `lambda_dependencies_are_exactly_the_allowed_set` (§1's sets).
- **Depends on:** ADR-002 (`parse`, `ParseError`), ADR-004 (`WELL_KNOWN_PATH`, `MEDIA_TYPE`), ADR-005 (`validate`, `Report`, `Severity`, `Conformance`), ADR-006 (Signature own-member checks), ADR-008 (`Fetched::new`, `accept_response`, `load`, `Source`, `FetchPolicy`: interop and live probe only), ADR-011 (`dependency_direction`; producer commands `init`, `add`, `remove` and `turul_ai_catalog_client::produce`, §4 there), ADR-012 (M13 placement of header and signer-shape checks). Binding decisions: M2, M4, M12, M13, M15, M17, D11 (`publish = false`), D15 (`lambda_http`, `tokio` with `rt`, `macros`; dev-only `ai-catalog`, `ai-catalog-validate`), D18.
- **Spec:** https://ai-catalog.io, `specification/ai-catalog.md` as read in ADR-001 (last commit 2026-11-10). RFC 8615 (well-known URIs), RFC 9110 (HTTP semantics).

## Context

M2 adds `turul-ai-catalog-lambda`, which serves one bundled catalog file at `/.well-known/ai-catalog.json` on AWS Lambda, validated through `turul-ai-catalog` at cold start. The repo owns the handler only: no IAM, SAM/CDK, S3 or runtime generation. D18 fixes the mechanism: `crates/turul-ai-catalog-lambda/catalog.json` is bundled with `include_bytes!`, parsed and validated once at cold start, init fails on any error diagnostic, and the bytes are served verbatim to GET and HEAD on the well-known path, with 404 for other paths and 405 for other methods. M13 adds that an optional-signature defect never fails cold start. M17 replaces D18's "maintainer content": the catalog is created by tooling, never hand-written; `catalog.json` is produced by the CLI producer commands (ADR-011), and a test re-creates it with the recorded commands and asserts byte equality with the committed bundle. S10 runs after S12.

Spec text this ADR implements:

- #well-known-uri: "To support automated discovery, hosts MAY serve an AI Catalog at the following well-known URI [[RFC8615]]: `/.well-known/ai-catalog.json`". "Use of the well-known URI is OPTIONAL."
- #location-independence: "When served over HTTP, the document SHOULD be served with the media type `application/ai-catalog+json`."
- #level-2-discoverable-catalog: "Includes a `host` object identifying the catalog operator"; "MAY be served at the well-known URI `/.well-known/ai-catalog.json` to enable automated domain-level discovery".
- #media-type-registration-application-ai-catalog-json: "Required parameters: N/A", "Optional parameters: N/A", "Encoding considerations: binary (UTF-8 encoded JSON [[RFC8259]])", "The document MUST contain `specVersion` and `entries` fields."
- #well-known-uri-registration-ai-catalog-json: "The well-known URI returns a JSON document with media type `application/ai-catalog+json` conforming to the AI Catalog schema".
- #level-2-discoverable-catalog, #host-info, #top-level-structure, #catalog-entry, #multi-version-entries (entry uniqueness) and #producer-behavior govern the default bundle's content; the producer commands that apply them are ADR-011's (§4 there).
- #dynamic-discovery: "Defining dynamic discovery behavior is out of scope for this specification."
- #verification-result (through M13): "Failure MUST NOT be treated as a verified endorsement. Consumers MAY retain or display unverified content".

## Decision

### 1. Crate shape and dependencies

- `turul-ai-catalog-lambda`, `publish = false` (D11). A library (`src/lib.rs`: `BUNDLE`, `ServedCatalog`, `ColdStartError`, `handle`, `ALLOW`) and a binary (`src/main.rs`: `main` only), so tests drive the handler in process (M4, "handler invoked in-process").
- Normal dependencies, all `workspace = true`: `turul-ai-catalog`, `lambda_http`, `tokio` (features `rt`, `macros`) (D15). Dev-dependencies: `turul-ai-catalog-client` (ADR-008), `ai-catalog` and `ai-catalog-validate` at `=0.4.0` (D15, M4). Nothing else. `src/` holds no `std::net` and no `std::fs`: the bundle is compiled in, so nothing is read, fetched or generated at run time (M2). The lambda never depends on the client or the trust crate in normal dependencies, so it never fetches and never verifies a signature. `dependency_direction` (ADR-011) pins all of this.
- Request and response types are `lambda_http`'s re-exported `http` types; tests build requests with `lambda_http::http::Request::builder()`, so no `http` dev-dependency is added.

### 2. The bundle and cold start (D18, M13, M15)

- `pub const BUNDLE: &[u8] = include_bytes!("../catalog.json");` is the only catalog the crate serves. `catalog.json` is produced by the CLI producer commands (M17, ADR-011 §4), never hand-written; its content and provenance are §6.
- `ServedCatalog::load(bytes: &'static [u8]) -> Result<ServedCatalog, ColdStartError>` is the one validation gate: `turul_ai_catalog::parse::parse(bytes)`, then `turul_ai_catalog::validate::validate(&catalog)`. A `ParseError` gives `ColdStartError::Parse(ParseError)`. A `Report` holding any diagnostic of `Severity::Error` gives `ColdStartError::Invalid(Report)`. Warnings never fail (D18 "fails init on any error diagnostic"), and neither does `Conformance::Undetermined` from a nesting-depth warning (ADR-005). `ServedCatalog` holds only the `&'static [u8]`: it exists only for bytes that passed this gate, and the parsed model is dropped because nothing serves it.
- `ColdStartError` is `#[non_exhaustive]`, implements `Display` and `std::error::Error`. Its `Display` names the `ParseErrorKind` and pointer, or every error diagnostic's `code` and `path`, so the Lambda init log identifies the defect.
- `main` is `#[tokio::main(flavor = "current_thread")]`: it calls `ServedCatalog::load(BUNDLE)?` before `lambda_http::run`, so an invalid bundle returns `Err` from `main`, the process exits non-zero and Lambda reports an init failure (D18). On success it runs `lambda_http::run(service_fn(...))` whose closure returns `Ok(handle(&served, &request))`; the handler never fails.
- M13 and M15 at cold start: `turul-ai-catalog` never reads a JWS header or the `did:web` shape of a `signer` (M12, ADR-012), so a malformed `jws` header or a non-root `did:web` signer gives no diagnostic and the bundle starts and is served. The lambda performs no signature verification, so no signature outcome exists to fail on. A Signature object missing `signer`, `profile`, `issuedAt` or `jws` is a `ParseError` (M15, ADR-002) and fails cold start, as does any other ADR-006 Signature own-member error (empty `profile`, `expiresAt` not after `issuedAt`, `signer` not an `absolute-URI`), because those keep their error severity (M15).

### 3. Routing and responses (`handle`)

`handle(served: &ServedCatalog, request: &Request) -> Response<Body>` is pure: no I/O, no clock, no state. The route is decided by `request.uri().path()` and `request.method()` only:

| Path | Method | Status | Headers | Body |
|---|---|---|---|---|
| exactly `WELL_KNOWN_PATH` | `GET` | 200 | `Content-Type: application/ai-catalog+json` (`MEDIA_TYPE`) | `BUNDLE` bytes, verbatim |
| exactly `WELL_KNOWN_PATH` | `HEAD` | 200 | the same `Content-Type` | empty |
| exactly `WELL_KNOWN_PATH` | any other | 405 | `Allow: GET, HEAD` (`ALLOW`) | empty |
| any other path | any | 404 | none | empty |

- The path and media type come only from `turul_ai_catalog::identity::{WELL_KNOWN_PATH, MEDIA_TYPE}` (ADR-004). No string literal for either appears in the lambda's `src/` (`dependency_direction`).
- `Content-Type` is exactly `application/ai-catalog+json`, with no `charset` or other parameter (AMB-111).
- The body is the bundle bytes unchanged: not re-serialised, not canonicalised, not reformatted (D18; AMB-116).
- Path is compared exactly; the query string is ignored; `Accept` is ignored (AMB-112, AMB-115).
- The path check precedes the method check, so `POST /other` is 404 and `POST /.well-known/ai-catalog.json` is 405 (AMB-113).
- The handler adds no other header (no `Cache-Control`, `ETag`, CORS or `Content-Length`). Framing headers are the runtime's. No feature beyond D18 is built.

### 4. What is served (AMB-110, AMB-114)

- Any bundle that passes §2 is served, `Met(Minimal)` included. `host` is not required.
- Cold start validates the bundled document, including inline nested catalogs in `data` (through `validate`, ADR-005). Nested catalogs referenced by `url` are other documents: the lambda does not fetch them (no client in normal dependencies, M2), so they are not validated at cold start.

### 5. Interop (M4: lambda ↔ consumers) and live probe

- `consumer_interop`: for the bundled `catalog.json` and every lambda test fixture that passes §2, the GET response is accepted by `turul_ai_catalog_client::accept_response(&Fetched::new(status, content_type, body))` with no `media-type-generic` warning, and by `ai_catalog::parse_slice(body)` followed by `ai_catalog_validate::validate` with an empty `errors`. Both agree on values: `specVersion`, `host.displayName` and `host.identifier` when present, and per entry, in order, `identifier`, `type`, `version` and `url`. The comparison never uses ai-catalog-validate's level, because its `Minimal` doubles as the has-errors state (RD-06). A fixture that hits another RD row is not used here; the S7 corpus (ADR-001) covers RD disagreements.
- `live_probe` (`#[ignore]`): needs `TURUL_AI_CATALOG_LIVE_URL` (the deployed handler's full well-known URL) and `AI_CATALOG_CLI` (path to the official `ai-catalog` binary from `github.com/Agent-Card/ai-catalog-cli`). It fails, naming the missing variable, when either is unset. With both set: `turul_ai_catalog_client::load(&Source::Url(url), &FetchPolicy::default())` succeeds and its document equals `parse(BUNDLE)`; then the official CLI is run against the URL and must exit 0. The CLI's argument list for "fetch and validate a URL" is not in the local corpus (`ai-catalog-cli` is not under `target/upstream/`), so S10 records the exact arguments and the CLI version it ran against in this ADR's revision log. The probe is opt-in and never runs in CI (DECISIONS M4 table).

### 6. Bundle content and provenance (M17)

Default content (AMB-117): a minimal valid Level 2 (Discoverable) catalog describing the AI Catalog spec's own example artifacts, the three entries of `specification/examples/ai-catalog.json` (ADR-001 corpus), with `host` taken from the same file. Each entry carries only the members the producer commands set (`identifier`, `type`, `url`, `displayName`, `description`, `tags`); `validate` reports `Met(Discoverable)` and no error.

The recorded commands, run in this order from the directory that holds `catalog.json` (the file must not exist):

```
turul-ai-catalog init --file catalog.json --host-display-name "Acme Services Inc." --host-identifier did:web:acme-corp.com --host-documentation-url https://docs.acme-corp.com/agents
turul-ai-catalog add  --file catalog.json --identifier urn:air:acme-corp.com:agent:finance --type application/a2a-agent-card+json --url https://api.acme-corp.com/agents/acme-finance-agent.json --description "Multi-protocol finance agent." --tag finance --tag trading
turul-ai-catalog add  --file catalog.json --identifier urn:air:acme-corp.com:data:market-2026q1 --type application/parquet --url https://data.acme-corp.com/datasets/market-dataset-2026q1.parquet --display-name "Market Dataset Q1 2026" --description "Artifact-focused card for training and evaluation." --tag dataset --tag finance
turul-ai-catalog add  --file catalog.json --identifier urn:air:acme-corp.com:plugin:finance-assistant-plugin --type application/agent-plugins+zip --url https://plugins.acme-corp.com/finance-assistant.zip --display-name "Finance Assistant Plugin" --description "Agent Plugin bundle with skills and MCP server for finance workflows." --tag finance --tag plugin --tag skills --tag mcp
```

- The test `crates/turul-ai-catalog-cli/tests/bundle_provenance.rs` is the one place these commands are recorded, as argument arrays. `lambda_bundle_is_reproduced_by_the_recorded_commands` runs each through the CLI binary (`env!("CARGO_BIN_EXE_turul-ai-catalog")`) in a fresh temporary directory, asserts each exits 0, and asserts the resulting file equals `include_bytes!("../../turul-ai-catalog-lambda/catalog.json")` byte for byte. The committed file is therefore the canonical bytes `to_canonical_vec` writes (ADR-003), with no trailing newline.
- The served bytes stay the committed file, verbatim (D18): the lambda never runs the commands, and nothing is generated at build or run time (M2). Provenance is a test-time check only.
- A deployer replaces the default by deleting `catalog.json`, running `init` and `add` with their own values, and updating the argument arrays in `bundle_provenance.rs` in the same change, so the test keeps reproducing the committed file. The default names the spec example's host, `did:web:acme-corp.com`; it is a working example and a test fixture, and is not meant to be deployed as anyone's catalog (AMB-117).

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Served catalog bytes | `crates/turul-ai-catalog-lambda/catalog.json`, produced by the CLI producer commands through `turul_ai_catalog_client::produce` (ADR-011 §4), compiled in as `turul_ai_catalog_lambda::BUNDLE` | `main` → `ServedCatalog::load(BUNDLE)` |
| Bundle provenance (the recorded commands) | `crates/turul-ai-catalog-cli/tests/bundle_provenance.rs` | `cargo test -p turul-ai-catalog-cli` |
| Cold-start validation gate | `ServedCatalog::load` (`parse` then `validate`; fails on `ParseError` or any `Severity::Error`) | `main`; tests |
| Cold-start error | `ColdStartError::{Parse(ParseError), Invalid(Report)}` | `ServedCatalog::load` |
| Route and response | `handle(&ServedCatalog, &Request) -> Response<Body>` | `main`'s `service_fn` closure |
| Well-known path | `turul_ai_catalog::identity::WELL_KNOWN_PATH` (ADR-004) | `handle` |
| Media type | `turul_ai_catalog::identity::MEDIA_TYPE` (ADR-004) | `handle` |
| `Allow` value | `turul_ai_catalog_lambda::ALLOW = "GET, HEAD"` | `handle` |

The bundle is a produced artifact: its one producer is `turul_ai_catalog_client::produce`, reached through the CLI commands; its bytes are `canonical::to_canonical_vec` of the model, whose schema digest is ADR-003's; and `bundle_provenance` proves the canonical producer reproduces it deterministically. No other golden or schema digest is added here. The handler serves those bytes without a writer.

## Requirement → test

Test paths are under `crates/turul-ai-catalog-lambda/tests/`.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| well-known-uri-1 | hosts "MAY serve an AI Catalog at … `/.well-known/ai-catalog.json`" | `well_known_handler::get_serves_the_bundle_at_the_well_known_path` |
| location-independence-2 | "SHOULD be served with the media type `application/ai-catalog+json`" | `well_known_handler::content_type_is_the_ai_catalog_media_type_without_parameters` (GET and HEAD) |
| level-2-discoverable-catalog-1 | a Discoverable Catalog "MAY be served at the well-known URI" | `well_known_handler::discoverable_bundle_is_served_at_the_well_known_path` (a bundle with `host`, `Met(Discoverable)`, is served with status 200) |

Keyword sentences in the cited sections owned elsewhere or out of scope:

| Requirement | Owner |
|---|---|
| well-known-uri-2 (clients "SHOULD attempt to retrieve this well-known URL") | ADR-008, client `tests/well_known.rs` |
| well-known-uri-3 ("SHOULD use the entries' `url` values to retrieve individual artifacts") | ADR-013, trust `tests/artifact_integrity.rs` |
| well-known-uri-4 ("Use of the well-known URI is OPTIONAL") | OOS: statement (PLAN §4) |
| location-independence-1 ("MAY be served from any URL") | OOS: hosting statement (PLAN §4) |
| media-type-registration-application-ai-catalog-json-1 ("MUST contain `specVersion` and `entries`") | ADR-002, `tests/required_members.rs`; the lambda inherits it through `ServedCatalog::load`, pinned by `cold_start::unparseable_bundle_fails_cold_start` |

Maintainer decisions and plan ambiguities owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| D18 (bundle, verbatim) | the body is `BUNDLE`'s bytes unchanged | `well_known_handler::get_body_is_the_bundle_bytes_verbatim` (a fixture with indentation, unsorted members and `1.0` is served byte-identical) |
| D18 (HEAD) | same status and headers as GET, empty body | `well_known_handler::head_returns_the_get_headers_and_no_body` |
| D18 (404) | every other path is 404 | `well_known_handler::other_paths_are_not_found` |
| D18 (405) | every other method on the path is 405 | `well_known_handler::other_methods_are_not_allowed_with_an_allow_header` |
| D18 (cold start) | any error diagnostic or `ParseError` fails init; warnings do not | `cold_start::bundle_with_an_error_diagnostic_fails_cold_start`, `cold_start::unparseable_bundle_fails_cold_start` (invalid JSON; missing `entries`; `specVersion` `2.0`), `cold_start::warnings_do_not_fail_cold_start` (a `digest-algorithm-unknown` warning; a fifth nesting level, `Undetermined`) |
| D18 (shipped bundle) | the committed `catalog.json` passes the gate | `cold_start::bundled_catalog_loads` |
| M17 (bundle provenance) | `catalog.json` is produced by the CLI producer commands, re-created byte for byte by the recorded commands | `bundle_provenance::lambda_bundle_is_reproduced_by_the_recorded_commands` |
| AMB-117 (default content) | Level 2 catalog of the spec example's three entries and host | `cold_start::bundled_catalog_is_discoverable` (`validate(parse(BUNDLE))` has no error and is `Met(Discoverable)`; its entries' (`identifier`, `type`, `url`) equal those of `specification/examples/ai-catalog.json` in order, and `host` equals that file's `host`) |
| AMB-118 (test placement) | provenance runs the real binary from the CLI crate | `bundle_provenance::lambda_bundle_is_reproduced_by_the_recorded_commands` |
| D18 (init log) | `ColdStartError` names each error code and pointer | `cold_start::cold_start_error_names_each_error_code_and_pointer` |
| M2 | handler only; nothing fetched, read or generated at run time | `dependency_direction::lambda_dependencies_are_exactly_the_allowed_set`, `dependency_direction::sources_have_no_network_filesystem_or_process_access` (ADR-011) |
| M4 (lambda ↔ consumers) | our client and ai-catalog-rust accept the response and agree on values | `consumer_interop::client_and_sdk_accept_the_served_catalog_and_agree_on_values` |
| M4 (live probe) | opt-in probe of a deployment | `live_probe::deployed_handler_serves_the_bundle_to_both_clients` (`#[ignore]`), `live_probe::live_probe_requires_its_environment` (`#[ignore]`; fails naming the missing variable) |
| M13 | a malformed `jws` header or a non-root `did:web` signer never fails cold start | `cold_start::signature_defects_do_not_fail_cold_start` (header `{}`, header `{"alg":"none"}`, `signer` `did:web:Example.com` with `contributor` changed to the same value so #contributor-identity's "MUST exactly equal" holds; each starts and GET serves it) |
| M15 | a Signature object missing a required member fails cold start | `cold_start::signature_missing_a_required_member_fails_cold_start` (`ColdStartError::Parse` naming the missing member's pointer) |
| AMB-56 (server half; client half owned by ADR-008) | the lambda serves the well-known URI | `well_known_handler::get_serves_the_bundle_at_the_well_known_path` |
| AMB-110 | a bundle without `host` is served | `well_known_handler::minimal_bundle_without_host_is_served` |
| AMB-111 | `Content-Type` carries no parameter | `well_known_handler::content_type_is_the_ai_catalog_media_type_without_parameters` |
| AMB-112 | exact path; query ignored; no normalisation | `well_known_handler::query_string_does_not_change_the_response`, `well_known_handler::near_miss_paths_are_not_found` (`/.well-known/ai-catalog.json/`, `/.well-known/AI-catalog.json`, `/.well-known/ai-catalog`, `/ai-catalog.json`, `/.well-known/ai%2Dcatalog.json`) |
| AMB-113 | path before method; `Allow: GET, HEAD` on 405 | `well_known_handler::other_methods_are_not_allowed_with_an_allow_header` (`POST`, `PUT`, `DELETE`, `OPTIONS`), `well_known_handler::other_paths_are_not_found` (`POST /other` is 404) |
| AMB-114 | `url`-nested catalogs are not fetched or validated at cold start | `cold_start::url_nested_children_are_not_fetched_at_cold_start` (a bundle whose child `url` is `https://127.0.0.1/x.json` starts) |
| AMB-115 | `Accept` ignored; never 406 | `well_known_handler::accept_header_does_not_change_the_response` (`Accept: text/html`) |
| AMB-116 | bytes verbatim, so no writer refusal at cold start | `well_known_handler::get_body_is_the_bundle_bytes_verbatim`, `cold_start::non_i_json_numbers_outside_signed_objects_do_not_fail_cold_start` (`1e400` in `data` of an unsigned entry) |

New ids (not in PLAN §6). The block AMB-110 to AMB-119 is used so that ids added concurrently by ADR-004 to ADR-009 (from AMB-98) cannot collide.

- AMB-110: #level-2-discoverable-catalog attaches "MAY be served at the well-known URI" to Level 2, and #well-known-uri says "hosts MAY serve an AI Catalog" with no level condition. A permission on Level 2 is not a prohibition on Level 1, and the requirement that distinguishes Level 2 is the `host` object, which `validate` reports as `Met(Discoverable)` (ADR-005). Decision: the lambda serves any bundle without error diagnostics; requiring `host` would be a gate D18 does not name. The bundle decides its own level (the default is Level 2, AMB-117).
- AMB-111: #location-independence names the media type, and the IANA registration lists "Required parameters: N/A" and "Optional parameters: N/A" with "binary (UTF-8 encoded JSON)". A `charset` parameter is not defined for this type (RFC 8259 §11 likewise defines none for `application/json`). Decision: exactly `application/ai-catalog+json`, written as `MEDIA_TYPE`.
- AMB-112: RFC 8615 registers a path, and RFC 3986 paths are case-sensitive. Decision: `request.uri().path()` must equal `WELL_KNOWN_PATH` byte for byte. No percent-decoding, case folding or trailing-slash tolerance: a client following #well-known-uri sends the literal path, and each tolerance is a rule with no spec basis. The query string is not part of the path and is ignored, because #dynamic-discovery places content that varies by query parameters out of scope, so every caller gets the same bytes. Trade-off: the path is the one `lambda_http` presents; a deployment front end that adds a prefix (for example an API Gateway REST stage) makes the handler answer 404. Deployment wiring is not this repo's (M2); `live_probe` is the check.
- AMB-113: D18 fixes 404 and 405 without an order. Decision: path first, since a resource that does not exist has no method set (RFC 9110 §15.5.5). RFC 9110 §15.5.6: "The origin server MUST generate an Allow header field in a 405 (Method Not Allowed) response", so 405 carries `Allow: GET, HEAD`. HEAD carries the GET header fields and no content (RFC 9110 §9.3.2).
- AMB-114: D18 says "parses and validates it once at cold start", and #nested-catalog-entries makes a `url`-nested catalog a separate document. Fetching it would need the client at run time, which M2 and D18 do not grant. Decision: cold start validates the bundle and its inline `data` catalogs only. Trade-off: a broken remote child is found by consumers (ADR-009 `Skipped`), not at deploy.
- AMB-115: the spec defines no content negotiation for the well-known URI. Decision: `Accept` and every other request header are ignored; the response never depends on them.
- AMB-116: D18 serves the bytes verbatim, so no JCS writer runs and ADR-003's `NotIJson` refusal never applies to the served body. A number that is not I-JSON in an unsigned place is an ADR-005 warning and does not fail cold start; in a signed object it is an ADR-005 error and does.
- AMB-117: M17 asks for a catalog "produced by these commands", and the commands set only `identifier`, `type`, `url`/`data`, `displayName`, `description`, `version`, `tags` and `host`. The spec example (`specification/examples/ai-catalog.json`) also carries `updatedAt`, `publisher`, `digest` and a `trustManifests` with a signed manifest. Those are left out: the commands cannot set them (ADR-011 AMB-348), the example's `jws` ends in the placeholder `detached-jws-signature` and would serve an unverifiable signature, and its `digest`s describe artifact bytes this repo does not serve. What remains is Level 2 by #level-2-discoverable-catalog ("Includes a `host` object"), with every entry Level 1 complete (#level-1-minimal-catalog: `identifier`, `type`, exactly one of `url` or `data`), unique by identifier (#multi-version-entries), and `specVersion` `1.0` (#producer-behavior). `host` is copied from the same example rather than invented: the entries' `urn:air` identifiers name `acme-corp.com` as publisher, and a host of our own would describe an operator the entries do not belong to. Trade-off: deploying the default unchanged names Acme as the operator; §6 says the deployer regenerates it first.
- AMB-118: M17 asks for the recorded commands to run through the CLI binary (or the library entrypoint). Cargo builds a package's binary only for that package's own integration tests (`CARGO_BIN_EXE_<name>`), and the lambda must not depend on the CLI (ADR-011 `dependency_direction`). `assert_cmd` is not in D15 and a new dependency is a stop condition (PLAN §5). So the test lives in the CLI crate and uses `std::process::Command`, as `dependency_direction` already pins lambda rules from there. Running the binary, not `produce`, also tests the argument mapping the deployer types. ADR-010 owns the test; ADR-011's `dependency_direction` source scans cover only `src/`, so the test's use of `std::process` is allowed.

## Divergences from ai-catalog-rust 0.4.0

ai-catalog-rust has no server or handler, so no behaviour is compared directly. Two RD rows shape the interop test:

| Id | ai-catalog-rust | This crate | Spec | Pinned by |
|---|---|---|---|---|
| RD-06 | `ai_catalog_validate` reports `Minimal` both for a document with errors and for one without `host` | cold start fails only on errors; a `Met(Minimal)` bundle is served (AMB-110) | #conformance-levels | `consumer_interop::client_and_sdk_accept_the_served_catalog_and_agree_on_values` compares `errors` emptiness, never level |
| RD-30 | `ai-catalog-discover` ignores the response media type | the lambda always sends `application/ai-catalog+json`; our client checks it (ADR-008, AMB-74) | #location-independence | `well_known_handler::content_type_is_the_ai_catalog_media_type_without_parameters`; the SDK half of `consumer_interop` cannot observe `Content-Type`, so the client half is the media-type check |

## Consequences

- One HTTP serving path in the workspace. The served bytes are exactly the committed bundle, so a deploy publishes what review saw.
- A bundle that fails `validate` cannot deploy: Lambda init fails and the function serves nothing. A bundle whose only signature defects are header or signer-shape defects deploys (M13), and the lambda says nothing about them; verification is `turul-ai-catalog-trust`'s, run by consumers.
- `url`-nested children and the deployment's path mapping are not checked by `cargo test`; consumers (ADR-009) and `live_probe` cover them.
- Plan drift recorded:
  - PLAN §2 lists the lambda's dev-dependencies as "client, ai-catalog"; S10's success criterion ("`ai_catalog::parse` + `validate` both accept") needs `ai-catalog-validate` too, which D15 already names as a workspace dev-dependency.
  - PLAN §2 lists `ServedCatalog::load` and `handle`; this ADR adds `BUNDLE` (so `live_probe` and `bundled_catalog_loads` compare against the shipped bytes) and `ALLOW` (RFC 9110 §15.5.6).
  - The lambda's dependency and source rules are pinned in the CLI crate's `dependency_direction.rs`, the one dependency-direction test (ADR-011), rather than in a lambda test file.
  - M17 supersedes PLAN §0b D18's "(maintainer content)": `catalog.json` is produced by the CLI producer commands (§6). S10 therefore depends on S12, and adds `bundle_provenance.rs` to the CLI crate's tests.

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
