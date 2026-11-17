# Changelog

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow SemVer.

## [Unreleased]

## [0.0.23] - 2026-11-18

### Added

- `CatalogStore::purge_catalog` removes every item stored under a catalog id, and `count_catalog` counts what it would remove, by kind (`ItemCounts`: host, entries, guards, other). Both work on backend keys alone and read no record bytes, provenance or guard values, so they work on items an earlier release wrote. In-memory and SQL purge in one atomic step; DynamoDB deletes item by item (retrying conflicts at most 8 times) and is not atomic, so a failure part-way is finished by running the purge again.
- `ImportMode::Replace { now }`: `import` makes the catalog id hold exactly the document. It overwrites the host and every document entry as curated by the writer (also an entry of another writer, an expired lease or a record an earlier release wrote), deletes entries the document lacks, and refuses with `ImportError::LiveLeases` before writing anything while a lease is live at `now`. Document entries the id already holds stay served while it runs; an identifier whose version kind changes is absent from when the old kind is deleted until the new one is written. An absent entry deleted by someone else during the replace is not an error. A document without a host leaves the stored host.
- `turul-ai-catalog store import --file PATH --writer WRITER [--replace]` and `store purge [--yes]`. `purge` previews by default and removes only with `--yes`; it refuses the id equal to `TURUL_AI_CATALOG_ID`, so the id to purge must be given as `--catalog-id`. `import` follows no nested catalog, and exits 1 only for a document with an error diagnostic.
- `CatalogStore::scan_entries` (an entry listing that reports unreadable provenance per entry instead of failing) and `Condition::Overwrite` (a put that replaces any record under its key without reading it), the two operations `Replace` needs.

### Changed

- `import` takes a fifth argument, `ImportMode`; pass `ImportMode::Merge` for the previous behaviour.
- `CatalogStore` gains `scan_entries`, `count_catalog` and `purge_catalog`, and `Condition` gains `Overwrite`: a `CatalogStore` implemented outside this crate must add them.
- Upgrading a store from 0.0.20 or earlier no longer needs a hand-written delete: replace in place with `store import --replace` against the same catalog id once no live lease exists, or import under a new id and `store purge` the old one.

### Verification

- The parity suite runs purge, overwrite and scan against in-memory, SQLite, PostgreSQL and DynamoDB Local, including items with unreadable provenance and a DynamoDB table holding the earlier release's items.

## [0.0.22] - 2026-11-18

### Changed

- `turul_ai_catalog_client::fetch(url, accept, policy)` takes a caller-named `Accept` (`Option<&str>`), sent on every hop for content negotiation (RFC 9110 §12.5.1), e.g. `Some("application/a2a-agent-card+json")` for an A2A card. Pass `None` for the previous request. Safe Fetching is unchanged. See ADR-008.

### Added

- `FetchError::InvalidAccept`: an `Accept` value that is not a valid HTTP header value, refused before any request is sent.

### Compatibility

- `fetch` gains a parameter; callers add `None` to keep their behaviour. The stored entry format is unchanged, so no re-import is needed from 0.0.21.

## [0.0.21] - 2026-11-17

**Breaking for anyone holding a store:** re-import under a new catalog id before deploying (see ADR-017).

### Added

- `turul_ai_catalog_storage::Registrar`: opt-in runtime registration over a `CatalogStore`. A `RegistrationPolicy` gives each `WriterId` a `WriterGrant` of `urn:air` publishers and non-`urn:air` identifier prefixes, and sets the lease length; a writer without a grant is refused. `register` creates, updates and refreshes; `unregister` removes the writer's own entry. A lease refresh writes only once less than three quarters of the lease remain. A grant permits leases only; `WriterGrant::with_curated` also permits curated entries, and a curated request under a lease-only grant is refused with `RegistrationError::CuratedNotGranted` before the store is read. Writers granted the same publisher or prefix share its identifiers: each may add versions of an identifier another registered, but cannot overwrite or remove the other's records. A grant may require a publisher signature, checked on every write by a `PublisherVerifier` given to `Registrar::new`; without one construction fails with `PolicyError::VerifierMissing`. See ADR-017.
- `Provenance` (a lease with its writer and expiry, or curated with its writer), `ModeKind`, `WriterId` and `StoredEntry` in the storage crate. `StoreError` gains `ModeConflict` and `CorruptProvenance`: a put that would replace a record of the other mode fails atomically on every backend.
- `assemble` leaves expired leases and entries that fail the current gate out of the served document and reports each in `Assembled::omitted` (`Omission`, `OmissionReason`); `Assembled::next_expiry` is the earliest expiry among the served leases.
- `list_registrations` and `remove_registration` in the storage crate, and the CLI commands `turul-ai-catalog store list` and `store remove`, which list a store's entries with their provenance and expiry state and remove one entry by identifier and version. The backend is `--sqlite`, `--postgres` (CLI feature `postgres`) or `--table` (DynamoDB, CLI feature `dynamodb`), each also read from an environment variable, as is `--catalog-id`. The CLI now depends on `turul-ai-catalog-storage` (with `sqlite`) and `tokio`.
- `turul-ai-catalog-trust` feature `registration`: `DidWebPublisherVerifier`, a `PublisherVerifier` that accepts an entry only with a verified `did-web-v1` publisher signature covering the entry and its artifact bytes, and `publisher_verdict`. The feature adds `turul-ai-catalog-storage`, `async-trait` and `tokio`; the default build is unchanged.
- `turul_ai_catalog_trust::verify_entry` and `verify_entry_resolved`: the per-entry trust report.
- `turul_ai_catalog::identity::is_root_did_web_domain`: the root `did:web` domain rule, now shared by `DidWebRoot::parse` and `WriterGrant::new`.
- `StoreError::Contended`: the DynamoDB backend retries a write that conflicts with other writers' transactions at most 8 times, with jittered exponential backoff, then returns it; `Registrar::register` reports it as `RegistrationError::Contended`. The storage `dynamodb` feature adds `tokio` (`time`) for the backoff timer.
- `PolicyError::LeaseTtlOutOfRange`: `RegistrationPolicy::new` refuses a lease length whose lease, written at the Unix epoch, would end past year 9999. `RegistrationError::ExpiryOutOfRange`: a lease whose `now + lease_ttl` ends past year 9999 is refused before the store is read, where it used to panic on overflow or store an expiry no read could parse.
- `Provenance::to_json`.

