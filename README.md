[Project website](https://aussierobots.com.au/turul-agent-card.html) · [Turul family](https://aussierobots.com.au/turul.html)

# Turul Agent Card

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/assets/turul/logo-dark.png">
  <img src="docs/assets/turul/logo-light.png" alt="Turul AGENT CARD" width="600">
</picture>

Rust tooling for [AI Catalog](https://ai-catalog.io), the Linux Foundation
[Agent Card](https://github.com/Agent-Card) working group's format for discovering AI artifacts.
An AI Catalog (`application/ai-catalog+json`) is one JSON document that lists what a host offers:
A2A agents, MCP servers, agent skills and plugins, and other catalogs, each declared by its type
and referenced by URL or embedded inline. Optional Trust Manifests add publisher identity,
attestations and signatures.

Turul Agent Card reads, validates, creates, fetches, serves and signs catalogs. It is the one
place the turul projects (such as [turul-a2a](https://github.com/aussierobots/turul-a2a) and
[turul-mcp-framework](https://github.com/aussierobots/turul-mcp-framework)) read and write
AI Catalog documents.

```json
{
  "specVersion": "1.0",
  "host": { "displayName": "Example Corp", "identifier": "did:web:example.com" },
  "entries": [
    {
      "identifier": "urn:air:example.com:a2a:research",
      "type": "application/a2a-agent-card+json",
      "url": "https://agents.example.com/.well-known/agent-card.json"
    },
    {
      "identifier": "urn:air:example.com:mcp:weather",
      "type": "application/mcp-server-card+json",
      "url": "https://mcp.example.com/.well-known/mcp/server-card.json"
    }
  ]
}
```

The catalog does not redefine the artifacts it lists: an A2A Agent Card or an MCP Server Card
keeps its own schema, and the catalog adds discovery, publisher and trust information around it.

## Crates

| Crate | What it is |
|---|---|
| [`turul-ai-catalog`](crates/turul-ai-catalog) | Library: the document model, strict parsing, validation with conformance levels, entry name/description/version resolution, RFC 8785 canonical JSON (a number RFC 8785 rounds to another value, such as `9007199254740993`, is written in its RFC 8785 form and reported as a `number-inexact` warning). No runtime dependencies. |
| [`turul-ai-catalog-client`](crates/turul-ai-catalog-client) | Library: create catalog files, load catalogs from a file, a URL or a domain's well-known URI under the spec's Safe Fetching rules, and follow nested catalogs into one tree. |
| [`turul-ai-catalog-cli`](crates/turul-ai-catalog-cli) | The `turul-ai-catalog` binary: `init`, `add`, `remove`, `validate`, `list`, `view`. |
| [`turul-ai-catalog-tui`](crates/turul-ai-catalog-tui) | The terminal catalog browser behind `turul-ai-catalog view` ([ratatui](https://ratatui.rs)). |
| [`turul-ai-catalog-trust`](crates/turul-ai-catalog-trust) | Trust Manifests: signed payloads, artifact digests, ES256 signing and `did:web` verification, through [turul-jwt-validator](https://github.com/aussierobots/turul-jwt-validator). |
| [`turul-ai-catalog-storage`](crates/turul-ai-catalog-storage) | Library: entry-level catalog storage with content-derived ETags and conditional writes, assembly of the served document, and `import` of a catalog file. Backends `in-memory` (default), `sqlite`, `postgres` and `dynamodb` as features. |
| [`turul-ai-catalog-lambda`](crates/turul-ai-catalog-lambda) | An AWS Lambda handler that serves the catalog assembled from a `turul-ai-catalog-storage` store at `/.well-known/ai-catalog.json`. |

## Quick start

### Command line

```sh
cargo install turul-ai-catalog-cli

turul-ai-catalog init --file ai-catalog.json \
  --host-display-name "Example Corp" --host-identifier did:web:example.com
turul-ai-catalog add --file ai-catalog.json \
  --identifier urn:air:example.com:mcp:weather \
  --type application/mcp-server-card+json \
  --url https://mcp.example.com/.well-known/mcp/server-card.json
turul-ai-catalog validate --file ai-catalog.json   # exit 0 valid, 1 invalid
turul-ai-catalog list --file ai-catalog.json
turul-ai-catalog view --well-known example.com     # browse a live catalog in the terminal
```

`add` and `remove` validate before writing and refuse a change that would make the catalog
invalid, such as a duplicate entry or both `url` and `data`.

### Library

```sh
cargo add turul-ai-catalog
```

Build a catalog, check it against the specification, and write canonical JSON:

```rust
use turul_ai_catalog::canonical::to_canonical_vec;
use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier, HostIdentifier};
use turul_ai_catalog::model::{AiCatalog, ArtifactSource, CatalogEntry, HostInfo};
use turul_ai_catalog::validate::{Conformance, ConformanceLevel, validate};

let agent = CatalogEntry::new(
    EntryIdentifier::new("urn:air:example.com:a2a:research"),
    ArtifactType::A2A_AGENT_CARD,
    ArtifactSource::Url("https://agents.example.com/.well-known/agent-card.json".into()),
);
let server = CatalogEntry::new(
    EntryIdentifier::new("urn:air:example.com:mcp:weather"),
    ArtifactType::MCP_SERVER_CARD,
    ArtifactSource::Url("https://mcp.example.com/.well-known/mcp/server-card.json".into()),
);

let mut catalog = AiCatalog::new(vec![agent, server]);
let mut host = HostInfo::new("Example Corp".into());
host.identifier = Some(HostIdentifier::new("did:web:example.com"));
catalog.host = Some(host);

let report = validate(&catalog);
assert!(!report.has_errors());
assert_eq!(report.conformance(), Conformance::Met(ConformanceLevel::Discoverable));

// RFC 8785 canonical JSON: the same catalog always gives the same bytes.
// Numbers take their RFC 8785 form; only a number that is not a finite double (1e400) is refused.
let bytes = to_canonical_vec(&catalog).expect("every value is I-JSON");
assert!(bytes.starts_with(br#"{"entries":"#));
```

Parsing, loading from HTTPS and creating catalog files are covered in the
[library guide](crates/turul-ai-catalog/README.md) and the
[client guide](crates/turul-ai-catalog-client/README.md), where every snippet is compiled and run
as a test.

## Serving a catalog

`turul-ai-catalog-lambda` serves the catalog assembled from a `turul-ai-catalog-storage` store at
`/.well-known/ai-catalog.json` with `Content-Type: application/ai-catalog+json`. The assembled
document is validated at cold start, and the function fails to start if it has errors. A warm
instance re-assembles when its snapshot is older than `TURUL_AI_CATALOG_MAX_AGE_SECS` (default 60)
and keeps the last good snapshot if a refresh fails.

Built with the `dynamodb` feature, the function reads the table named by `TURUL_AI_CATALOG_TABLE`
and the catalog named by `TURUL_AI_CATALOG_ID`, and fails to start if either is missing. Built
without it, it serves an empty in-memory store. Load a curated catalog file (made with the CLI
`init`, `add` and `remove`) into a store with `turul_ai_catalog_storage::import`; services that
register themselves write through `EntryRecord::new` and `CatalogStore::put_entry`. The deploying
project owns the table, the function and every grant (IAM, SAM, CDK): the serving role needs
`dynamodb:GetItem` and `dynamodb:Query`. An importer (`import`, `put_host`, `put_entry`) needs
`dynamodb:PutItem` and `dynamodb:UpdateItem`; a writer that also removes entries (`delete_entry`)
adds `dynamodb:DeleteItem`, `dynamodb:ConditionCheckItem` and `dynamodb:GetItem`. Entry writes are
`TransactWriteItems` calls, which IAM authorises by these per-item actions.

To serve a catalog file on a local HTTP port while developing, run the same handler as a plain
server:

```sh
cargo run -p turul-ai-catalog-lambda --features local --bin turul-ai-catalog-serve -- \
  --file ai-catalog.json --addr 127.0.0.1:8080
curl -i http://127.0.0.1:8080/.well-known/ai-catalog.json
```

The file is imported into an in-memory store and assembled exactly as at Lambda cold start, and
the responses are identical; without `--file` the server serves an empty catalog. To run the Lambda
itself locally instead, use [cargo-lambda](https://www.cargo-lambda.info):
`cargo lambda watch -p turul-ai-catalog-lambda`, which serves an empty in-memory catalog. AI Catalog
clients, this one included, refuse plain `http://` and loopback addresses under the
specification's Safe Fetching rules, so check a local server with `curl` or a browser.

## Specification conformance

Turul Agent Card follows the [AI Catalog specification](https://ai-catalog.io) (`specVersion`
`1.0`) and reports Level 1 (Minimal) and Level 2 (Discoverable) conformance. It is checked against
the working group's own implementations:

- The format and trust test suites compare results with
  [`ai-catalog-rust`](https://github.com/Agent-Card/ai-catalog-rust) 0.4.0 on every spec example
  and fixture, in both directions.
- Catalogs written by this CLI validate with the official
  [`ai-catalog`](https://github.com/Agent-Card/ai-catalog-cli) CLI 0.4.0.
- A deployed Lambda can be probed with both clients:
  `TURUL_AI_CATALOG_LIVE_URL=... AI_CATALOG_CLI=... cargo test -p turul-ai-catalog-lambda --test live_probe -- --ignored`.

Where the specification and `ai-catalog-rust` disagree, this project follows the specification;
each case is recorded in [ADR-001](docs/adr/ADR-001-sources-corpus-and-decisions.md) and pinned by
a test.

## Project documentation

- [Architecture decision records](docs/adr): one per contract, each naming the tests that prove it.
- [CHANGELOG](CHANGELOG.md)
- [Contributor guidelines](AGENTS.md)

## Building

```sh
cargo build --workspace
cargo test --workspace      # no network needed
```

Rust 1.95 or later.

## Brand artwork

Turul brand artwork is © 2026 Toast Technology Pty Ltd, all rights reserved,
and is excluded from the software licence. See the [artwork notice](docs/assets/turul/ARTWORK-NOTICE.md) and [master assets](https://github.com/aussierobots/turul-brand).

[Project icons and artwork](docs/assets/turul/README.md).

## Licence

Apache-2.0. See [`LICENSE`](LICENSE).
