# turul-ai-catalog-storage

Storage for [AI Catalog](https://ai-catalog.io) entries. A [`CatalogStore`] keeps entry-level
records and the Host Info record of each catalog, every record versioned by a content-derived
[`Etag`] and written under an optimistic [`Condition`]. [`assemble`] builds the served catalog
document from the stored records, and [`import`] loads a catalog document into them.

Part of [turul-agent-card](https://github.com/aussierobots/turul-agent-card).

## Feature flags

| Feature    | Enables                                   | Default |
|------------|-------------------------------------------|---------|
| `in-memory`| `InMemoryCatalogStore`                | yes     |
| `sqlite`   | `SqliteCatalogStore` (SQLite via sqlx)  | no      |
| `postgres` | `PostgresCatalogStore` (PostgreSQL via sqlx) | no |
| `dynamodb` | `DynamoDbCatalogStore` (AWS DynamoDB)   | no      |

Every backend guarantees atomic conditional writes, read-your-writes, byte-identical reads, and
that no identifier holds both unversioned and versioned entries.

## Quick start

```sh
cargo add turul-ai-catalog-storage
```

Store an entry, then assemble the catalog document it belongs to:

```rust
use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
use turul_ai_catalog::model::{ArtifactSource, CatalogEntry};
use turul_ai_catalog_storage::{
    CatalogId, CatalogStore, Condition, EntryRecord, InMemoryCatalogStore, assemble,
};

# futures::executor::block_on(async {
let store = InMemoryCatalogStore::new();
let catalog = CatalogId::new("main")?;

let entry = CatalogEntry::new(
    EntryIdentifier::new("urn:air:example.com:a2a:research"),
    ArtifactType::A2A_AGENT_CARD,
    ArtifactSource::Url("https://agents.example.com/.well-known/agent-card.json".into()),
);
let record = EntryRecord::new(entry)?;
store.put_entry(&catalog, &record, Condition::Absent).await?;

let served = assemble(&store, &catalog).await?;
assert!(!served.bytes.is_empty());
# Ok::<(), Box<dyn std::error::Error>>(())
# })?;
# Ok::<(), Box<dyn std::error::Error>>(())
```