### Changed

- `CatalogStore::put_entry` takes a `&Provenance`; `get_entry` and `list_entries` return `StoredEntry`. The entry `Etag` now covers the entry and its provenance, so every entry `Etag` changes value.
- `assemble(store, catalog, now)` and `ServedCatalog::assemble(store, catalog, now)` take the time at which leases are judged.
- `import(store, catalog, document, writer)` records every entry as curated by `writer`, and refuses an entry whose key holds a lease (`StoreError::ModeConflict`) or another writer's curated record (`ImportError::OwnedByAnotherWriter`). The writer id is permanent configuration for that catalog. `ServedCatalog::from_document` imports with a fixed writer id of its own; its signature is unchanged.
- `SnapshotCache::new(served, max_age, clock)` takes the clock whose reading it passes to `assemble`, refreshes at the earlier of `max_age` and the snapshot's next lease expiry, and the Lambda logs one line per omitted entry on each assembly. `ServedCatalog` gains `omitted()` and `next_expiry()`.
- `Registrar` owns its store and verifier: `Registrar::new(Arc<dyn CatalogStore>, CatalogId, RegistrationPolicy, Option<Arc<dyn PublisherVerifier>>)`, and `Registrar` has no lifetime, so it can live in a service's static state.
- `Provenance::to_bytes` and `Etag::of_entry` return `Result<_, ProvenanceError>` and refuse a provenance whose bytes would not read back. Both are written by the core crate's canonical writer (`Json::to_canonical_vec`); stored bytes and entry `Etag`s are unchanged.
- `import` reports a record other writers kept changing as `ImportError::Store(StoreError::Contended)`, not `PreconditionFailed`.
- The workspace depends on `turul-ai-catalog-storage` without default features and on `tokio` with `rt` only; each crate asks for what it uses. The trust `registration` feature now adds storage without a backend and `tokio` with `rt` only, as ADR-017 states.
- The illustrative MCP Server Card URL in the READMEs is `https://mcp.example.com/mcp/server-card` (turul-mcp servers serve `<streamable-http-url>/server-card`).

### Compatibility

- Records written before this release have no provenance and are not read as curated. sqlite and postgres `connect` fail on a table from an earlier release (the table gains `provenance` and `mode` columns); on DynamoDB such items read as `StoreError::CorruptProvenance`. Re-import under a new catalog id (sqlite: a new file; postgres: a new database), deploy the function on that id, then delete the old id's items. Never point the previous release at a new-format catalog id. See ADR-017.
- `StoreError`, `ImportError` and `Assembled` gain variants or fields; all are `#[non_exhaustive]`.

### Fixed

- Every backend's `put_entry` refuses, with `StoreError::CorruptProvenance` and before touching the store, a provenance whose bytes would not read back, so no backend stores a record its own reads would refuse.
- The sqlite and postgres backends report a stored mode that is missing or unknown as `StoreError::CorruptProvenance`, as DynamoDB does, instead of `StoreError::Backend` text.
- The storage crate builds without default features under `-D warnings`; CI runs that clippy step.

### Verification

- fmt, clippy `-D warnings` (all storage features, Lambda `local`), all tests and doctests, the DynamoDB Local and PostgreSQL parity suites, and `cargo doc -D warnings` pass.

## [0.0.20] - 2026-11-17

### Added

