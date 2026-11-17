# ADR-016: Catalog storage

- **Status:** Proposed (accepted when every test file below is green for `in-memory` and `sqlite` in CI, and for `postgres` and `dynamodb` against a live service; the Lambda serves only an assembled document; `crates/turul-ai-catalog-lambda/catalog.json`, `BUNDLE` and `bundle_provenance.rs` are deleted; and no code path in the workspace other than `assemble` builds a served catalog from stored records)
- **Date:** 2026-11-17
- **Implements:** none yet. The implementing slice adds, under `crates/turul-ai-catalog-storage/tests/`: `parity/mod.rs` (the shared contract, not a test target), `in_memory.rs`, `sqlite.rs` (feature `sqlite`), `postgres.rs` (feature `postgres`, `#[ignore]` unless `TURUL_AI_CATALOG_POSTGRES_URL` is set), `dynamodb.rs` (feature `dynamodb`, `#[ignore]` unless `TURUL_AI_CATALOG_DYNAMODB_TABLE` is set), `record_keys.rs`, `records.rs`, `assembly.rs`, `import.rs`. Under `crates/turul-ai-catalog-lambda/tests/`: `snapshot_refresh.rs`, with `cold_start.rs`, `well_known_handler.rs` and `consumer_interop.rs` migrated (§7). `crates/turul-ai-catalog-cli/tests/dependency_direction.rs` gains `storage_default_features_add_no_runtime_or_network_dependency` and updates `lambda_dependencies_are_exactly_the_allowed_set`. `crates/turul-ai-catalog-cli/tests/bundle_provenance.rs` is deleted.
- **Depends on:** ADR-002 (`parse`, `ParseError`; gains `parse::parse_entry`, `parse::parse_host`), ADR-003 (`to_canonical_vec`, `Json::to_canonical_vec`, `CanonicalError`; `CatalogEntry::to_json` and `HostInfo::to_json` become public), ADR-004 (`EntryKey`, `CatalogEntry::key`, `SPEC_VERSION`, `WELL_KNOWN_PATH`, `MEDIA_TYPE`; gains `EntryKey::new`), ADR-005 (`validate`, `Report::has_errors`, `Severity`), ADR-006 (entry Trust Manifests), ADR-010 (the Lambda, changed by §7), ADR-011 (`produce`, `dependency_direction`). Binding decisions: **M19** (this ADR's mandate), M2 (superseded in part, §7), M3, M6, M10, M12, M13, M17, M18.
- **Spec:** https://ai-catalog.io, `specification/ai-catalog.md` as read in ADR-001 (last commit 2026-11-10).

## Context

M19 approves `turul-ai-catalog-storage`: a `CatalogStore` trait holding entry-level records with ETag versions, backends `in-memory`, `sqlite`, `postgres` and `dynamodb` as cargo features following `turul-mcp-task-storage`, and the Lambda serving from the store instead of the M2 bundle, so services can register entries at runtime. Whether turul-a2a and turul-mcp services actually do so is open (TODO F1); this ADR builds what holds either way.

The pattern followed: `turul-mcp-task-storage` has one `#[async_trait]` trait (`TaskStorage: Send + Sync`, `backend_name()`), an in-memory default, `sqlite`/`postgres`/`dynamodb` as features each pulling its own client crate, and one shared set of parity functions taking `&dyn TaskStorage` that every backend runs. Its DynamoDB backend never creates tables unless told to. Two things are not copied: the task status state machine (a catalog entry has no lifecycle in the spec), and parity tests compiled inside `src/` (here they are an integration-test module, so they test only the public API).

Spec text this ADR implements:

- #top-level-structure: a catalog "MUST contain" `specVersion` and `entries`; `entries` "MAY be empty"; `host`, `extensions`, `signature` and `additionalSignatures` are optional.
- #host-info: Host Info "MUST contain" `displayName`; it "identifies the operator of the catalog".
- #catalog-entry: `identifier`, `type` and exactly one of `url` or `data` are required. "A catalog operator incorporating an entry for the first time with a publisher-authorized `urn:air` identifier MUST preserve that identifier exactly."
- #multi-version-entries: "When `version` is present, the combination of `identifier` and `version` MUST be unique within the catalog. When `version` is absent, `identifier` alone MUST be unique." Clients needing the latest version "SHOULD sort entries sharing the same `identifier` by `version` … or by `updatedAt`".
- #catalog-projection: identifier selection rules 1 to 3; "A catalog operator MUST NOT infer publisher authorization from the artifact's URL or an unsigned `publisher` field"; an operator-assigned identifier "MUST remain stable across versions, source-coordinate changes, and retrieval-URL changes, and MUST NOT be reassigned to another artifact"; replaced source identifiers "SHOULD be stored in a namespaced entry extension" and "do not participate in catalog uniqueness".
- #catalog-snapshot-coverage: "A catalog signature covers the entire catalog object … This includes `specVersion`, every entry in order, Host Info, extensions, and all other present members". "Adding, removing, or changing a contribution therefore changes the catalog snapshot and requires a new catalog signature."
- #signed-payload-and-jws-construction: the payload is the containing object minus its own `signature.jws` and `additionalSignatures`, canonicalised with JCS; "a catalog signature includes complete nested Trust Manifests and their signatures".
- #nested-catalog-entries: "Each sub-catalog is an independent AI Catalog document with its own `host` and entries".
- #well-known-uri: "hosts MAY serve an AI Catalog at … `/.well-known/ai-catalog.json`". "Trust metadata, when present, is carried inline on the entries as Trust Manifests."
- #dynamic-discovery: "Implementing protocols MAY support dynamic catalog generation through their own mechanisms … Defining dynamic discovery behavior is out of scope for this specification."

## Decision

### 1. Crate and dependencies

- `turul-ai-catalog-storage`, `publish = true`, depends on `turul-ai-catalog` only among workspace crates. Never on `turul-ai-catalog-client` (network) or `turul-ai-catalog-trust` (JWS): the store neither fetches nor verifies.
- Normal dependencies (all `workspace = true`, new root entries at the versions `turul-mcp-task-storage` uses): `turul-ai-catalog`, `async-trait` 0.1, `sha2` (already a workspace dependency). These three are the default (`in-memory`) feature set: no async runtime, no network. The in-memory backend uses `std::sync::RwLock`.
- Features, each adding only its own client:

| Feature | Adds | Network | Async runtime |
|---|---|---|---|
| `in-memory` (default) | nothing | no | no |
| `sqlite` | `sqlx` (`sqlite`, `runtime-tokio`), `tokio` | no (local file) | yes |
| `postgres` | `sqlx` (`postgres`, `runtime-tokio`, `tls-rustls`), `tokio` | yes | yes |
| `dynamodb` | `aws-config`, `aws-sdk-dynamodb`, `tokio` | yes | yes |

- M6 holds: `turul-ai-catalog` gains no dependency. Its new public items (§2) are pure.
- `dependency_direction` pins: the storage crate's default-feature dependency set is exactly `{turul-ai-catalog, async-trait, sha2}`; no feature of it depends on the client or trust crate.

### 2. Records and keys

The store holds two record kinds per catalog: one optional host record and any number of entry records. Nothing else from the document is stored (§4 says what that excludes).

- **`CatalogId`**: a storage namespace naming one catalog document. It is not a spec member (the spec gives a catalog no identifier, and `host.identifier` is optional), so it is chosen by the deployer. `CatalogId::new(String) -> Result<CatalogId, RecordError>` accepts 1 to 128 bytes of `[A-Za-z0-9._-]`. Each nested sub-catalog is "an independent AI Catalog document" (#nested-catalog-entries), so it is its own `CatalogId`; a parent references it with an ordinary entry (`type` `application/ai-catalog+json`, `url`). The store never follows that link.
- **`EntryKey`** is the existing ADR-004 key (`identifier`, `version` as written), unchanged in meaning. ADR-004 gains `EntryKey::new(EntryIdentifier, Option<String>)`, the one typed constructor; `CatalogEntry::key` calls it. A caller that holds an identifier and version (the CLI `remove`, a deregistering service) builds the key through it and never compares fields itself. `produce`'s `Remove` migrates to compare `EntryKey`s in the same slice.
- **`EntryRecord`**: an entry that passed the record gate. `EntryRecord::new(entry: CatalogEntry) -> Result<EntryRecord, RecordError>` runs `validate` on the one-entry document `AiCatalog::new(vec![entry])` and refuses on any `Severity::Error` (`RecordError::Invalid(Report)`), then computes the record bytes `Json::to_canonical_vec(&entry.to_json())` and refuses `CanonicalError` (`RecordError::NotIJson`). It holds the entry, its key and those bytes. It is the only way to build one, so every stored entry passed `validate` and JCS (AMB-401).
- **`HostRecord`**: the same gate for Host Info: `HostRecord::new(HostInfo) -> Result<HostRecord, RecordError>`, validating `AiCatalog { host: Some(..), ..AiCatalog::new(vec![]) }`.
- **Record bytes** are JCS of the member's own JSON. Reading back uses ADR-002's new `parse::parse_entry(&[u8]) -> Result<CatalogEntry, ParseError>` and `parse::parse_host(&[u8]) -> Result<HostInfo, ParseError>`, which run the same member readers `parse` uses for `entries[i]` and `host`; no second reader exists. A stored record that fails to parse is `StoreError::Corrupt { key: RecordKey, error: ParseError }`.
- **`RecordKey`**: the backend key of a record within a catalog, owned by `RecordKey::entry(&EntryKey)` and `RecordKey::HOST`:
  - entry: `"e:"` followed by the lowercase hex SHA-256 of `Json::to_canonical_vec` of the JSON array `[identifier, version]`, where an absent `version` is `null`. `null` and `""` therefore give different keys, as `None` and `Some("")` are different `EntryKey`s.
  - host: `"h"`.
  - Fixed length (66 bytes) for every identifier, so no backend's key-length limit (DynamoDB sort key 1024 bytes, PostgreSQL btree tuple size) depends on artifact identifiers. The readable key lives inside the record bytes.
- **`Etag`**: the version of one stored record, the lowercase hex SHA-256 of its record bytes. It is computed by the crate (`Etag::of(&[u8])`), never by a backend, so every backend returns the same `Etag` for the same record and the parity contract can assert it. Same content gives the same `Etag`; a conditional write that matches therefore only ever replaces bytes equal to those the caller read, including after a delete and re-create (no ABA on content) (AMB-402).

### 3. The `CatalogStore` trait

```rust
#[async_trait]
pub trait CatalogStore: Send + Sync {
    fn backend_name(&self) -> &'static str;

    async fn put_entry(&self, catalog: &CatalogId, record: &EntryRecord, condition: Condition)
        -> Result<Etag, StoreError>;
    async fn get_entry(&self, catalog: &CatalogId, key: &EntryKey)
        -> Result<Option<Stored<EntryRecord>>, StoreError>;
    async fn delete_entry(&self, catalog: &CatalogId, key: &EntryKey, expected: Option<&Etag>)
        -> Result<(), StoreError>;
    async fn list_entries(&self, catalog: &CatalogId)
        -> Result<Vec<Stored<EntryRecord>>, StoreError>;

    async fn put_host(&self, catalog: &CatalogId, record: &HostRecord, condition: Condition)
        -> Result<Etag, StoreError>;
    async fn get_host(&self, catalog: &CatalogId)
        -> Result<Option<Stored<HostRecord>>, StoreError>;
}

pub enum Condition { Any, Absent, Matches(Etag) }
pub struct Stored<R> { pub record: R, pub etag: Etag }

#[non_exhaustive]
pub enum StoreError {
    PreconditionFailed,                      // Absent but present; Matches but absent or different
    NotFound,                                // delete of a missing record with `expected: None`
    Corrupt { key: RecordKey, error: ParseError },
    Backend(String),                         // the backend's own error, as text
}
```

- `assemble` is **not** a trait method: it is the free function in §4, so no backend can override it and the served document has one builder (AMB-403).
- There is no status, no TTL, no expiry and no lease. A record exists or it does not. Entry lifecycle beyond put and delete (refresh, expiry of a service that stopped) is F1's.
- `delete_entry` with `expected: Some(etag)` fails `PreconditionFailed` when the record is absent or its `Etag` differs. No `delete_host` is added: no writer in this workspace removes a host, and `put_host` replaces one.

What **every backend must guarantee** (the parity contract, §6):

1. **Atomic conditional write.** `put_*` and `delete_entry` evaluate their condition and write as one indivisible step against the record's current committed state. Of two concurrent `Condition::Absent` puts of one key, exactly one succeeds; of two concurrent `Matches(e)` puts, at most one succeeds.
2. **Read-your-writes.** After a write returns `Ok`, every later `get_*` and `list_entries` on the same store observes it (strongly consistent reads; DynamoDB `ConsistentRead = true`).
3. **Record integrity.** A read returns exactly the record bytes written, so `Stored.etag == Etag::of(bytes)`. A backend never reformats, truncates or partially writes a record.
4. **Complete listing.** `list_entries` returns every entry record of the catalog committed before the call began, each once, following every backend page. Order is unspecified (§4 sorts).
5. **Catalog isolation.** No operation on one `CatalogId` reads or changes another's records.
6. **Uniqueness by key.** At most one entry record per `RecordKey` per catalog, which makes #multi-version-entries uniqueness hold by construction.
7. **No snapshot across records.** Not guaranteed: `list_entries` is not required to be one isolated snapshot with `get_host`. Each record read is a committed version.

### 4. Assembly on read

`pub async fn assemble(store: &dyn CatalogStore, catalog: &CatalogId) -> Result<Assembled, AssembleError>` is the one owner of the served document:

1. `get_host` and `list_entries`.
2. Sort the entries ascending by `EntryKey`'s derived `Ord`: `identifier` by bytes, then `version` with absent before present, present versions by bytes (AMB-404).
3. Build `AiCatalog { spec_version: SPEC_VERSION, host, entries, signature: None, additional_signatures: None, extensions: None, unrecognized: empty }`.
4. `validate`; any `Severity::Error` gives `AssembleError::Invalid(Report)`.
5. `to_canonical_vec`; a `CanonicalError` gives `AssembleError::NotIJson`.
6. Return `Assembled { bytes: Vec<u8>, report: Report }` (warnings only).

Store errors give `AssembleError::Store(StoreError)`. The result is a pure function of the committed records: equal store state gives equal bytes on every backend, which the parity contract asserts byte for byte. A store with no records assembles to `{"entries":[],"specVersion":"1.0"}`, a valid Level 1 document (#top-level-structure: `entries` "MAY be empty").

**Catalog-level `signature` (#catalog-snapshot-coverage).** An assembled document never carries `signature` or `additionalSignatures`. The catalog signature covers "every entry in order, Host Info, extensions, and all other present members", and "adding, removing, or changing a contribution … requires a new catalog signature": every store write changes the snapshot, so a stored signature would be invalid after the next write, and signing each assembly would put a signing key in the serving path, which M10 and M19 do not grant. Therefore:

- The store holds no catalog signature, and `import` (§5) refuses a signed document (`ImportError::Signed`), as `produce` refuses one (`ProduceError::Signed`).
- A signed catalog is a fixed file, produced and signed outside the store and published at its own URL; a store-assembled catalog may reference it with a nested-catalog entry.
- **Entry Trust Manifests ride inside entry records.** An `EntryRecord` keeps `trustManifests`, `digest`, `publisher` and every other entry member, including unrecognized ones, as parsed. A manifest signature covers the JCS of the manifest (#signed-payload-and-jws-construction), and ADR-002 keeps every member and number lexeme, so storing, reading and re-serialising an entry leaves each manifest's payload unchanged. #entry-release-coverage still binds: a writer that changes an entry's `version` or `digest` without a matching manifest makes that manifest unacceptable for the entry; the store does not detect this, verification (ADR-015) does.
- Top-level `extensions` and unrecognized top-level members are not stored, so an assembled document has none. `import` refuses a document that has either (`ImportError::Unstorable { pointer }`) rather than drop a member silently (AMB-405).

**Mapping to #catalog-projection.** A stored catalog is the operator's catalog, and every `put_entry` of an entry taken from another catalog or source system is a projection. The store:

- never assigns, rewrites or normalises an identifier: an `EntryRecord` holds the identifier the writer gave, so rule 1 ("MUST preserve it exactly") holds for anything the writer passes through;
- keys only on `(identifier, version)`, so replaced source identifiers kept in a namespaced entry extension "do not participate in catalog uniqueness", as required;
- does not decide publisher authorization, identifier assignment (rules 2 and 3) or stability across versions and URL changes ("MUST NOT be reassigned to another artifact"). Those are the writer's obligations, and the store cannot check them: it has no history of what an identifier named before. Who the writer is, and with what authority, is F1;
- states the operator through `put_host` (#catalog-projection: "The operator is the entity identified by the top-level `host` field").

Requests vary nothing: the same store state is served to every caller, so the Lambda performs no #dynamic-discovery.

### 5. Writers

- **One gate, two targets.** "Change → validate → write" has one validation owner, `validate` and `to_canonical_vec` (ADR-005, ADR-003), reached by two writers that each own one target:
  - file: `turul_ai_catalog_client::produce` (M17, ADR-011), unchanged except for `Remove` comparing `EntryKey`s (§2);
  - store: `EntryRecord::new` / `HostRecord::new` followed by a conditional `put_*` or `delete_entry`. The record gate is the store's validate step; a writer cannot reach a backend without passing it.
  
  `produce` gains no store target: it is synchronous and lives in the client crate, which carries the network dependency, and a store target would make every store writer (services, the Lambda) depend on the client (AMB-406).
- **`import(store: &dyn CatalogStore, catalog: &CatalogId, document: &AiCatalog) -> Result<Vec<Etag>, ImportError>`** is the one path from a catalog document to store records: it refuses a signed document or one with top-level members the store cannot hold (§4), gates the host and each entry, then writes the host and each entry with `Condition::Any`. It is how a curated catalog file (made with the M17 producer commands) is loaded into a store, and how the local server seeds its in-memory store (§7). It is not atomic across records; a failure part-way leaves the records written so far, and re-running it converges.
- **Runtime registration** by turul-a2a and turul-mcp services, if F1 decides for it, goes through this crate's public API only: build the entry with the `turul-ai-catalog` model, `EntryRecord::new`, `put_entry` (`Condition::Absent` to create, `Matches` to replace), `delete_entry` to deregister. Those repos depend on `turul-ai-catalog-storage` and implement nothing of it (workspace boundary, CLAUDE.md). Their work goes in handoffs once F1 is decided.

### 6. Backends and the parity contract

Each backend is one public type with an async constructor taking its own config. No backend creates cloud infrastructure.

| Backend | Type | Storage shape |
|---|---|---|
| `in-memory` | `InMemoryCatalogStore::new()` | `RwLock<BTreeMap<(CatalogId, RecordKey), (Vec<u8>, Etag)>>` |
| `sqlite` | `SqliteCatalogStore::connect(path)` | table `catalog_records (catalog_id TEXT NOT NULL, record_key TEXT NOT NULL, record BLOB NOT NULL, etag TEXT NOT NULL, PRIMARY KEY (catalog_id, record_key))`; created if missing on connect (a local file the process owns) |
| `postgres` | `PostgresCatalogStore::connect(url)` | the same table with `record BYTEA`; `CREATE TABLE IF NOT EXISTS` on connect, so the role needs that privilege or the table pre-created |
| `dynamodb` | `DynamoDbCatalogStore::new(client, table_name)` | one table, partition key `pk` (S) = `CatalogId`, sort key `sk` (S) = `RecordKey` (`"h"` or `"e:<hex>"`); attributes `record` (B) and `etag` (S). Never creates or verifies the table |

- Conditional writes: SQL uses `INSERT … ON CONFLICT DO NOTHING` for `Absent`, `UPDATE … WHERE etag = $e` for `Matches`, and checks the affected-row count; DynamoDB uses `ConditionExpression` `attribute_not_exists(pk)` for `Absent` and `etag = :e` for `Matches`, mapping `ConditionalCheckFailedException` to `PreconditionFailed`.
- `list_entries`: SQL `WHERE catalog_id = $1 AND record_key LIKE 'e:%'`; DynamoDB `Query` on `pk = :c AND begins_with(sk, "e:")` with `ConsistentRead`, following `LastEvaluatedKey` to the end.
- Record size: DynamoDB refuses items over 400 KB, which an entry with large inline `data` can exceed; that is `StoreError::Backend` naming the limit. No backend caps size below its native limit.

The parity contract is `tests/parity/mod.rs`: `pub async fn` checks taking `&dyn CatalogStore` (and `Arc<dyn CatalogStore>` for concurrency), run by `in_memory.rs` and `sqlite.rs` in CI, and by `postgres.rs` and `dynamodb.rs` against a live service when their environment variable is set (DynamoDB Local works). Each backend file calls every check; a check added to the module and not called fails review, not silently.

### 7. The Lambda (supersedes M2's bundle; changes ADR-010)

- **Serving.** `main` builds the store, then `ServedCatalog::assemble(store: &dyn CatalogStore, catalog: &CatalogId) -> Result<ServedCatalog, ColdStartError>`, which calls `assemble` and holds the bytes. An `AssembleError` fails cold start exactly as an invalid bundle did. `handle(&ServedCatalog, &Request)` stays pure and unchanged in routing, status codes, headers and `Content-Type`; the GET body is the assembled bytes.
- **Freshness.** `SnapshotCache` (lambda lib) holds an `Arc<ServedCatalog>` and the instant it was assembled. On a request, when its age is at least `max_age`, `main` re-assembles before calling `handle`; a failed re-assembly keeps serving the last good snapshot. `max_age` is `TURUL_AI_CATALOG_MAX_AGE_SECS`, default 60. The clock lives in `SnapshotCache`, never in `handle`. Trade-off: a registration becomes visible within `max_age` per warm instance, and each refresh is one `GetItem` and one `Query` per instance; cold-start-only assembly would make visibility depend on instance recycling, which defeats M19's runtime registration. `0` re-assembles on every request.
- **Backend selection.** Built with the lambda's `dynamodb` feature (forwarding `turul-ai-catalog-storage/dynamodb`), `main` requires `TURUL_AI_CATALOG_TABLE` and `TURUL_AI_CATALOG_ID` and fails cold start naming a missing one, so a deployed function never serves an empty in-memory store by accident. Built without it, `main` uses an empty `InMemoryCatalogStore` and `TURUL_AI_CATALOG_ID` defaults to `default`; `cargo lambda watch` keeps working and serves the empty valid catalog.
- **Local server (M18).** `turul-ai-catalog-serve --file PATH` parses the file, `import`s it into an `InMemoryCatalogStore`, and serves through `ServedCatalog::assemble` and `handle`. Without `--file` it serves the empty store. It remains the only code in the crate that reads a file or binds a port.
- **IAM.** The deploying repo owns the table, the function and every grant. The serving role needs `dynamodb:GetItem` and `dynamodb:Query` on the table only; writers need `dynamodb:PutItem` and `dynamodb:DeleteItem`. This repo ships no IAM, SAM or CDK (M2's "handler only" stands).

**What changes in ADR-010** (updated, with its revision log, in the implementing slice):

| ADR-010 | Change |
|---|---|
| §1 dependencies | adds `turul-ai-catalog-storage`; feature `dynamodb` forwards to it; `src/` still holds no `std::fs` or `std::net` outside `src/bin/serve.rs` |
| §2 bundle and cold start | `BUNDLE`, `catalog.json` and `ServedCatalog::load(&'static [u8])` deleted; cold start is `ServedCatalog::assemble`; `ColdStartError` becomes `{Assemble(AssembleError), Config(&'static str)}`; `ColdStartError::Parse` goes, since records are parsed at write time and a corrupt record is `StoreError::Corrupt` |
| §3 routing | unchanged, except the body is the assembled bytes; D18's "verbatim" now means verbatim from `ServedCatalog` |
| §4 what is served | any assembled document without error diagnostics; `url`-nested children still not fetched |
| §5 interop, live probe | fixtures are `import`ed into an in-memory store; `live_probe` asserts both clients accept the deployed document and agree, no longer that it equals `BUNDLE` |
| §6 bundle content and provenance | deleted, with `bundle_provenance.rs`, AMB-117 and AMB-118. M17's producer commands still author files; a curated deployment `import`s one |
| AMB-116 | superseded: bytes are canonical, and non-I-JSON numbers are refused at the record gate, not served |
| M18 | `--file` goes through `import` and `assemble`; no default bundle |

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Catalog namespace | `turul_ai_catalog_storage::CatalogId::new` | writers, `assemble`, Lambda `main` |
| Entry identity | `turul_ai_catalog::identity::EntryKey::new` (ADR-004); `CatalogEntry::key` | `get_entry`, `delete_entry`, `produce` `Remove` |
| Backend record key | `RecordKey::entry(&EntryKey)`, `RecordKey::HOST` | every backend |
| Record version | `Etag::of(&[u8])` (SHA-256 of record bytes) | every backend, via the crate |
| Entry record bytes | `EntryRecord::new` (`validate`, then `Json::to_canonical_vec` of `CatalogEntry::to_json`) | `put_entry` |
| Host record bytes | `HostRecord::new` | `put_host` |
| Record reading | `parse::parse_entry`, `parse::parse_host` (ADR-002) | every backend's reads |
| Store contract | `trait CatalogStore` | four backend types |
| Served document | `assemble` | `ServedCatalog::assemble` |
| Document → records | `import` | curated loads, `turul-ai-catalog-serve` |
| Served snapshot freshness | `turul_ai_catalog_lambda::SnapshotCache` | Lambda `main` |

Schema digests: record bytes are ADR-003 JCS of ADR-002's member model, whose digest is ADR-003's; `RecordKey` and `Etag` are SHA-256 over JCS and record bytes. No new schema. Determinism is proved by `assembly.rs` and the parity check `same_state_assembles_to_the_same_bytes`.

## Requirement → test

Paths are under `crates/turul-ai-catalog-storage/tests/` unless stated. `parity::x` runs on every backend.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| multi-version-entries-1 | `identifier` + `version` "MUST be unique" | `parity::one_record_per_entry_key` (put twice with `Any` keeps one; same identifier, versions `1`, `2` and absent are three records) |
| multi-version-entries-2 | absent `version`: `identifier` alone "MUST be unique" | `record_keys::absent_and_empty_version_are_distinct_keys`, `record_keys::distinct_entry_keys_give_distinct_record_keys` |
| top-level-structure-1 | `specVersion` and `entries` required; `entries` "MAY be empty" | `assembly::empty_store_assembles_to_a_valid_empty_catalog` |
| host-info-1 | Host Info "MUST contain" `displayName` | `records::host_record_refuses_an_invalid_host` |
| catalog-entry-1 | required members; exactly one of `url` or `data` | `records::entry_record_refuses_an_entry_with_an_error_diagnostic` |
| catalog-entry-2 / catalog-projection-1 | publisher-authorized `urn:air` identifier "MUST preserve … exactly" | `parity::stored_entry_reads_back_byte_identical` (identifier, extensions and unrecognized members kept) |
| catalog-projection-2 | retained source identifiers "do not participate in catalog uniqueness" | `record_keys::extensions_do_not_change_the_record_key` |
| catalog-snapshot-coverage-1 | changing a contribution "requires a new catalog signature" | `import::signed_document_is_refused`, `assembly::assembled_document_has_no_signature` |
| signed-payload-1 | manifest payload is JCS of the manifest | `parity::entry_trust_manifest_payload_survives_storage` (the JCS of each `trustManifests[i]` minus `signature.jws` is equal before and after a round trip) |
| nested-catalogs-1 | each sub-catalog is "an independent AI Catalog document" | `parity::catalogs_are_isolated` |

Keyword sentences in the cited sections owned elsewhere or out of scope:

| Requirement | Owner |
|---|---|
| catalog-projection-3 (MUST NOT infer authorization from URL or unsigned `publisher`) | the writer (F1); verification is ADR-015 |
| catalog-projection-4 (operator-assigned identifier MUST remain stable, MUST NOT be reassigned) | the writer (F1); the store holds no identifier history |
| multi-version-entries-3 (clients SHOULD sort by `version` or `updatedAt` for latest) | client, ADR-007 |
| well-known-uri-1 (hosts MAY serve at the well-known URI) | ADR-010, `well_known_handler.rs` |
| entry-release-coverage (version or digest change invalidates a manifest) | ADR-015 |
| dynamic-discovery-1 (out of scope) | OOS: statement |

Maintainer decisions and ambiguities owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| M19 (ETag, conditional writes) | `Absent`, `Matches`, `Any`; precondition failures typed | `parity::absent_condition_refuses_an_existing_record`, `parity::matches_condition_refuses_a_stale_etag`, `parity::delete_with_a_stale_etag_is_refused`, `parity::delete_of_a_missing_record_is_not_found` |
| M19 (atomicity) | of concurrent `Absent` puts exactly one wins | `parity::concurrent_absent_puts_have_one_winner` |
| M19 (read-your-writes, completeness) | a returned write is visible; listing is complete across pages | `parity::writes_are_visible_to_the_next_read`, `parity::list_returns_every_entry` (300 entries; `dynamodb.rs` also runs it with more than 1 MB of entries, so `Query` returns several pages) |
| M19 (parity) | equal state, equal bytes on every backend | `parity::same_state_assembles_to_the_same_bytes` (asserts against a committed expected byte string, so backends are compared through it) |
| M6 | core crate unchanged in dependencies; storage default adds no runtime or network | `dependency_direction::storage_default_features_add_no_runtime_or_network_dependency` (CLI crate) |
| M2 / M19 (Lambda) | the Lambda serves the assembled document | ADR-010's migrated `well_known_handler.rs`, `cold_start::invalid_store_fails_cold_start`, `cold_start::missing_table_configuration_fails_cold_start` (feature `dynamodb`) |
| AMB-401 | the record gate is the only constructor | `records::entry_record_refuses_non_i_json`, `records::entry_record_bytes_are_canonical` |
| AMB-402 | `Etag` is content-derived and backend-independent | `parity::etag_is_the_digest_of_the_record_bytes`, `parity::recreated_identical_record_has_the_same_etag` |
| AMB-404 | entries are ordered by `EntryKey` | `assembly::entries_are_ordered_by_entry_key` (insertion order reversed, absent version before present) |
| AMB-405 | top-level members the store cannot hold are refused on import | `import::top_level_extensions_are_refused`, `import::unrecognized_top_level_members_are_refused` |
| AMB-407 | a failed refresh keeps the last good snapshot | `snapshot_refresh::stale_snapshot_is_reassembled`, `snapshot_refresh::failed_reassembly_keeps_the_last_good_snapshot`, `snapshot_refresh::fresh_snapshot_is_not_reassembled` (lambda crate) |

New ids, block AMB-400 to AMB-419:

- AMB-401: M19 asks that the served document go through `validate`; an entry that cannot pass alone would make every later assembly fail and the Lambda stop refreshing. Decision: gate at write time per record (entry-local rules) and again at assembly (the whole document). Cross-entry errors that only appear in the whole are found at assembly, which keeps the last good snapshot (AMB-407).
- AMB-402: a counter version per record allows ABA after delete and re-create with different content. A content digest makes a match mean "the bytes I read", needs no clock or counter in any backend, and is equal across backends. Trade-off: `sha2` is used directly here; `turul-ai-catalog-trust` also computes SHA-256 for #digest-format, which is a different artifact (an artifact digest, not a store version), and the storage crate must not depend on the trust crate (§1).
- AMB-403: the brief lists `assemble` among the store operations. A trait method with a default body can be overridden per backend, making a second document builder. Decision: a free function over `&dyn CatalogStore`.
- AMB-404: the spec fixes no entry order for an unsigned catalog; it fixes "every entry in order" only for a signature, which assembly never carries. `EntryKey`'s derived `Ord` is total, deterministic and backend-independent; insertion time is not, and would need a clock. Clients choose latest versions themselves (#multi-version-entries).
- AMB-405: silent loss of a top-level member would change what the operator published. Refusing keeps the store's content an exact subset of the spec document. Storing catalog-level `extensions` waits for a writer that needs them.
- AMB-406: `produce` stays file-only; see §5.
- AMB-407: a refresh failure (store unreachable, or a cross-entry error) serving a 5xx would make discovery depend on store health. Serving the last validated snapshot keeps every response a document that passed `validate`.

## Consequences

- The Lambda's content comes from a store the deploying repo provisions; the repo builds and tests the handler with the in-memory backend and needs no AWS account for CI.
- The well-known document can no longer carry a catalog-level `signature`. A signed catalog is published as a fixed file elsewhere and referenced by URL.
- Served bytes are canonical JCS, not a hand-formatted file.
- `bundle_provenance` and the default Acme bundle go: a deployment's content is its store, loaded by `import` or by registering writers.
- Assembly costs one host read and one listing per refresh per warm instance. Catalogs with very many entries make each refresh proportionally larger; paging the served document is not defined by the spec and is not built.
- turul-a2a and turul-mcp-framework need nothing from this ADR until F1 decides runtime registration.

## Open question (not blocking): runtime self-registration or curated deployment

TODO F1: do turul-a2a and turul-mcp services add their own entries at runtime, or is the catalog curated with the CLI and deployed? Either way the following hold unchanged: the trait and its guarantees, records and keys, assembly and its order, the Lambda serving and cache, and the backends. Curated: the producer commands write a file, `import` loads it, and only a deployment pipeline holds write grants. Self-registration: services write through `EntryRecord::new` and conditional puts. What F1 must still decide, and this ADR does not: which services register, the authority under which they write (and who satisfies #catalog-projection's authorization and identifier-stability duties), and entry lifecycle (refresh, removal, expiry of a service that stopped). Expiry would add state to the store and is not built here.

## What this ADR does not do

- No code, tests or `Cargo.toml` changes: they follow in the implementing slice, which also updates ADR-002, ADR-003, ADR-004, ADR-010 and ADR-011 and their revision logs, `CHANGELOG.md` and the version.
- No new maintainer decision: it implements M19 as recorded in `docs/plan/DECISIONS.md`.
- No signing of assembled catalogs, no TTL or expiry of records, no CLI store commands, no infrastructure.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created under M19: crate and features, `CatalogId`, `EntryRecord`/`HostRecord` gate, `RecordKey`, content-derived `Etag`, `CatalogStore` trait and backend guarantees, `assemble`, `import`, backend schemas including the DynamoDB `pk`/`sk` shape, parity contract, Lambda changes to ADR-010. AMB-401 to AMB-407 added. |
