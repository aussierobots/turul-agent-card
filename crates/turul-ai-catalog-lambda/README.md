# turul-ai-catalog-lambda

An AWS Lambda handler that serves an [AI Catalog](https://ai-catalog.io) at the well-known URI
`/.well-known/ai-catalog.json`. The catalog is assembled from a
[`turul-ai-catalog-storage`](https://docs.rs/turul-ai-catalog-storage) store, validated, and
served as canonical `application/ai-catalog+json` bytes.

Part of [turul-agent-card](https://github.com/aussierobots/turul-agent-card).

- `GET` and `HEAD` on the well-known path answer 200 with the catalog media type; other methods
  answer 405 with `Allow: GET, HEAD`; any other path answers 404.
- Cold start fails if the catalog cannot be assembled or has an error diagnostic. A later refresh
  that fails keeps serving the last good snapshot.
- The deployed function binary reads `TURUL_AI_CATALOG_ID` (required with `dynamodb`, otherwise
  default `default`), `TURUL_AI_CATALOG_TABLE` (with `dynamodb`) and
  `TURUL_AI_CATALOG_MAX_AGE_SECS` (default 60).

## Feature flags

| Feature    | Enables |
|------------|---------|
| `dynamodb` | The `turul-ai-catalog-lambda` binary reads the catalog from a DynamoDB table, and `table_name` is available. |
| `local`    | The `turul-ai-catalog-serve` binary: the same handler on a plain HTTP port for local development. |

Without `dynamodb` the function binary serves an empty in-memory store.

## Example

```rust
use lambda_http::{Body, http::Request};
use turul_ai_catalog_lambda::{ServedCatalog, handle};
use turul_ai_catalog_storage::{CatalogId, InMemoryCatalogStore};

# futures::executor::block_on(async {
let store = InMemoryCatalogStore::new();
let catalog = CatalogId::new("default")?;
let served = ServedCatalog::assemble(&store, &catalog).await?;

let request = Request::builder()
    .uri("/.well-known/ai-catalog.json")
    .body(Body::Empty)?;
assert_eq!(handle(&served, &request).status(), 200);
# Ok::<(), Box<dyn std::error::Error>>(())
# })?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## License

Apache-2.0.