- `turul_ai_catalog_lambda::ServedCatalog::from_document(&AiCatalog)` builds the served catalog from a caller-supplied document through the in-memory store (`import`, then `assemble`): the same gate as cold start, with no file read and no default catalog. Errors are `ColdStartError::Import` and `ColdStartError::Assemble`. The local server uses it. See ADR-010.
- `turul_ai_catalog::identity::UrnAir`: the `urn:air` identifier parser now lives in the core crate, so building and validating a catalog no longer needs the trust crate. See ADR-004.

### Fixed

- `turul_ai_catalog_storage::import` validates the whole document first and refuses one with errors (`ImportError::Invalid`) before writing anything. Two entries with the same identifier and version used to collapse into one record, and the assembled catalog passed with an entry missing. See ADR-016.
- ADR-010 records that the Lambda crate is published. ADR-001, ADR-003 and ADR-013 are Accepted.

### Removed

- `turul_ai_catalog_trust::UrnAir` and `turul_ai_catalog_trust::identity::UrnAir`; import `turul_ai_catalog::identity::UrnAir` instead.

### Compatibility

- `ImportError` and `ColdStartError` gain a variant each; both are `#[non_exhaustive]`.

## [0.0.19] - 2026-11-17

### Added

- `turul_ai_catalog::error::chain`: renders an error followed by its `source()` chain in one line. The CLI, the local server and the Lambda's refresh log use it.
- `Display` and `std::error::Error` for `DigestError`, `PolicyUrlsError` and `TimestampError`.
- API documentation for every public item in every crate: summaries, `# Errors` sections, offline examples, and spec links. `missing_docs` is enforced. On docs.rs, the storage crate documents all backends and marks the feature that enables each one.

### Changed

- Error `Display` messages follow the Rust API Guidelines: each states only its own failure, and the cause is returned by `source()` (trust, tui and client errors). Callers that printed `to_string()` and relied on the cause being included should use `error::chain`.

### Verification

- fmt, clippy `-D warnings` with all features, all tests and doctests, the DynamoDB Local and PostgreSQL parity suites, `cargo doc -D warnings`, and the docs.rs nightly build pass.

## [0.0.18] - 2026-11-17

### Added

- `StoreError::VersionKindConflict { identifier }`, returned by `put_entry`.
- `RecordKey::guard`: the backend key of an identifier's version-kind guard, `"g:"` and a fixed-length digest. See ADR-016.

### Fixed

- `CatalogStore::put_entry` now enforces the #multi-version-entries rule that an unversioned entry's `identifier` alone is unique. It refuses a new entry whose identifier already has stored entries of the other version kind (unversioned beside versioned, or the reverse) with `VersionKindConflict`. Every backend makes the check and the write one atomic step, so concurrent writers cannot both pass: in-memory under its lock, sqlite and postgres in one transaction holding a per-identifier guard row, DynamoDB in one `TransactWriteItems` with a per-identifier guard item. Deleting the last entry of a kind frees the identifier for the other kind. One accepted put can no longer stop every Lambda instance from serving the catalog. See ADR-016.
- `import` of a document that has both kinds for one identifier now fails with `ImportError::Store(VersionKindConflict)` at the second entry, instead of storing a catalog that cannot be assembled.

### Compatibility

- sqlite and postgres: `connect` creates a second table, `catalog_guards`, when missing; a postgres role needs the same privilege for it as for `catalog_records`, or the table pre-created. DynamoDB: no table change; guard items share the table (`sk` = `"g:…"`). Writers need `dynamodb:UpdateItem`, `dynamodb:ConditionCheckItem` and `dynamodb:GetItem` beside `PutItem` and `DeleteItem`; the serving role is unchanged.
- Stores written by 0.0.17 hold no guards, so `put_entry` cannot see the version kind of their records, and a conflict they already hold stays. Re-run `import` (or the registering writers) into an empty catalog or table to get the guarantee. Assembly still validates the whole document, so such a store keeps failing assembly rather than serving an invalid catalog.

### Verification

- The parity suite runs the new checks on in-memory, sqlite and DynamoDB Local, including writers racing an unversioned entry against versioned ones; fmt, clippy `-D warnings`, test and `cargo doc` pass.

## [0.0.17] - 2026-11-17

### Added

- `turul-ai-catalog-storage`: the `CatalogStore` trait, the record gate, entry assembly and `import`, with backends behind features: `in-memory` (default), `sqlite`, `postgres` and `dynamodb`. Every backend passes the same parity suite. See ADR-016.
- `turul-ai-catalog-lambda` feature `dynamodb`: serve a catalog from a DynamoDB-backed store.

### Changed

- `turul-ai-catalog-lambda` serves an AI Catalog assembled from a catalog store at the well-known URI instead of a bundled `catalog.json`; the bundled file is removed. See ADR-016.
- `turul-ai-catalog-serve` (the local server in `turul-ai-catalog-lambda`): `--file` goes through `import` and `assemble`; there is no default bundle. See ADR-016.
- A warm Lambda instance re-assembles a snapshot older than `TURUL_AI_CATALOG_MAX_AGE_SECS` (default 60); a failed refresh keeps the last good snapshot, is written to stderr, and is retried once per max age rather than on every request.

