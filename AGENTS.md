# Repository Guidelines

## Current State

- Six crates, ADR-002 to ADR-015 in `docs/adr/`: `turul-ai-catalog` (format: `parse`, `validate`, `to_canonical_vec`), `-client` (`fetch`, `load`, `produce`), `-tui` (`render`, `run`), `-cli` (binary `turul-ai-catalog`), `-trust` (`sign_manifest`, `verify_signature`, `verify_resolved`), `-lambda` (`ServedCatalog::load`, `handle`).
- Dependencies point at the core: `client` -> core; `tui` -> core, `client`; `cli` -> core, `client`, `tui`; `trust` and `lambda` -> core, `client`.
- Local server: `cargo run -p turul-ai-catalog-lambda --features local --bin turul-ai-catalog-serve -- --file ai-catalog.json --addr 127.0.0.1:8080`; Lambda: `cargo lambda watch -p turul-ai-catalog-lambda`.
- Deferred features: `docs/plan/TODO.md`. Governance: CI, the `adr-review` agent and the `agent-card-slice` workflow.
- Target: the [AI Catalog specification](https://ai-catalog.io) (Linux Foundation, Agent Card Working Group), `specVersion` `1.0`.

## Source of Truth

In this order:

1. The AI Catalog specification: `specification/ai-catalog.md` in `github.com/Agent-Card/ai-catalog`, published at https://ai-catalog.io.
2. The spec's own guides and mappings on https://ai-catalog.io (non-normative; they explain, they do not override).
3. Working Group ADRs in `github.com/Agent-Card/ai-catalog/adr` (intent behind spec text).
4. `ai-catalog-rust` at the version named in ADR-001, and its `fixtures/`.
5. Accepted and Proposed ADRs under `docs/adr/`.
6. This file, then `CLAUDE.md`.
7. Patterns from turul-agent-skills, turul-a2a and turul-mcp-framework.

When in doubt, the reference sources (1–4) resolve it, never local preference or memory. Fetch and cite them with section anchors.

If spec prose and a reference implementation disagree, neither wins silently: the governing ADR records the decision and a test pins it.

## Default Role In This Repo

- Act as critic and architecture reviewer first: spec compliance, single ownership of the format, boundary discipline, test quality.
- Do not make implementation changes unless explicitly asked.
- For review requests, lead with concrete findings.

## Response Style

- Answer the direct question first. Shortest answer that keeps the key caveat or next action.
- Separate confirmed facts from assumptions; say what was verified and when.
- For go/no-go, state the decision, then only blocking issues.

## Non-Negotiable Format Rules

### One Owner

- This workspace is the only turul implementation of AI Catalog parsing, validation, serialisation and trust-manifest verification. A parallel implementation elsewhere is drift; it is reported and migrated.
- Each identity key (entry `identifier`, host `identifier`, media type) is built by one typed constructor. No caller compares or validates them from raw strings.
- The catalog does not model artifact schemas. A2A agent cards, MCP server cards and skills stay opaque here and typed in their owning workspaces.

### Spec Truthfulness

- Every MUST/SHOULD (and the other BCP 14 requirement keywords) in the spec maps to an ADR row and a test, or to an ADR row recording why it is out of scope. All three Conformance Levels are conformant; the level this workspace declares is an ADR decision.
- Unrecognised members, extensions, version handling and nesting depth follow the spec sections that define them, decided in ADRs, not ad hoc.
- Serialisation is deterministic; parse ∘ serialise round-trips.

### Version Hygiene

- Reference sources by version, never by commit SHA or file hash: spec read date and `ai-catalog-rust` version in ADR-001.
- `Cargo.lock` is git-ignored and gates nothing.
- Patch bumps by default. Every behaviour or contract change bumps the version and updates `CHANGELOG.md` in the same commit.

## Architecture Guardrails

- The core library has no network or async dependency. Fetching, discovery, signing and signature verification (the spec's Level 2 and Level 3 consumer behaviour, anything needing network or crypto) need a maintainer yes before they exist.
- No A2A, MCP or skills type in a public API of this workspace.

## Testing Policy

- Tests come from the spec and the ADR, before implementation.
- Each ADR names its contract test file(s) in its `Implements:` line; those tests are the executable contract.
- Spec examples (`specification/examples/*.json` and inline examples) are fixtures; every one parses and round-trips.

## Planning Rules

- ADR before implementation for any contract. An ADR stays Proposed until every alternative path is deleted.
- No new feature, crate or infrastructure without the maintainer's explicit yes. State the one-sentence ask and what breaks without it.
- Delete superseded paths in the same slice, callers and tests included. No compatibility shims.

## Workspace Boundary

- Never edit a sibling repo from here. Report gaps as a handoff naming the target session: owner, goal, gates, stop conditions.
- Commits carry no AI attribution.
