# turul-ai-catalog-tui

Terminal view over a resolved [AI Catalog](https://ai-catalog.io) tree, built on `ratatui`.

The crate renders a `ResolvedCatalog` (from `turul-ai-catalog-client`) as a scrollable tree of
entries with a detail pane for the selected one. Every string that comes from a document, a
report or a URL is drawn as `DisplayText`, and an entry's `data` is shown only as the text of its
canonical JSON, whatever the entry `type` is.

The crate has no feature flags.

## Example

```no_run
use turul_ai_catalog_client::ResolvedCatalog;

fn show(catalog: ResolvedCatalog) -> Result<(), turul_ai_catalog_tui::RunError> {
    // Fails with `RunError::NotATerminal` unless stdin and stdout are both terminals.
    turul_ai_catalog_tui::run(catalog)
}
```

Part of [turul-agent-card](https://github.com/aussierobots/turul-agent-card), a Rust implementation of the [AI Catalog](https://ai-catalog.io) specification.

## Licence

Apache-2.0.