### Removed

- The bundle provenance check in `turul-ai-catalog-cli` and the bundled Lambda catalog.

### Compatibility

- The store does not enforce the spec rule that an unversioned entry's `identifier` alone is unique. A put of an unversioned entry beside versioned entries of the same identifier (or the reverse) is accepted, and every assembly of that catalog then fails: new Lambda instances fail to start and warm ones keep their last snapshot until the record is deleted. See ADR-016.

### Verification

- fmt, clippy `-D warnings` (workspace and all storage features), test, the DynamoDB-local parity test and `cargo doc` pass.

## [0.0.16] - 2026-11-17

### Added

- `validate::Code::NumberInexact` (`number-inexact`): a number whose RFC 8785 form denotes another value (`9007199254740993`, `333333333.33333329`, `1e-400`). Always a warning, signed and digested contexts included. See ADR-005.

### Changed

- The canonical writer (`canonical::to_canonical_vec`, `Json::to_canonical_vec`) is exactly RFC 8785. Every finite number is written as the ECMAScript form of its nearest double, also when that form denotes another value (`9007199254740993` as `9007199254740992`, `333333333.33333329` as `333333333.3333333`, `1e-400` as `0`). `CanonicalError::NotIJson` is returned for a number only when it is not a finite double (`1e400`). Noncharacters are still refused. See ADR-003 and DECISIONS M20.
- `number-not-i-json` now means a number that is not a finite double: an error in a signed or digested context, a warning elsewhere. See ADR-005.
- `turul-ai-catalog add --data` writes such a number in its RFC 8785 form, prints the `number-inexact` warning and exits 0; a number that is not a finite double still exits 2.
- The project and `turul-ai-catalog` READMEs no longer say that such numbers are refused.

### Fixed

- Signed payloads (`turul_ai_catalog_trust::payload`) and inline `data` digests that an RFC 8785 producer computed over such a number could not be reproduced, so their signatures and digests failed. They now verify, and `ai_catalog_trust::canonicalize` gives the same bytes for every number form `sdk_interop` checks.

### Compatibility

- Re-serialising a document changes the value of each `number-inexact` number. An attestation `size` above 2^53 that a double does not hold is written in its RFC 8785 form; `18446744073709551615` becomes `18446744073709552000`, which `parse` refuses. Keep exact large numbers as strings where the spec allows it.

### Verification

- `number_diagnostics`, `i_json_numbers`, `producer_commands` and `sdk_interop::numbers_canonicalise_as_ai_catalog_trust_does` pin the new behaviour; fmt, clippy `-D warnings`, test and `cargo doc` pass for the workspace.

## [0.0.15] - 2026-11-17

### Changed

- `turul_ai_catalog_trust::SignerParams::signer` is a `DidWebRoot`, not a `String`. `sign_manifest` and `sign_catalog` write `profile: did-web-v1`, so they now take only a root `did:web` signer; the `kid` is built by `header::Kid::parse`. `SignError::Kid` now means only a `key_fragment` that is not a non-empty RFC 3986 fragment. See ADR-013.
- The `turul-ai-catalog` README states that a number RFC 8785 would round to another value is refused rather than written. See ADR-003.

### Fixed

- `sign_manifest` and `sign_catalog` signed for any non-empty signer without `#` (such as `https://example.com/x`, `did:web:Example.COM` or `did:example:123`), producing `did-web-v1` signatures that `header::inspect` refuses.
- `turul_ai_catalog_client::fetch`: `FetchPolicy::timeout` now bounds the body read. A server sending the body slowly kept the call open past the deadline, because only each read was bounded. See ADR-008.

### Compatibility

- Callers that build `SignerParams` pass `DidWebRoot::parse(signer)` instead of a string.

### Verification

- `safe_fetching::trickling_body_times_out` and `signing::signer_outside_the_did_web_profile_cannot_sign` added; fmt, clippy `-D warnings`, test and `cargo doc` pass for the workspace.

## [0.0.14] - 2026-11-17

### Added

