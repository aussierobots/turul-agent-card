# turul-ai-catalog-cli

The `turul-ai-catalog` command: validate, list and view an [AI Catalog](https://ai-catalog.io) and its nested catalogs, and create or edit a catalog file with `init`, `add` and `remove`.

Part of [turul-agent-card](https://github.com/aussierobots/turul-agent-card), a Rust implementation of the AI Catalog specification. The command is a thin layer over `turul-ai-catalog-client` (loading and producing) and `turul-ai-catalog-tui` (the `view` browser); it has no library API.

## Install

```sh
cargo install turul-ai-catalog-cli
```

## Usage

Reading commands take exactly one source: `--file PATH`, `--url URL` or `--well-known HOST`. Loading always uses the default safe-fetching policy.

```sh
turul-ai-catalog validate [--format text|json] (--file PATH | --url URL | --well-known HOST)
turul-ai-catalog list     [--format text|json] [--identifier ID] [--version V] [--artifact-type T] [--tag TAG] SOURCE
turul-ai-catalog view     SOURCE
```

Producer commands write the catalog file named by `--file`. Nothing is written when the resulting document has an error diagnostic.

```sh
turul-ai-catalog init   --file catalog.json [--host-display-name TEXT [--host-identifier TEXT] [--host-documentation-url URL] [--host-logo-url URL]]
turul-ai-catalog add    --file catalog.json --identifier ID --type TYPE (--url URL | --data PATH) [--display-name TEXT] [--description TEXT] [--version V] [--tag TAG]...
turul-ai-catalog remove --file catalog.json --identifier ID [--version V]
```

`validate` and `list` write their output, diagnostics included, to stdout; `--format json` writes the canonical JSON of the loaded tree (filtered, for `list`). Load warnings go to stderr. Producer commands print their diagnostics to stderr.

## Operating a store

```sh
turul-ai-catalog store list   BACKEND --catalog-id ID
turul-ai-catalog store remove BACKEND --catalog-id ID --identifier ID [--version V]
```

`BACKEND` is one of `--sqlite PATH`, `--postgres URL` (feature `postgres`) or `--table NAME` (DynamoDB, feature `dynamodb`). Each reads an environment variable when its flag is absent: `TURUL_AI_CATALOG_SQLITE`, `TURUL_AI_CATALOG_POSTGRES_URL` and `TURUL_AI_CATALOG_TABLE`, and `--catalog-id` reads `TURUL_AI_CATALOG_ID`, as the Lambda does. DynamoDB credentials come from the AWS default chain. `list` prints one line per entry: identifier, version, mode, writer and whether it is live, expired or never expires. `remove` deletes the entry with exactly that identifier and version (no `--version` names the unversioned entry) and refuses when the entry changed since it was read. Both exit 0 on success and 2 on any failure, including `remove` matching no entry.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success; the loaded tree has no error diagnostic |
| 1 | the loaded tree, or the document a producer command would write, has an error diagnostic |
| 2 | usage error, load failure, any other producer refusal (such as `remove` matching no entry), output failure, or `view` without a terminal |

## Licence

Apache-2.0.
