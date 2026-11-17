# turul-ai-catalog-client

Create AI Catalog files, and load catalogs from files or HTTPS (under the specification's Safe
Fetching rules), following nested catalogs into one resolved tree. Built on
[`turul-ai-catalog`](../turul-ai-catalog).

Part of [turul-agent-card](https://github.com/aussierobots/turul-agent-card).

## Quick start

```sh
cargo add turul-ai-catalog-client
```

Create a catalog file and add an MCP server to it. Every change is validated, and only a
document without errors is written, as canonical JSON:

```rust
use turul_ai_catalog_client::{Change, Content, EntryInput, HostInput, produce};

let dir = std::env::temp_dir().join(format!("ai-catalog-readme-{}", std::process::id()));
std::fs::create_dir_all(&dir)?;
let file = dir.join("ai-catalog.json");
# let _ = std::fs::remove_file(&file);

let mut host = HostInput::new("Example Corp".into());
host.identifier = Some("did:web:example.com".into());
produce(&file, &Change::Init { host: Some(host) })?;

let mut server = EntryInput::new(
    "urn:air:example.com:mcp:weather".into(),
    "application/mcp-server-card+json".into(),
    Content::Url("https://mcp.example.com/.well-known/mcp/server-card.json".into()),
);
server.display_name = Some("Weather".into());
produce(&file, &Change::Add(server))?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Load a catalog and list its entries. `Source::File` reads locally; `Source::Url` and
`Source::WellKnown` fetch over HTTPS with private addresses, redirects, size, time and request
count all bounded by `FetchPolicy`:

```rust
use turul_ai_catalog_client::{FetchPolicy, Source, load};

# let dir = std::env::temp_dir().join(format!("ai-catalog-readme-load-{}", std::process::id()));
# std::fs::create_dir_all(&dir)?;
# let file = dir.join("ai-catalog.json");
# std::fs::write(&file, br#"{"specVersion":"1.0","entries":[{"identifier":"urn:air:example.com:mcp:weather","type":"application/mcp-server-card+json","url":"https://mcp.example.com/.well-known/mcp/server-card.json"}]}"#)?;
let tree = load(&Source::File(file), &FetchPolicy::default())?;
for row in &tree.rows {
    println!("{}", row.name);
}
for warning in &tree.warnings {
    eprintln!("warning: {warning:?}");
}
assert_eq!(tree.rows[0].name, "weather");
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Licence

Apache-2.0.