- New crate `turul-ai-catalog-trust` (`publish = false`): signing and verification of trust manifests and catalogs. `payload::{manifest_payload, catalog_payload}` build the JCS signed payload; `digest::{compute, verify_artifact}` check artifact integrity; `sign::{sign_manifest, sign_catalog, SigningKey, SignerParams}` produce detached ES256 `jws` values; `header::{policy, inspect, kid_matches_signer}` vet JWS headers; `identity::{DidWebRoot, UrnAir}` parse signer identities; `did_web::{document_url, accept_document, resolve}` and `DidDocument` resolve `did:web` documents through `turul_ai_catalog_client::fetch`, with `select_key`. See ADR-012, ADR-013 and ADR-014.
- Acceptance and reporting in the trust crate: `TrustPolicy`, `SignedObject`, `verify_signature`, `entry_release_coverage`, `publisher_authorized`, `catalog_endorsed`, `select_current`, `DidDocuments`, `verify_resolved`, `verify` and `TrustReport` with `Reason`. See ADR-015.
- New crate `turul-ai-catalog-lambda` (`publish = false`): `ServedCatalog`, `ColdStartError` and `handle` serve a bundled catalog at `/.well-known/ai-catalog.json` (GET and HEAD; 404 for other paths, 405 for other methods). A bundle with an error diagnostic fails at cold start. See ADR-010.
- `turul-ai-catalog`, `-client`, `-tui` and `-cli` are publishable (`publish = true`) with README usage, categories and package excludes for test fixtures.
- `turul-ai-catalog-serve`: a local development HTTP server in `turul-ai-catalog-lambda`, behind the optional `local` feature. It serves a catalog file, or the bundled catalog, with the Lambda's own handler and cold-start validation: `cargo run -p turul-ai-catalog-lambda --features local --bin turul-ai-catalog-serve -- --file ai-catalog.json`. The deployed Lambda binary is unchanged. See ADR-010.
- Getting-started documentation. The project README has a CLI and a library quick start. The `turul-ai-catalog` and `turul-ai-catalog-client` READMEs are the crates' rustdoc, and every Rust snippet in them is compiled and run as a doctest.

### Compatibility

- Nothing is published to crates.io by this release.

### Verification

- fmt, clippy `-D warnings`, test and `cargo doc` pass for the workspace.

## [0.0.13] - 2026-11-17

### Added

- New crate `turul-ai-catalog-cli` with binary `turul-ai-catalog`: `validate`, `list` and `view` over one explicit source flag (`--file`, `--url`, `--well-known`), exit codes 0/1/2, canonical JSON output and load warnings on stderr; producer commands `init`, `add` and `remove`. See ADR-011 §3 and §4.
- `turul_ai_catalog_client::produce` with `Change`, `HostInput`, `EntryInput`, `Content` and `ProduceError`: the one creation API for catalog files (start document, change, `validate`, canonical bytes, atomic write).
- `ResolvedCatalog::has_errors`: whether any loaded catalog in the tree has an error diagnostic.
- `crates/turul-ai-catalog-lambda/catalog.json`: the lambda bundle, produced by the recorded `init` and `add` commands (M17).

## [0.0.12] - 2026-11-17

### Added

- New crate `turul-ai-catalog-tui` (library): `render(&ResolvedCatalog, &ViewState, &mut Frame)`, `ViewState` with `on_key` (Up/Down, `k`/`j`, `q`/Esc/Ctrl-C), `Action`, and `run` (`RunError::{NotATerminal, Io}`). Every document-derived string reaches the buffer through `DisplayText`; `data` is shown only as its canonical JSON text; trust manifests are shown as a count labelled not verified. See ADR-011 §2.

## [0.0.11] - 2026-11-17

### Added

- `load` follows `url` entries of type `application/ai-catalog+json` depth-first in entry order, to nesting level 4, with a current-path cycle check on the requested and the final URL. Each child is validated with `validate_at` at its own depth. A failed child is `Child::Skipped { url, reason }` with `SkipReason::{Budget, Depth, Cycle, Unresolvable, Fetch(LoadError)}`; only a root failure is `Err`. See ADR-009.
- `ResolvedCatalog` gains `depth`, `rows` (`EntryRow { index, name, description, version }`, resolved without fetching any artifact) and `children` (`Nested { entry, child }`).
- `FetchPolicy::max_requests` (default 256): requests one `load` may send, redirect hops included, counted in the one hop loop of `fetch`. `LoadError::Budget { limit }` for a root beyond it; `LoadWarning::RequestBudget { limit, skipped }` (code `request-budget`) on the root when children are skipped for it. `LoadWarning` implements `Display`. `fetch` alone ignores the field.
- `Filter` and `ResolvedCatalog::filter`: exact `identifier`, `version`, `artifact_type` and `tag`, all that are set must hold; only rows are pruned.
- `DisplayText`: replaces every Unicode `Cc` character with U+FFFD for text renderings.
- `ResolvedCatalog::to_json`: columnar output with sorted legends and no trust verdict; canonical bytes through `Json::to_canonical_vec`.

### Changed

- `FetchError::Transport` holds an `Arc` and `LoadError::Io` an `Arc<io::Error>`, so `FetchError`, `AcceptError`, `LoadError` and the whole tree are `Clone`.

## [0.0.10] - 2026-11-17

### Added

