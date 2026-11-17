# CLAUDE.md

1. Don’t assume. Don’t hide confusion. Surface tradeoffs.
2. Minimum code that solves the problem. Nothing speculative.
3. Touch only what you must. Clean up only your own mess.
4. Define success criteria. Loop until verified.

## Project Overview

Turul-agent-card is a Rust implementation of the [AI Catalog](https://ai-catalog.io) specification (Linux Foundation, Agent Card Working Group, `github.com/Agent-Card`): the `application/ai-catalog+json` catalog, its entries, Host Info, Publisher and the optional Trust Manifest. Licensed under Apache-2.0.

**This workspace owns the AI Catalog format for the turul workspaces.** turul-a2a, turul-mcp-framework and any other turul workspace that reads or writes an AI Catalog consumes this crate; none re-implements it. The catalog is agnostic about the artifacts it indexes: A2A agent cards belong to turul-a2a, MCP server cards to turul-mcp-framework, SKILL.md to turul-agent-skills.

**Current release**: see `CHANGELOG.md`. Maintainer decisions: ADR-001. CI (`.github/workflows/ci.yml`) runs fmt, clippy `-D warnings`, test and `cargo doc` with `RUSTDOCFLAGS=-D warnings` on stable, plus `cargo check` on the MSRV.

## Build & Development Commands

```bash
cargo build --workspace
cargo test --workspace                     # No network required
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo doc --no-deps --workspace
```

**All crate dependencies MUST use `workspace = true`**: versions live in root `[workspace.dependencies]`, dev-dependencies included. Never put a version number in a crate's own `Cargo.toml`.

**Versions, not SHAs.** Upstream sources (spec, `ai-catalog-rust`) and sibling crates are referenced by version, never by commit SHA or file hash. Sibling crates are major.minor only. `Cargo.lock` is git-ignored and gates nothing.

**Parallel worktrees:** give each worktree its own `CARGO_TARGET_DIR`.

## Git Conventions

- Commit messages: succinct, no Co-Authored-By or any AI attribution.
- Do not push unless explicitly asked.
- Patch bumps by default. Every behaviour or contract change bumps the version and updates `CHANGELOG.md` in the same commit.

## CHANGELOG.md style

For adopters and maintainers: what changed for someone consuming these crates. Use `### Added` / `### Changed` / `### Fixed` / `### Removed` / `### Compatibility`, plus a one-line `### Verification` when behaviour-sensitive. No task/phase numbers, commit hashes, ADR section numbers or review-loop narration. `see ADR-00N` is fine.

## Release & Publish (crates.io)

Publishing is irreversible. Never run `cargo publish` without explicit instruction; authorize each call separately.

## Architecture

Crate structure, owning symbols and dependency direction are decided in ADRs under `docs/adr/`. Seven crates:

- `turul-ai-catalog`: the format and nothing else; no network or async. Owns `parse`, `validate`/`validate_at`, `to_canonical_vec`, `Timestamp`, `DigestValue`, the `resolve` display functions (ADR-002 to ADR-007).
- `turul-ai-catalog-client`: consumer logic over the core. Owns `fetch`, `load`, `ResolvedCatalog`, `Filter` and `produce` (ADR-008, ADR-009, ADR-011).
- `turul-ai-catalog-tui`: ratatui view over `ResolvedCatalog`. Owns `render`, `run` (ADR-011).
- `turul-ai-catalog-cli`: binary `turul-ai-catalog`: `validate`, `list`, `view`, `init`, `add`, `remove`. Thin; no catalog logic (ADR-011).
- `turul-ai-catalog-trust`: signing and verification as a library (`turul-jwt-validator`). Owns `sign_manifest`, `sign_catalog`, `verify_signature`, `verify_resolved` (ADR-012 to ADR-015).
- `turul-ai-catalog-storage`: entry-level catalog storage. Owns `CatalogStore`, `EntryRecord`/`HostRecord` (the record gate), `CatalogId`, `RecordKey`, `Etag`, `assemble` and `import`; backends `in-memory` (default), `sqlite`, `postgres`, `dynamodb` as features (ADR-016).
- `turul-ai-catalog-lambda`: serves the catalog assembled from a store at `/.well-known/ai-catalog.json`. Owns `ServedCatalog::assemble`, `SnapshotCache` and `handle` (ADR-010, ADR-016).

Dependency direction: `client` -> core; `tui` -> core, `client`; `cli` -> core, `client`, `tui`; `trust` -> core, `client`; `storage` -> core; `lambda` -> core, `storage`. Nothing depends on `cli`; `storage` and `lambda` never depend on `client`, `cli` or `trust`.

Local server: `cargo run -p turul-ai-catalog-lambda --features local --bin turul-ai-catalog-serve -- --file ai-catalog.json --addr 127.0.0.1:8080`. Lambda locally: `cargo lambda watch -p turul-ai-catalog-lambda`. Features agreed but not started: `docs/plan/TODO.md`.

### Normative Sources

- Specification: https://ai-catalog.io (sources: `specification/ai-catalog.md` in `github.com/Agent-Card/ai-catalog`; published at https://agent-card.github.io/ai-catalog/).
- Working Group ADRs: `adr/` in `github.com/Agent-Card/ai-catalog`. They explain spec intent; the spec text wins.
- Official Rust implementation `ai-catalog-rust` (`github.com/Agent-Card/ai-catalog-rust`), referenced by version in ADR-001. Its `fixtures/` are the candidate conformance corpus.

Where spec prose and a reference implementation differ, the governing ADR records the decision and a test pins it. Re-read the live spec before describing a release as spec-compliant.

### Architecture Decision Records

Long-form rationale lives under `docs/adr/`. Each ADR cites the spec section(s) it implements, names the owning symbol and entrypoint for every key and derived artifact, and stays Proposed until every alternative path is deleted. Each ADR names its contract test file(s) in its `Implements:` line. Test files, modules and functions are named for the behaviour they test, never after an ADR. A contract change updates the ADR revision log, its tests and `CHANGELOG.md` in the same slice.

ADR refs belong in commit messages, PR descriptions, `CHANGELOG.md`, ADR cross-references and `README.md` files. **Not in code comments, docstrings or rustdoc.**

### Agent workflow

`.claude/workflows/agent-card-slice.js` plans and implements slices with costed agents (haiku fetch and commits; sonnet research, tests, implementation and code-vs-ADR check; opus plan, ADRs, tests-vs-ADR check and devil's advocate; fable review). It has three modes:

- `mode: "plan"` writes `docs/plan/PLAN.md`, which an opus devil's advocate and fable review, then stops.
- `mode: "implement"` runs only a plan that carries the maintainer's `Approved: <date>` line, against the upstream versions that plan records. Per slice it runs: the ADR (opus, only if its file does not exist), then one sonnet agent that writes the tests from the ADR, implements and runs the gates until green (opus adjudicates test disputes), then marks the slice done. After the last slice: one version bump and CHANGELOG entry. It stops on a maintainer ask, an unresolved test dispute, or failed gates.
- `mode: "review"` is the one full review before push: adr-review, an opus devil's advocate and fable, with opus fixes.

See the script header.

### ADR-review agent: mandatory precommit gate

Before committing any change touching `crates/`, `docs/adr/`, `CLAUDE.md`, `AGENTS.md`, `CHANGELOG.md` or the root `Cargo.toml`, dispatch the `adr-review` subagent (`.claude/agents/adr-review.md`). BLOCKERs and HIGHs are fixed before the commit lands.

### Naming, Comment and Docstring Style

**Names say what a thing is.** No file, module, test, function, type or fixture is named after an ADR, task or phase. The ADR points at the code; the code never points at the ADR.

Comments state the invariant, the contract, or the timeless "why". Never ADR numbers or sections, phase/slice/task labels, issue numbers or review history. Upstream spec citations (URL + section) are fine when the comment would be incomplete without them.

### TDD Discipline

Tests are written from the spec and the ADR first; implementation follows. If code disagrees with a test, re-check the spec before changing either. Only change a test when it is wrong relative to the spec.

### Workspace Boundary

This workspace never edits turul-a2a, turul-mcp-framework, turul-agent-skills or any sibling repo. Gaps found there go in a handoff naming the target session (owner, goal, gates, stop conditions).
