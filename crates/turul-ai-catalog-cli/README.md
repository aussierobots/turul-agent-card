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

`validate` and `list` write their output, diagnostics included, to stdout; `list` prints one line per entry: name, identifier, type, version and url (`-` for an embedded entry); `--format json` writes the canonical JSON of the loaded tree (filtered, for `list`). Load warnings go to stderr. Producer commands print their diagnostics to stderr.

## Operating a store

```sh
turul-ai-catalog store list   BACKEND --catalog-id ID
turul-ai-catalog store remove BACKEND --catalog-id ID --identifier ID [--version V]
turul-ai-catalog store import BACKEND --catalog-id ID --file PATH --writer WRITER [--replace]
turul-ai-catalog store purge  BACKEND --catalog-id ID [--yes]
```

`BACKEND` is one of `--sqlite PATH`, `--postgres URL` (feature `postgres`) or `--table NAME` (DynamoDB, feature `dynamodb`). Each reads an environment variable when its flag is absent: `TURUL_AI_CATALOG_SQLITE`, `TURUL_AI_CATALOG_POSTGRES_URL` and `TURUL_AI_CATALOG_TABLE`, and `--catalog-id` reads `TURUL_AI_CATALOG_ID`, as the Lambda does. DynamoDB credentials come from the AWS default chain. `list` prints one line per entry: identifier, version, mode, writer and whether it is live, expired or never expires. `remove` deletes the entry with exactly that identifier and version (no `--version` names the unversioned entry) and refuses when the entry changed since it was read. `import` stores the entries of a catalog file as curated by the permanent writer id `--writer`, following no nested catalog; with `--replace` the catalog id then holds exactly the file (a file without a host keeps the stored host), and it refuses, writing nothing, while the id holds a live lease. `purge` only counts the id's items by kind unless given `--yes`, and refuses the id equal to `TURUL_AI_CATALOG_ID`, so the id to purge must be given as `--catalog-id`; `--yes` also refuses while `TURUL_AI_CATALOG_ID` is unset, so set it to the id the function serves. Each exits 0 on success, 1 when `import` refuses a file with an error diagnostic, and 2 on any other failure, including `remove` matching no entry.

## Publisher signing

```text
turul-ai-catalog key generate     --ssm-parameter NAME --did DID --fragment F
turul-ai-catalog key did-document --ssm-parameter NAME --did DID --fragment F
turul-ai-catalog sign SOURCE --ssm-parameter NAME --did DID --fragment F --issued-at T [--expires-at T] [--catalog]
turul-ai-catalog verify SOURCE [--require-publisher]
```

`key` and `sign` need the feature `ssm`; `verify` is always built. The P-256 key is held as a `SecureString` in AWS Systems Manager Parameter Store (credentials from the AWS default chain) and is never printed or written to a file. `key generate` creates the parameter, refuses (exit 2) when it exists, and prints the DID document to serve at `https://<domain>/.well-known/did.json`; `key did-document` prints it again. `sign` sets each authorised entry's `digest` from its artifact and signs its publisher Trust Manifest, keeping a manifest whose release is unchanged, and prints the catalog; with `--catalog` it signs the whole catalog as it is. It follows no nested catalog and exits 0 when written, 1 when the loaded document has an error diagnostic, 2 on any other failure. `verify` prints one line per catalog and entry and exits 1 when a signature is rejected, a relied-on artifact fails its digest, a nested catalog was not loaded, a catalog has an error diagnostic, or, with `--require-publisher`, an entry has no endorsement by its publisher. Every artifact release changes that entry's digest, so run `sign` again before publishing the catalog. `sign --file` has no base URL, so relative `url` entries need `--url` or `--well-known`. `sign` removes a whole-catalog signature when it changes an entry; run `sign --catalog` afterwards. If `key generate` reports that the parameter exists after a network retry, `key did-document` prints the document of the key it holds.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success; the loaded tree has no error diagnostic |
| 1 | the loaded tree, or the document a producer command would write, has an error diagnostic |
| 2 | usage error, load failure, any other producer refusal (such as `remove` matching no entry), output failure, or `view` without a terminal |

## Licence

Apache-2.0.
