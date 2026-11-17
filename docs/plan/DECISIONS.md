# Maintainer decisions

Answers to plan asks. The plan and ADRs cite these by id. Only the maintainer adds or changes a row.

| Id | Date | Decision |
|---|---|---|
| M1 | 2026-11-17 | Workspace and repo stay `turul-agent-card`. The format crate is `turul-ai-catalog`: the single turul owner of the AI Catalog format, with no network or async dependency. |
| M2 | 2026-11-17 | Yes: add a `turul-ai-catalog-lambda` crate that serves one **bundled** catalog file at `/.well-known/ai-catalog.json` on AWS Lambda, validated through `turul-ai-catalog` at cold start. This repo owns the handler only, not deployment infrastructure (IAM, SAM/CDK). No S3, no runtime generation. |
| M3 | 2026-11-17 | Yes: add a CLI, a Rust client and a ratatui TUI that **share common code**. The consumer logic lives in one crate, `turul-ai-catalog-client`. The CLI and TUI are thin presentation layers over it and hold no catalog logic of their own. |
| M4 | 2026-11-17 | Yes: interop tests wherever a boundary meets another implementation. `ai-catalog-rust` is a **dev-dependency only**, never a normal dependency (follows from M5). |
| M5 | 2026-11-17 | Plan D1: turul owns its own AI Catalog model. `ai-catalog-rust` 0.4.0 is used only for interop tests (M4). Accepted cost: a second Rust implementation exists in the ecosystem. Inside turul, this workspace is the single owner. |
| M6 | 2026-11-17 | Plan D3, as written: `turul-ai-catalog` has zero runtime dependencies, with a hand-rolled strict RFC 8259 reader and its own `Json` type that keeps number lexemes and rejects duplicate members. |
| M7 | 2026-11-17 | Plan D2, D4, D9, D11, D14, D15, D16, D17 and D18 are accepted as written in PLAN.md §1. |
| M8 | 2026-11-17 | Plan D19 is done: the workflow treats this file as binding. |
| M9 | 2026-11-17 | Plan D5, D6, D7 and D8 are in scope. Where design is unclear, the spec decides. |
| M10 | 2026-11-17 | JWS signing and verification (D7, D8) come from `turul-jwt-validator`, extended by its own session through the handoff in `handoff-turul-jwt-validator.md`. This workspace never re-implements JWS cryptography. It owns JCS payload construction, `did:web` resolution through the client, key selection from the DID document, and authority and coverage checks. Slices that need the new validator API wait for its release. |
| M12 | 2026-11-17 | Plan D6 checks (JWS protected-header and `did:web` signer-shape checks) live in `turul-ai-catalog-trust` and run through `turul-jwt-validator`. `turul-ai-catalog` never decodes JWS or base64url, so there is one decoder in turul. The core `validate` checks only the Signature object's own members. |
| M13 | 2026-11-17 | A malformed or failing optional signature is a warning-severity diagnostic and never an endorsement. It does not make the document or entry non-conformant, fail Lambda cold start, or change the CLI exit code by itself. Spec basis: #verification-result ("MUST NOT be treated as a verified endorsement. Consumers MAY retain"). |
| M14 | 2026-11-17 | Plan D21: `turul-jwt-validator = { version = "0.3.4", default-features = false, features = ["detached_rust_crypto"] }` in the trust crate only. No backend feature is forwarded to consumers. |
| M15 | 2026-11-17 | Plan D23: M13 covers JWS header, `did:web` signer-shape and verification defects only. A Signature object missing `signer`, `profile`, `issuedAt` or `jws` (#signature-object, CDDL-required) is a `ParseError`, like any other missing required member. |
| M16 | 2026-11-17 | Yes: `FetchPolicy::max_requests` (default 256) caps the number of HTTP requests per `load`, redirects included. Once the budget is spent, every further child is `Skipped(Budget)`, and the load emits one warning-severity `LoadWarning` naming the limit and the number of children skipped. That warning is carried in `ResolvedCatalog::warnings`; the CLI prints it to stderr and the TUI shows it. No logging-framework dependency is added. Owner: ADR-009 (S9). |
| M17 | 2026-11-17 | The catalog is created by tooling, never hand-written. The CLI gains producer subcommands that author a catalog file:<br>• `init`: a new catalog, with an optional `host`.<br>• `add`: an entry with `identifier`, `type`, exactly one of `url` or `data` file, and the optional `displayName`, `description`, `version` and `tags`.<br>• `remove`: an entry, by entry key.<br>Each command builds through the `turul-ai-catalog` model constructors, refuses to write a document that `validate` reports with an error, and writes `to_canonical_vec` bytes. All creation logic lives in the format crate or the client; the CLI only parses arguments (M3). The Lambda's `catalog.json` is produced by these commands. A test re-creates it with the recorded commands and asserts byte equality with the committed bundle. Owners: ADR-011 (CLI producer commands, S12) and ADR-010 (bundle provenance, S10). S10 runs after S12. |
| M18 | 2026-11-17 | Yes: a local development HTTP server. `turul-ai-catalog-serve` is a second binary in `turul-ai-catalog-lambda`, behind the optional `local` feature (`axum`, plus `tokio` `net` and `signal`), so the deployed Lambda never compiles it. It serves `--file PATH` (default: the bundled catalog) on `--addr` (default `127.0.0.1:8080`). The file goes through `ServedCatalog::load`, the cold-start gate, and every request is answered by the same `handle`. It is the only code in the crate that reads a file or binds a port. Owner: ADR-010. |
| M19 | 2026-11-17 | Yes, ADR first: add `turul-ai-catalog-storage` with a `CatalogStore` trait. It stores entry-level records with ETag versions. Backends are in-memory, sqlite, postgres and dynamodb as cargo features, following `turul-mcp-task-storage`. The Lambda switches from the bundled file to the store, so services can register entries at runtime. This supersedes the M2 bundle, and ADR-010 changes to match. Owner: ADR-016, which is written before any code. Whether turul-a2a and turul-mcp services actually register at runtime, or the catalog stays curated and deployed, is still open: docs/plan/TODO.md F1. |
| M21 | 2026-11-17 | Yes, spec compliance: `put_entry` refuses an entry whose identifier already has stored records of the other version kind, unversioned beside versioned or the reverse (#multi-version-entries: "When `version` is absent, `identifier` alone MUST be unique"). Every backend enforces it atomically, so concurrent writers cannot both pass. The parity suite proves it, including under concurrency. Owner: ADR-016. |
| M22 | 2026-11-17 | Publish to crates.io once the M21 fix and the documentation pass are done and every `cargo publish --dry-run` is clean: `turul-ai-catalog`, `-client`, `-storage`, `-tui`, `-trust`, `-lambda` (both now `publish = true`; maintainer: "publish them all") and `-cli`, in dependency order. Then tag the release and switch the README install line to crates.io. |
| M23 | 2026-11-17 | Yes (a downstream consumer's request): `ServedCatalog::from_document(&AiCatalog)` in `turul-ai-catalog-lambda` builds the served catalog from a caller-supplied document through an in-memory store (`import`, then `assemble`). It reads no file and ships no default catalog. The local `serve` binary routes through it, so serving a carried document has one owner. Owner: ADR-010. Ships as 0.0.20. |
| M24 | 2026-11-17 | Yes (a downstream consumer's request): `UrnAir` moves from `turul-ai-catalog-trust` to `turul_ai_catalog::identity`, next to `EntryIdentifier`. The trust crate imports it from core, with no re-export (no compatibility copies). Core `validate()` does not start judging `urn:air`. Owner: ADR-004; ADR-012 and ADR-015 cite it. Ships in 0.0.20. |
| M25 | 2026-11-17 | Yes: publish 0.0.20 of all seven crates to crates.io once the gates pass, then tag `v0.0.20`. |
| M26 | 2026-11-17 | Yes, build F1 now: an opt-in registration API on `CatalogStore` with: a per-writer `urn:air` publisher allow-list, refusing when none is configured; an optional signed-Trust-Manifest requirement; two entry modes, lease with expiry and curated, that refuse writes from the other mode; and assembly that reports expired or omitted entries. Owner: ADR-017. The ADR is written and approved by the maintainer before any code. |
| M27 | 2026-11-17 | Yes: a CLI store command for operators, `turul-ai-catalog store`, at minimum listing a catalog's entries with their provenance and expiry state and removing an entry by entry key, against the configured backend. This is the concrete removal path for stale leases. Owner: ADR-017, with ADR-011 cited for CLI shape. All logic lives in the storage or client crates; the CLI only parses arguments. |
| M28 | 2026-11-17 | Yes, fail forward: publish the release carrying ADR-017 (R1 is breaking for store holders) once the gates and the pre-push review pass, with no compatibility path. Tell downstream consumers the version and the upgrade sequence when it ships. |
| M29 | 2026-11-18 | Yes (a downstream consumer's request): `turul_ai_catalog_client::fetch(url, accept: Option<&str>, policy)` sends a caller-named `Accept` (RFC 9110 §12.5.1). `None` keeps the current request. Safe Fetching is unchanged. Fail forward: the signature changes in place and every caller is updated. Owner: ADR-008. Ships as 0.0.22. |
| M11 | 2026-11-17 | Plan D20: draft the upstream issue texts in `upstream-issues.md`. The maintainer files them; nothing is posted from this workspace. |
| M20 | 2026-11-17 | Decided by spec under the standing instruction "the spec decides, don't prompt me" (2026-11-17); answers ADR-003 AMB-94 and reverses the earlier PLAN S2 choice. The canonical writer is exactly RFC 8785: a finite number is written as JCS §3.2.2.3 prescribes, the ECMAScript form of the IEEE 754 double nearest its lexeme, even when that differs from the lexeme (`9007199254740993` → `9007199254740992`). The writer refuses, as `NotIJson` at the pointer, only what JCS cannot write: a number that is not finite or overflows as a double (`1e400`). Lone surrogates and noncharacters keep their handling. Validation still reports the lossy case: `number-inexact` is a warning; `number-not-i-json` (non-finite or overflow) is an error in a signed or digested context, so the MUST/SHOULD split has its predicate. A typed `Attestation::size` gets no refusal of its own (reasoning in ADR-003 §3). Reason: #signed-payload-and-jws-construction requires JCS, and the payload "MUST satisfy JCS's I-JSON constraints"; RFC 7493 §2.2 makes excess precision a SHOULD NOT, not a MUST; refusal made signatures and digests from conformant producers unverifiable and made the writer a variant of a standard algorithm. |

## M4 interop tests

| Boundary | Test | Runs |
|---|---|---|
| Format ↔ official Rust SDK | Every spec example, every `ai-catalog-rust` fixture and every fixture of ours: our parse agrees with theirs on accept/reject. Our serialise → their parse, and their serialise → our parse, round-trip to equal values. Disagreements are listed in ADR-001 with the spec anchor that decides them. | `cargo test`, offline |
| Lambda ↔ consumers | The handler's response (status, `Content-Type: application/ai-catalog+json`, body) parsed by `turul-ai-catalog-client` and by `ai-catalog-rust`. | `cargo test`, offline (handler invoked in-process) |
| Client ↔ real servers | Fetch through a local test HTTP server: well-known path, nested catalogs, depth limit, Safe Fetching rejections. | `cargo test`, loopback only |
| Live probe | The official `ai-catalog` CLI reads a deployed `turul-ai-catalog-lambda` URL. | Opt-in env var, never in CI by default |

Interop tests are named for the boundary they cover (`tests/sdk_interop.rs`), never after an ADR.

## M3 layering

```
turul-ai-catalog          format: model, parse, validate, serialise, resolution rules (sync, no I/O)
        ▲
turul-ai-catalog-client   consumer: load from file or HTTPS (spec Safe Fetching), well-known lookup,
        ▲                 nested-catalog traversal with the spec depth limit, diagnostics, query/filter
        │                 → one resolved catalog tree that every front end renders
   ┌────┴─────┐
turul-ai-catalog-tui      ratatui view over the client's resolved tree (render + key handling only)
turul-ai-catalog-cli      binary: argument parsing and output formatting; `view` opens the TUI
turul-ai-catalog-lambda   handler: serves a bundled catalog; depends on turul-ai-catalog only
```

Invariants for ADRs and tests:
- Fetching, nested traversal, display-name/description/version resolution and filtering each have one owning symbol in `turul-ai-catalog` or `turul-ai-catalog-client`. The CLI and TUI call those symbols; a dependency-direction test fails if either re-implements them or depends on `reqwest` or `serde_json` directly.
- CLI text/JSON output and TUI rendering are both produced from the same client type.
- The network dependency (HTTPS fetch) lives only in `turul-ai-catalog-client`. Loading a local file needs no network.
- Out of scope unless asked: search across registries, pull/cache, OCI, signing. Those are covered by the official `ai-catalog-cli`.