- New crate `turul-ai-catalog-client` (`publish = false`): load a catalog from a file, a URL or a host's well-known URI, with Safe Fetching. See ADR-008.
- `fetch(&str, &FetchPolicy) -> Result<Fetched, FetchError>`: the one request path for catalogs, DID documents and artifacts. One GET; redirects (301, 302, 303, 307, 308 with `Location`) are followed by `fetch` itself and every hop is re-vetted. `https` only. Each hop's host is resolved once, refused if any address is not globally reachable (loopback, private, link-local, `fc00::/7`, IPv4-mapped forms, cloud metadata addresses), and connected only to the vetted addresses (no second lookup). No proxy, TLS 1.2 minimum, `Accept-Encoding: identity`.
- `FetchPolicy` (`#[non_exhaustive]`, `Default`): `max_body_bytes` 4 MiB, `timeout` 10 s for the whole call, `max_redirects` 5, `permit_loopback_http` `false`.
- `Fetched { status, content_type, body }` with `url()` (the final hop's URL) and `Fetched::new(status, content_type, body)` for testing gates without a network (`url()` is `None`).
- `FetchError`: `InvalidUrl`, `Scheme`, `Resolve`, `DisallowedAddress(IpAddr)`, `TooManyRedirects`, `TooLarge`, `Timeout`, `ContentEncoding`, `ContentType`, `Transport`.
- `resolve_reference(Option<&str>, &str) -> Result<String, Unresolvable>`: the one URL-reference resolver (WHATWG, via `url`). A relative reference with no base is `Unresolvable`.
- `Source::{File, Url, WellKnown(WellKnownHost)}`, `Source::request_url()` and `WellKnownHost::new(&str) -> Result<_, InvalidHost>` (a host with optional `:port`; `https://<host>/.well-known/ai-catalog.json`).
- `accept_response(&Fetched) -> Result<Accepted, AcceptError>`: the one catalog response gate. Status exactly 200, media type `application/ai-catalog+json` (or `application/json` with the warning `LoadWarning::MediaTypeGeneric`, code `media-type-generic`), then the strict reader. `AcceptError::{Status, MediaType, Parse}`.
- `load(&Source, &FetchPolicy) -> Result<ResolvedCatalog, LoadError>`: read or fetch, accept, parse and `validate` the root catalog. `ResolvedCatalog` carries `document`, `report`, `base` and `warnings`. Nested-catalog traversal lands in S9.

### Compatibility

- `fetch` and `load` are blocking. From async code, call them through `tokio::task::spawn_blocking`.
- Refused by design, each with a named error: a non-200 2xx, `text/json` or a missing `Content-Type`, a compressed response, a host whose DNS also returns a private address, and fetching through a proxy.
- `Source::Url` fetches exactly the URL given and never tries the well-known URI; no `Link` header or HTML is read.
- A file source has no media-type rule and no size ceiling.

### Verification

- `safe_fetching.rs`, `well_known.rs` and `response_acceptance.rs` are covered by `cargo test -p turul-ai-catalog-client` against a loopback server (offline). TLS and the well-known request over the network are not exercised in CI.

## [0.0.9] - 2026-11-17

### Added

- `docs/plan/upstream-issues.md`: draft issues for ai-catalog-rust 0.4.0, one each for RD-01, RD-02, RD-03, RD-04, RD-06, RD-08, RD-12 and RD-13. Each gives the spec anchor, the observed 0.4.0 behaviour and the `sdk_interop.rs` test that pins it. Nothing has been filed from this repository; the maintainer files them (M11).

### Compatibility

- No code, API or behaviour change.

## [0.0.8] - 2026-11-17

### Added

- Interop testing against ai-catalog-rust 0.4.0 (`ai-catalog`, `ai-catalog-validate`, `ai-catalog-trust`, pinned `=0.4.0` as dev-dependencies only; nothing new at runtime). For every spec example, upstream fixture and interop fixture, our accept/reject matches theirs or the difference is a recorded divergence (RD-01, RD-02, RD-04, RD-08, RD-12, RD-13, RD-14, RD-15) with its spec anchor. See ADR-001 §8.
- Our canonical bytes parse to equal values in `ai_catalog`, and their serialisation parses to equal values in ours, except where `ai_catalog` drops empty arrays or `"data": null` (RD-02, RD-03).
- `to_canonical_vec(parse(b))` equals `ai_catalog_trust::canonicalize(b)` for the original bytes of every corpus document.

### Compatibility

- No public API change. Where we differ from ai-catalog-rust, the spec decides: we refuse a missing required member, a duplicate member name, a `contributor` outside RFC 3986, uppercase digest hex and a `+`-signed `specVersion`, where 0.4.0 accepts them.

### Verification

- `sdk_interop.rs` is covered by `cargo test -p turul-ai-catalog` (offline once dependencies are fetched).

## [0.0.7] - 2026-11-17

### Added

- `resolve::display_name(&CatalogEntry, Option<&str>) -> &str`: the entry's `displayName`, else the artifact name you pass, else the trailing segment of the identifier (`urn:air:example.com:mcp:weather` gives `weather`). See ADR-007.
- `resolve::description` and `resolve::version` (`-> Option<&str>`): the entry's value, else the artifact value you pass, else `None`. An entry `version` wins over a differing artifact version.
- `resolve::latest(&AiCatalog, &EntryIdentifier) -> Option<&CatalogEntry>`: highest SemVer 2.0.0 precedence when every version in the group parses, else latest `updatedAt` when every entry has one, else `None`. Ties give `None`; array order never decides.

### Compatibility

- A present but empty `displayName` (or an empty artifact name you pass) is returned as `""`, not replaced by the identifier segment.
- No `semver` dependency; SemVer parsing is crate-private.

### Verification

- `resolution.rs` and `latest_version.rs` are covered by `cargo test -p turul-ai-catalog` (offline).

## [0.0.6] - 2026-11-17

### Added

- `validate::validate_manifest(&TrustManifest) -> Report`: the one validator of a Trust Manifest and its signature, usable on a standalone manifest. Pointers are relative to the manifest. See ADR-006.
- `validate::DID_WEB_V1` (`did-web-v1`) and `validate::RESERVED_PROFILES`.
- `Subject::for_entry(&CatalogEntry) -> Option<Subject>` (`None` when the entry has no `digest`), `Subject::binds(&CatalogEntry)` and `TrustManifest::signer_matches_contributor()`.
- Ten diagnostic codes: `contributor-not-absolute-uri`, `manifest-not-substantive`, `manifest-subject-missing`, `signer-contributor-mismatch`, `signer-not-absolute-uri`, `profile-empty`, `profile-unrecognised` (a warning), `signature-expiry-order`, `subject-mismatch`, `entry-digest-missing`.

### Changed

- `validate` now checks every Trust Manifest and signature in the catalog, nested catalogs included, with the same rules as `validate_manifest`. A document that passed 0.0.5 can now be `NotMet`: for example a manifest whose `expiresAt` is not after `issuedAt` (instants compared, so `+10:00` and `Z` forms agree), an empty `profile`, a signer that differs from the contributor, a subject that does not match its entry, or a malformed `sourceDigest` (the spec's multi-artifact example, whose `sha256:99998888...` is truncated).
- Profiles compare exactly: `did-web-v1` is recognised, `DID-WEB-V1` is not and gives `profile-unrecognised`. An absolute-URI profile is accepted.

### Compatibility

- No signature is decoded or verified; header, `did:web` shape and key checks belong to the trust crate (M12, M13).

### Verification

- `manifest_validity.rs`, `subject_binding.rs` and `signature_object.rs` are covered by `cargo test -p turul-ai-catalog` (offline).

## [0.0.5] - 2026-11-17

### Added

- `validate::{validate, validate_at, Report, Diagnostic, Severity, Code, Conformance, ConformanceLevel, NestingDepth, MAX_NESTING_DEPTH}`: the one document validator. 17 diagnostic codes, each with a JSON Pointer; conformance is `Met(Minimal)`, `Met(Discoverable)`, `NotMet` or `Undetermined` (nesting limit 4). See ADR-005.
- `digest::{DigestValue, DigestAlgorithm, DigestError}`, `timestamp::{Timestamp, TimestampError}` (RFC 3339 `date-time`, ordered by instant), `extensions::{POLICY_URLS, PolicyUrls, PolicyUrlsError}` and `CatalogEntry::policy_urls`.

## [0.0.4] - 2026-11-17

### Added

- `identity::EntryIdentifier` and `identity::HostIdentifier`: opaque identifiers, stored verbatim and compared exactly (no trimming, case folding or scheme parsing). `new(impl Into<String>)` is the one constructor, `as_str()` the one reader. See ADR-004.
- `identity::EntryKey`, built only by `CatalogEntry::key()`: the entry's (`identifier`, `version`) as parsed. Absent, empty (`""`) and non-empty versions give distinct keys.
- `identity::ArtifactType` with the 10 known types of #catalog-entry as consts (`AI_CATALOG`, `AGENT_CARD`, `A2A_AGENT_CARD`, `MCP_SERVER_CARD`, `AGENT_SKILLS_JSON`, `AGENT_SKILLS_MD`, `AGENT_SKILLS_ZIP`, `AGENT_SKILLS_GZIP`, `AGENT_PLUGINS_ZIP`, `AGENT_PLUGINS_GZIP`). Any string is accepted; comparison is exact and case-sensitive.
- `identity::MEDIA_TYPE` (`application/ai-catalog+json`, equal to `ArtifactType::AI_CATALOG`) and `identity::WELL_KNOWN_PATH` (`/.well-known/ai-catalog.json`).

### Changed

- Breaking: `CatalogEntry::identifier` and `Subject::identifier` are `EntryIdentifier`, `CatalogEntry::artifact_type` and `Subject::artifact_type` are `ArtifactType`, and `HostInfo::identifier` is `Option<HostIdentifier>` (all were `String`). `CatalogEntry::new` and `Subject::new` take the typed values. Use `as_str()` where a `&str` is needed.

### Compatibility

- Parsing and serialisation output are unchanged: identifiers and types round-trip byte for byte. Empty identifiers and types still parse; rejecting them is left to validation.
- `Publisher.identifier`, `TrustSchema.identifier`, `Signature.signer` and `Attestation.attestation_type` stay `String`.

### Verification

- `entry_key.rs` and `artifact_types.rs` are covered by `cargo test -p turul-ai-catalog` (offline).

## [0.0.3] - 2026-11-17

### Added

- RFC 8785 (JCS) serialisation, the one JSON writer of the crate: `canonical::to_canonical_vec(&AiCatalog) -> Result<Vec<u8>, CanonicalError>` and `Json::to_canonical_vec(&self)`. No whitespace, members sorted by UTF-16 code units, array order (including `entries`) kept, strings escaped as ECMAScript does. See ADR-003.
- `AiCatalog::to_json` and `TrustManifest::to_json` (`-> Json`): the model as a `Json` value. Unrecognised members and `additionalSignatures` (`null` included) are written unchanged; an absent optional array and `[]` stay distinct.
- Numbers are written in their RFC 8785 form, the ECMAScript form of the nearest IEEE 754 double (`1.0` as `1`, `1e2` as `100`, `0.1` as `0.1`), only when that form is the same decimal value. A number is never rounded.
- `canonical::CanonicalError::NotIJson { pointer }`: returned, with the JSON Pointer of the first defect in canonical order, for a number whose RFC 8785 form would be another value (`9007199254740993`, `18446744073709551617`, a 30-digit integer, `0.1234567890123456789012345`, `333333333.33333329`, `1e-400`) or is not finite (`1e400`), and for a string or member name holding a Unicode noncharacter. This applies to every number, in `data` (inline catalogs included), `extensions`, `additionalSignatures` and unrecognised members as much as to an attestation `size` (`9007199254740993`, `9223372036854775808` and `18446744073709551615` are refused; `9007199254740992` and `18014398509481984` are written as themselves), so no value or `size` ceiling is ever changed.

### Changed

- `json::read(&[u8]) -> Result<Json, ParseError>` is now public, unchanged, for reading bare JSON values that are not catalog documents.

### Compatibility

- `parse(to_canonical_vec(m)) == m` for every document that serialises, every vendored spec example and upstream fixture included. Numbers are written in their shortest ECMAScript form (`1.0` becomes `1`); the parsed value is equal.
- A document holding such a number or a noncharacter still parses, but cannot be serialised. RFC 8785's own sample input is refused at `333333333.33333329`. Keep large or exact numbers as strings, as the spec recommends.
- A name the spec defines for an object, inserted into that object's `unrecognized` members, is never written; only the typed field is.
- Still zero runtime dependencies; `sha2` is a dev-dependency only.

### Verification

- RFC 8785 sample and Appendix B vectors and the spec signed-payload example are covered by `cargo test -p turul-ai-catalog` (offline).

## [0.0.2] - 2026-11-17

### Added

- `turul_ai_catalog::parse::parse(&[u8]) -> Result<AiCatalog, ParseError>`: strict RFC 8259 reader and typed model for AI Catalog `specVersion` 1.x documents (top level, Host Info, entries, Publisher, Trust Manifest and its Subject, Signature, Trust Schema, Attestation and Provenance objects). See ADR-002.
- `ParseError` carries a `ParseErrorKind` and the JSON Pointer of the offending value. Kinds: `Syntax`, `InvalidUtf8`, `DepthLimit`, `LoneSurrogate`, `DuplicateMember`, `SpecVersionInvalid`, `UnsupportedMajor`, `MissingMember`, `WrongType`, `ArtifactSourceConflict`, `ArtifactSourceMissing`.
- `json::Json`, `json::Number` (keeps the source lexeme, so large integers and long decimals are never rounded; equality compares the exact decimal value, for an exponent of any length) and `json::JsonPointer`.
- Unrecognised members on every object are kept in the model (`Members`).
- Producer constructors (`AiCatalog::new`, `CatalogEntry::new`, ...) that set only the required members; `AiCatalog::new` sets `specVersion` to `"1.0"` (`identity::SPEC_VERSION`).

### Compatibility

- Refused, by design: duplicate member names at any depth, unpaired surrogate escapes, a leading byte order mark, nesting deeper than 128, an entry with both or neither of `url` and `data`, and a `specVersion` major other than 1.
- Accepted: strings and member names holding Unicode noncharacters (U+FDD0..U+FDEF, U+xFFFE, U+xFFFF), raw or escaped, are kept unchanged.
- Zero runtime dependencies. No serialiser yet: parse only.
- Minimum supported Rust version is 1.95.

### Verification

- Every CDDL-required member, every vendored spec example and the reader edge cases are covered by `cargo test -p turul-ai-catalog` (offline).

## [0.0.1] - 2026-11-17

### Added

- Workspace scaffold: `turul-ai-catalog` library stub, CI, contributor guidance. No public API yet.
