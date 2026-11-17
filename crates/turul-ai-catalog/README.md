# turul-ai-catalog

Parse, validate, build and canonically serialise [AI Catalog](https://ai-catalog.io) documents
(`application/ai-catalog+json`). No async runtime, network or other runtime dependency.

Part of [turul-agent-card](https://github.com/aussierobots/turul-agent-card).

## Quick start

```sh
cargo add turul-ai-catalog
```

Build a catalog that lists an A2A agent and an MCP server, check it against the specification,
and write it out:

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

Read a catalog and report what is wrong with it. Parsing is strict (duplicate member names,
missing required members and unsupported major versions are errors); validation reports every
remaining finding with its JSON Pointer:

```rust
use turul_ai_catalog::parse::parse;
use turul_ai_catalog::validate::validate;

let input = br#"{"specVersion":"1.0","entries":[
  {"identifier":"urn:air:example.com:mcp:weather","type":"application/mcp-server-card+json",
   "url":"https://mcp.example.com/.well-known/mcp/server-card.json"}]}"#;

let catalog = parse(input).expect("a well-formed catalog");
for d in validate(&catalog).diagnostics() {
    println!("{:?} {} at {}", d.severity, d.code, d.path);
}

assert!(parse(br#"{"specVersion":"1.0"}"#).is_err()); // `entries` is required
```

A catalog does not parse or model the artifacts it lists: an A2A Agent Card or MCP Server Card is
referenced by `url`, or embedded as opaque JSON in `data`, and declared by its `type`.

## Feature flags

None. The crate has no dependencies and no optional features.

## Licence

Apache-2.0.
