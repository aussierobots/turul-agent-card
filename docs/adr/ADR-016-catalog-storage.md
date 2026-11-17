# ADR-016: Catalog storage

- **Status:** Proposed (accepted when every test file below is green for `in-memory` and `sqlite` in CI, and for `postgres` and `dynamodb` against a live service; the Lambda serves only an assembled document; `crates/turul-ai-catalog-lambda/catalog.json`, `BUNDLE` and `bundle_provenance.rs` are deleted; and no code path in the workspace other than `assemble` builds a served catalog from stored records)
- **Date:** 2026-11-17
- **Implements:** under `crates/turul-ai-catalog-storage/tests/`: `parity/mod.rs` (the shared contract, not a test target), `in_memory.rs`, `sqlite.rs` (feature `sqlite`), `postgres.rs` (feature `postgres`, `#[ignore]`; it owns its environment, §6), `dynamodb.rs` (feature `dynamodb`, `#[ignore]`; it owns its environment, §6), `record_keys.rs`, `records.rs`, `assembly.rs`, `import.rs`. Under `crates/turul-ai-catalog-lambda/tests/`: `snapshot_refresh.rs`, with `cold_start.rs`, `well_known_handler.rs` and `consumer_interop.rs` migrated (§7). `crates/turul-ai-catalog-cli/tests/dependency_direction.rs` gains `storage_default_features_add_no_runtime_or_network_dependency` and updates `lambda_dependencies_are_exactly_the_allowed_set`. `crates/turul-ai-catalog-cli/tests/bundle_provenance.rs` is deleted (done).
- **Depends on:** ADR-002 (`parse`, `ParseError`; gains `parse::parse_entry`, `parse::parse_host`), ADR-003 (`to_canonical_vec`, `Json::to_canonical_vec`, `CanonicalError`; `CatalogEntry::to_json` and `HostInfo::to_json` become public), ADR-004 (`EntryKey`, `CatalogEntry::key`, `SPEC_VERSION`, `WELL_KNOWN_PATH`, `MEDIA_TYPE`; gains `EntryKey::new`), ADR-005 (`validate`, `Report::has_errors`, `Severity`), ADR-006 (entry Trust Manifests), ADR-010 (the Lambda, changed by §7), ADR-011 (`produce`, `dependency_direction`). Binding decisions: **M19** (this ADR's mandate), **M21** (version-kind conflicts refused at write time, §3), **M30** (purge, overwrite and the tolerant scan, §3 and §6; fail forward, no read of an older stored format), M2 (superseded in part, §7), M3, M6, M10, M12, M13, M17, M18.
- **Spec:** https://ai-catalog.io, `specification/ai-catalog.md` as read in ADR-001 (last commit 2026-11-10).

## Context

M19 approves `turul-ai-catalog-storage`: a `CatalogStore` trait holding entry-level records with ETag versions, backends `in-memory`, `sqlite`, `postgres` and `dynamodb` as cargo features following `turul-mcp-task-storage`, and the Lambda serving from the store instead of the M2 bundle, so services can register entries at runtime. Who registers, under what authority, and entry lifecycle are ADR-017's (M26); this ADR builds the storage both curated and registered entries go through.

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
| `sqlite` | `sqlx` (`sqlite`, `runtime-tokio`; tokio comes through it) | no (local file) | yes |
| `postgres` | `sqlx` (`postgres`, `runtime-tokio`, `tls-rustls`), `tokio` | yes | yes |
| `dynamodb` | `aws-sdk-dynamodb` (the caller builds the `Client`, with `aws-config` in the deploying binary), `tokio` (`time`, the timer of the retry backoff, §6) | yes | yes |

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
  - guard: `"g:"` followed by the lowercase hex SHA-256 of `Json::to_canonical_vec` of the JSON array `[identifier]`, built by `RecordKey::guard(&EntryIdentifier)`. One per identifier per catalog; it is never an entry or host key (a different prefix, and a one-element preimage where entry keys hash two). It names the guard that holds the identifier's version kind (§3 guarantee 6, §6).
  - Fixed length (66 bytes) for every identifier, so no backend's key-length limit (DynamoDB sort key 1024 bytes, PostgreSQL btree tuple size) depends on artifact identifiers. The readable key lives inside the record bytes.
- **`Etag`**: the version of one stored record: for the host, the lowercase hex SHA-256 of its record bytes; for an entry, `Etag::of_entry(&EntryRecord, &Provenance)`, the SHA-256 of the JCS of `[entry JSON, provenance JSON]` (ADR-017 §4.2). It is computed by the crate (`Etag::of(&[u8])`), never by a backend, so every backend returns the same `Etag` for the same record and the parity contract can assert it. Same content gives the same `Etag`; a conditional write that matches therefore only ever replaces bytes equal to those the caller read, including after a delete and re-create (no ABA on content) (AMB-402).

### 3. The `CatalogStore` trait

```rust
#[async_trait]
pub trait CatalogStore: Send + Sync {
    fn backend_name(&self) -> &'static str;

    async fn put_entry(&self, catalog: &CatalogId, record: &EntryRecord, provenance: &Provenance, condition: Condition)
        -> Result<Etag, StoreError>;
    async fn get_entry(&self, catalog: &CatalogId, key: &EntryKey)
        -> Result<Option<StoredEntry>, StoreError>;
    async fn delete_entry(&self, catalog: &CatalogId, key: &EntryKey, expected: Option<&Etag>)
        -> Result<(), StoreError>;
    async fn list_entries(&self, catalog: &CatalogId)
        -> Result<Vec<StoredEntry>, StoreError>;

    async fn put_host(&self, catalog: &CatalogId, record: &HostRecord, condition: Condition)
        -> Result<Etag, StoreError>;
    async fn get_host(&self, catalog: &CatalogId)
        -> Result<Option<Stored<HostRecord>>, StoreError>;

    async fn scan_entries(&self, catalog: &CatalogId)
        -> Result<Vec<ScannedEntry>, StoreError>;
    async fn count_catalog(&self, catalog: &CatalogId)
        -> Result<ItemCounts, StoreError>;
    async fn purge_catalog(&self, catalog: &CatalogId)
        -> Result<PurgeReport, StoreError>;
}

pub enum Condition { Any, Absent, Matches(Etag), Overwrite(Etag) }
pub struct Stored<R> { pub record: R, pub etag: Etag }                       // the host
pub struct StoredEntry { pub record: EntryRecord, pub provenance: Provenance, pub etag: Etag }
pub struct ScannedEntry { pub key: EntryKey, pub etag: Etag, pub provenance: Result<Provenance, ProvenanceError> }
pub struct ItemCounts { pub host: u64, pub entries: u64, pub guards: u64, pub other: u64 }   // `total()` sums them
pub struct PurgeReport { pub removed: ItemCounts }

#[non_exhaustive]
pub enum StoreError {
    PreconditionFailed,                      // Absent but present; Matches but absent or different
    NotFound,                                // delete of a missing record with `expected: None`
    VersionKindConflict { identifier: EntryIdentifier }, // a new entry beside entries of the other version kind
    ModeConflict { stored: ModeKind },       // a put that would replace a record of the other mode (guarantee 8)
    CorruptProvenance { key: RecordKey, error: ProvenanceError }, // provenance missing, not parsing, or of an unknown mode; or a put's provenance that would not read back
    Corrupt { key: RecordKey, error: ParseError },
    Contended,                               // concurrent transactions conflicted on every attempt (DynamoDB, §6)
    Backend(String),                         // the backend's own error, as text
}
```

- `assemble` is **not** a trait method: it is the free function in §4, so no backend can override it and the served document has one builder (AMB-403).
- The trait holds no status, no TTL and no clock. Each entry is stored with the `Provenance` it was put with (its mode, writer and, for a lease, expiry); lifecycle (registration, refresh, expiry, ownership) is decided above the trait by ADR-017 (§3 to §6), and assembly applies expiry (§4).
- A stored entry whose mode is missing or unknown is `CorruptProvenance` on every backend, never `Backend` text: on a read when its provenance bytes do not parse, and on a put over it when the stored mode does not name `lease` or `curated`. A put whose provenance would not read back (`Provenance::to_bytes` refuses it, e.g. an expiry past year 9999) is refused with `CorruptProvenance` before the store is touched, so no backend stores a record its own reads would refuse.
- `delete_entry` with `expected: Some(etag)` fails `PreconditionFailed` when the record is absent or its `Etag` differs. No `delete_host` is added: `put_host` replaces a host, and only `purge_catalog` removes one.
- **`Condition::Overwrite(expected)`** (M30, M31) replaces the entry record stored under the key when its stored version is `expected`, whatever its mode and whether its provenance reads: the backend compares the stored `Etag` and reads nothing else of the old record (bytes, provenance or mode), and guarantee 8 does not apply. A key with no record, or one whose version differs, fails `PreconditionFailed` and nothing is written, as `Matches`. A replace keeps the key and so the kind, and leaves the guard count unchanged; it never creates, so guarantee 6 has nothing to check. `put_host` treats it as `Matches(expected)`. Its one caller in the workspace is `import` in `ImportMode::Replace` (ADR-017 §4.8), which passes each scanned entry's version so that a record written after the scan is never overwritten.
- **`scan_entries`** (M30) lists every entry record of the catalog as its `EntryKey` (from `parse::parse_entry` of the record bytes, the one record reader; record bytes have had one format since this ADR), its stored `Etag` as written, and its provenance as `Provenance::parse` reads it, or the `ProvenanceError` it gives (`Missing` for an item without provenance). An unreadable provenance is a value in the result, never a failure of the call; record bytes that do not parse fail the call with `Corrupt`. Assembly never calls it: `list_entries` stays the strict read. Its one caller is `import` in `ImportMode::Replace`.
- **`count_catalog`** and **`purge_catalog`** (M30) work on items by backend key only. Every item stored under the catalog id is counted by kind from its key: `"h"` is `host`, `"e:…"` `entries`, a guard (a `"g:"` key or a `catalog_guards` row) `guards`, any other key `other`. Neither reads record bytes, provenance, mode or guard values, so an item written by any release, or one this release cannot read, is counted and removed alike. `count_catalog` is what a purge would remove at the moment of the call. `purge_catalog` removes those items and returns their count in `PurgeReport::removed`; a catalog id with no items returns all zeros. Both serve the operator command `store purge` (ADR-017 §8).

What **every backend must guarantee** (the parity contract, §6):

1. **Atomic conditional write.** `put_*` and `delete_entry` evaluate their condition and write as one indivisible step against the record's current committed state. Of two concurrent `Condition::Absent` puts of one key, exactly one succeeds; of two concurrent `Matches(e)` puts, at most one succeeds.
2. **Read-your-writes.** After a write returns `Ok`, every later `get_*` and `list_entries` on the same store observes it (strongly consistent reads; DynamoDB `ConsistentRead = true`).
3. **Record integrity.** A read returns exactly the record bytes written, so a host's `Stored.etag == Etag::of(bytes)` and an entry's `StoredEntry.etag == Etag::of_entry(record, provenance)`. A backend never reformats, truncates or partially writes a record.
4. **Complete listing.** `list_entries` and `scan_entries` return every entry record of the catalog committed before the call began, each once, following every backend page; `count_catalog` counts every item so committed. Order is unspecified (§4 sorts).
5. **Catalog isolation.** No operation on one `CatalogId` reads or changes another's records.
6. **Uniqueness by key and by version kind.** At most one entry record per `RecordKey` per catalog, which enforces the first #multi-version-entries rule, "the combination of `identifier` and `version` MUST be unique". And no identifier has both an unversioned and a versioned entry record, which enforces the second, "When `version` is absent, `identifier` alone MUST be unique" (M21): `put_entry` of a record that does not yet exist fails `VersionKindConflict { identifier }` when its identifier already has stored entry records of the other version kind (`version` absent versus present; `Some("")` is present). The check and the write are one atomic step, as in guarantee 1: of two concurrent puts of `a` and `a@1`, at most one succeeds. Order: a failed `Condition` is `PreconditionFailed` first, so only a new record can conflict; a replace keeps its key and therefore its kind. `delete_entry` keeps the identifier's state in the same step, so deleting its last record of a kind frees it for the other kind. Each backend holds this in a per-identifier guard (§6) keyed by `RecordKey::guard`; the guard is internal and never listed or read through the trait.
7. **No snapshot across records.** Not guaranteed: `list_entries` is not required to be one isolated snapshot with `get_host`. Each record read is a committed version.
8. **Mode exclusion** (ADR-017 §4.3). A `put_entry` that would replace a record whose `ModeKind` differs from the new provenance's fails `StoreError::ModeConflict { stored }`, evaluated and written in one atomic step. Order of checks: `PreconditionFailed`, then `ModeConflict`, then `VersionKindConflict`. Proved by `parity::lease_and_curated_puts_refuse_each_other` and `parity::racing_cross_mode_puts_leave_one_mode`. `Condition::Overwrite` is exempt: it replaces a record of either mode, or one whose provenance does not read, at the expected version, in one atomic step.
9. **Purge** (M30). `purge_catalog` removes every item stored under the catalog id that was committed before the call began (host, entry records of either mode, guards, and any item whose key or contents this release does not read) without reading record bytes, provenance, mode or guard values, and returns their count by kind. It changes no other catalog (guarantee 5). Afterwards the id behaves as one never written: `assemble` gives the empty catalog, `get_host` is `None`, and an identifier of either version kind can be created. An item written concurrently with the call may survive it; a second call removes it. In-memory and SQL remove the items in one atomic step; DynamoDB does not (§6), so a failure part-way leaves the items not yet removed, and re-running converges.

### 4. Assembly on read

`pub async fn assemble(store: &dyn CatalogStore, catalog: &CatalogId, now: SystemTime) -> Result<Assembled, AssembleError>` is the one owner of the served document:

1. `get_host` and `list_entries`. Leases with `now >= expires_at` move to `Assembled::omitted` as `OmissionReason::LeaseExpired` (ADR-017 §6).
2. Sort the entries ascending by `EntryKey`'s derived `Ord`: `identifier` by bytes, then `version` with absent before present, present versions by bytes (AMB-404).
3. Build `AiCatalog { spec_version: SPEC_VERSION, host, entries, signature: None, additional_signatures: None, extensions: None, unrecognized: empty }`.
4. `validate`; on any `Severity::Error`, each entry that fails validation on its own moves to `omitted` as `OmissionReason::Invalid(Report)` and the document is validated again; errors that remain give `AssembleError::Invalid(Report)`.
5. `to_canonical_vec`; a `CanonicalError` gives `AssembleError::NotIJson`.
6. Return `Assembled { bytes, report, omitted, next_expiry }`: `report` has warnings only, every listed record is in `bytes` or `omitted`, and `next_expiry` is the earliest expiry among the leases in `bytes`.

Store errors give `AssembleError::Store(StoreError)`. Step 4 stays although guarantee 6 now keeps cross-entry errors out of every backend: a store written before the guard existed (0.0.17) may hold an unversioned entry beside versioned ones, and a `CatalogStore` implemented outside this crate may not keep the guarantee; assembly refuses such a document rather than serve it. The result is a pure function of the committed records and `now`: equal store state and equal `now` give equal bytes on every backend, which the parity contract asserts byte for byte. A store with no records assembles to `{"entries":[],"specVersion":"1.0"}`, a valid Level 1 document (#top-level-structure: `entries` "MAY be empty").

**Catalog-level `signature` (#catalog-snapshot-coverage).** An assembled document never carries `signature` or `additionalSignatures`. The catalog signature covers "every entry in order, Host Info, extensions, and all other present members", and "adding, removing, or changing a contribution … requires a new catalog signature": every store write changes the snapshot, so a stored signature would be invalid after the next write, and signing each assembly would put a signing key in the serving path, which M10 and M19 do not grant. Therefore:

- The store holds no catalog signature, and `import` (§5) refuses a signed document (`ImportError::Signed`), as `produce` refuses one (`ProduceError::Signed`).
- A signed catalog is a fixed file, produced and signed outside the store and published at its own URL; a store-assembled catalog may reference it with a nested-catalog entry.
- **Entry Trust Manifests ride inside entry records.** An `EntryRecord` keeps `trustManifests`, `digest`, `publisher` and every other entry member, including unrecognized ones, as parsed. A manifest signature covers the JCS of the manifest (#signed-payload-and-jws-construction), and ADR-002 keeps every member and number lexeme, so storing, reading and re-serialising an entry leaves each manifest's payload unchanged. #entry-release-coverage still binds: a writer that changes an entry's `version` or `digest` without a matching manifest makes that manifest unacceptable for the entry; the store does not detect this, verification (ADR-015) does.
- Top-level `extensions` and unrecognized top-level members are not stored, so an assembled document has none. `import` refuses a document that has either (`ImportError::Unstorable { pointer }`) rather than drop a member silently (AMB-405).

**Mapping to #catalog-projection.** A stored catalog is the operator's catalog, and every `put_entry` of an entry taken from another catalog or source system is a projection. The store:

- never assigns, rewrites or normalises an identifier: an `EntryRecord` holds the identifier the writer gave, so rule 1 ("MUST preserve it exactly") holds for anything the writer passes through;
- keys only on `(identifier, version)`, so replaced source identifiers kept in a namespaced entry extension "do not participate in catalog uniqueness", as required;
- does not decide publisher authorization, identifier assignment (rules 2 and 3) or stability across versions and URL changes ("MUST NOT be reassigned to another artifact"). Those are the writer's obligations, and the store cannot check them: it has no history of what an identifier named before. Who the writer is, and with what authority, is ADR-017 (§2, §3);
- states the operator through `put_host` (#catalog-projection: "The operator is the entity identified by the top-level `host` field").

Requests vary nothing: the same store state is served to every caller, so the Lambda performs no #dynamic-discovery.

### 5. Writers

- **One gate, two targets.** "Change → validate → write" has one validation owner, `validate` and `to_canonical_vec` (ADR-005, ADR-003), reached by two writers that each own one target:
  - file: `turul_ai_catalog_client::produce` (M17, ADR-011), unchanged except for `Remove` comparing `EntryKey`s (§2);
  - store: `EntryRecord::new` / `HostRecord::new` followed by a conditional `put_*` or `delete_entry`; entry writes go through ADR-017's `write_owned` (`import`, `Registrar::register`). The record gate is the store's validate step; a writer cannot reach a backend without passing it.
  
  `produce` gains no store target: it is synchronous and lives in the client crate, which carries the network dependency, and a store target would make every store writer (services, the Lambda) depend on the client (AMB-406).
- **`import(store: &dyn CatalogStore, catalog: &CatalogId, document: &AiCatalog, writer: &WriterId, mode: ImportMode) -> Result<Vec<Etag>, ImportError>`** is the one path from a catalog document to store records. `ImportMode::Merge` is the behaviour this bullet describes; `ImportMode::Replace { now }` makes the catalog id hold exactly the document (ADR-017 §4.8). In `Merge` it refuses a signed document or one with top-level members the store cannot hold (§4), gates the host and each entry, then writes the host with `Condition::Any` and each entry as a curated record of `writer` through ADR-017's crate-private `write_owned` (read, then `Condition::Absent` or `Matches`; ADR-017 §4.3, §4.5). It refuses an entry whose key holds a lease (`ImportError::Store(StoreError::ModeConflict)`) or another writer's curated record (`ImportError::OwnedByAnotherWriter`), and reports a record other writers kept changing as `ImportError::Store(StoreError::Contended)`. It is how a curated catalog file (made with the M17 producer commands) is loaded into a store, and how the local server seeds its in-memory store (§7). It is not atomic across records; a failure part-way leaves the records written so far, and re-running it converges. A document with an unversioned and a versioned entry of one identifier fails at the second of them with `ImportError::Store(StoreError::VersionKindConflict)`.
- **Runtime registration** is ADR-017's `Registrar`, over this crate's public API only. Consumers depend on `turul-ai-catalog-storage` and implement nothing of it (workspace boundary, CLAUDE.md).

### 6. Backends and the parity contract

Each backend is one public type with an async constructor taking its own config. No backend creates cloud infrastructure.

| Backend | Type | Storage shape |
|---|---|---|
| `in-memory` | `InMemoryCatalogStore::new()` | `RwLock` over two maps: records `BTreeMap<(CatalogId, RecordKey), Slot>` (bytes, `Etag`, and for an entry its provenance bytes and `ModeKind`) and guards `BTreeMap<(CatalogId, RecordKey), (bool, usize)>` (version kind, entry count); every write holds the one write lock |
| `sqlite` | `SqliteCatalogStore::connect(path)` | table `catalog_records (catalog_id TEXT NOT NULL, record_key TEXT NOT NULL, record BLOB NOT NULL, etag TEXT NOT NULL, PRIMARY KEY (catalog_id, record_key))` and table `catalog_guards (catalog_id TEXT NOT NULL, guard_key TEXT NOT NULL, versioned INTEGER NOT NULL, entries INTEGER NOT NULL, PRIMARY KEY (catalog_id, guard_key))`; both created if missing on connect (a local file the process owns) |
| `postgres` | `PostgresCatalogStore::connect(url)` | the same tables with `record BYTEA`, `versioned BOOLEAN`, `entries BIGINT`; `CREATE TABLE IF NOT EXISTS` on connect, so the role needs that privilege or the tables pre-created |
| `dynamodb` | `DynamoDbCatalogStore::new(client, table_name)` | one table, partition key `pk` (S) = `CatalogId`, sort key `sk` (S) = `RecordKey` (`"h"`, `"e:<hex>"` or `"g:<hex>"`); record items have `record` (B) and `etag` (S); guard items have `versioned` (BOOL) and `entries` (N). Never creates or verifies the table |

- Host puts: SQL uses `INSERT … ON CONFLICT DO NOTHING` for `Absent`, `UPDATE … WHERE etag = $e` for `Matches`, and checks the affected-row count; DynamoDB uses `ConditionExpression` `attribute_not_exists(pk)` for `Absent` and `etag = :e` for `Matches`, mapping `ConditionalCheckFailedException` to `PreconditionFailed`.
- Entry provenance and mode (guarantee 8; ADR-017 §4.6): SQL `catalog_records` has `provenance` and `mode` columns with a `CHECK` that a host row has neither and an entry row has both, and `connect` fails on a table without them; a DynamoDB entry item has `provenance` (B) and `mode` (S), and its replace condition tests `#mode`.
- Entry writes and the version-kind guard (guarantee 6). A guard exists while its identifier has at least one counted entry record; it holds the identifier's kind (`versioned`) and that count (`entries`).
  - SQL: one transaction per `put_entry` and `delete_entry` (sqlite `BEGIN IMMEDIATE`; postgres the default `READ COMMITTED`). Its first statement locks the guard row: `INSERT INTO catalog_guards … VALUES (…, versioned, 0) ON CONFLICT (catalog_id, guard_key) DO UPDATE SET entries = catalog_guards.entries RETURNING versioned, entries`, which inserts or row-locks it until commit, so two writers of one identifier serialise. Then the record's existence and `Condition` decide: `PreconditionFailed`; a replace (`UPDATE`, `entries` unchanged); or a create, refused with `VersionKindConflict` when `entries > 0` and `versioned` differs, otherwise `INSERT` and `entries + 1`. A delete removes the record under its condition and decrements a positive count. Each transaction ends by deleting the guard row when `entries` is 0, so a zero row never outlives the transaction that inserted it.
  - DynamoDB: a create is one `TransactWriteItems` of the record `Put` (`attribute_not_exists(pk)`) and the guard `Update` `SET versioned = :v ADD entries :1` with `attribute_not_exists(pk) OR versioned = :v`. A cancellation whose record item failed is `PreconditionFailed` for `Absent`; for `Any` it is a replace, a `PutItem` with `attribute_exists(pk)`, which never changes the kind (a record deleted meanwhile is created on the next attempt). A cancellation whose guard item failed is `VersionKindConflict`. `Matches` is a `PutItem` with `etag = :e`, unchanged. A delete reads the guard (`ConsistentRead`), then runs one `TransactWriteItems` of the record `Delete` (its condition as before) and, conditioned on the count read, a guard `Delete` (count 1), a guard `Update` `ADD entries :-1` (count above 1), or a `ConditionCheck` `attribute_not_exists(pk)` (no guard). A cancellation for the guard item, or a `TransactionConflict` reason on any item, retries; each retry follows another writer's committed change.
  - DynamoDB retries are bounded: every write loop (create, replace, delete) makes at most 8 attempts, the bound `write_owned` uses above the trait (ADR-017 §4.3), waiting before each retry a uniformly random delay below 20 ms × 2^(retry − 1) (jittered exponential backoff, on the `tokio` timer). Exhaustion is `StoreError::Contended`, which `Registrar::register` reports as `RegistrationError::Contended`. `dynamodb::conflicts_on_every_attempt_are_contended` answers every request with a conflict and asserts 8 attempts, then `Contended`, on the create and replace paths.
  - A record written before the guard existed was never counted: a replace of it adds nothing, and a delete of it decrements only a positive count.
- `list_entries`: SQL `WHERE catalog_id = $1 AND record_key LIKE 'e:%'`; DynamoDB `Query` on `pk = :c AND begins_with(sk, "e:")` with `ConsistentRead`, following `LastEvaluatedKey` to the end. Guards are never listed. `scan_entries` is the same read; a provenance that does not parse is returned in its `ScannedEntry` instead of failing the read.
- `Condition::Overwrite(expected)` (guarantee 8's exemption). In-memory: under the write lock, the slot's `Etag` is compared with `expected` and the slot replaced. SQL: inside the existing per-identifier transaction, after the guard row lock, only the row's `etag` is read (`SELECT etag`); a missing row or another `etag` is `PreconditionFailed`, otherwise one `UPDATE` sets every column (`record`, `provenance`, `mode`, `etag`), so no other previous value is read and none survives. DynamoDB: a `PutItem` conditioned only on `etag = :expected`, which fails on a missing item as on another version; `PutItem` replaces the whole item, so no attribute of the old item survives or is read, and the guard count is untouched. A `TransactionConflictException` is retried with the bounded loop of every other write (8 attempts, then `Contended`).
- `count_catalog` and `purge_catalog` (guarantee 9):
  - In-memory: under the write lock, every key of the catalog in both maps is counted, and for a purge removed.
  - SQL: one transaction (sqlite `BEGIN IMMEDIATE`; postgres the default `READ COMMITTED`) of `DELETE FROM catalog_records WHERE catalog_id = $1 RETURNING record_key` and `DELETE FROM catalog_guards WHERE catalog_id = $1 RETURNING guard_key`; the returned keys are counted by kind, so the report counts exactly the rows deleted. `count_catalog` runs the same two statements as `SELECT`.
  - DynamoDB: `Query` on `pk = :c` with `ConsistentRead` and `ProjectionExpression` `pk, sk`, so no record, provenance or guard attribute is fetched, following `LastEvaluatedKey` to the end. A purge deletes each item of each page with `DeleteItem` by key, conditioned on `attribute_exists(pk)`: an item another writer removed in between fails the condition, is not counted and is not an error. A `TransactionConflict` (the item is inside another writer's transaction) or a throttling error retries the item under the bound of every write loop (at most 8 attempts, the same jittered backoff), then `Contended`; the SDK's own retries run inside each attempt. Not atomic across items: a failure stops the purge with the error, the items deleted so far stay deleted, and a re-run removes the rest. `count_catalog` is the same `Query` without deletes.
- Record size: DynamoDB refuses items over 400 KB, which an entry with large inline `data` can exceed; that is `StoreError::Backend` naming the limit. No backend caps size below its native limit.

The parity contract is `tests/parity/mod.rs`: `pub async fn` checks taking `&dyn CatalogStore` (and `Arc<dyn CatalogStore>` for concurrency), run by `in_memory.rs` and `sqlite.rs` in CI, and by `postgres.rs`, which owns its environment: it uses `TURUL_AI_CATALOG_POSTGRES_URL` when set, otherwise finds `initdb`, `pg_ctl` and `createdb` (`POSTGRES_BIN`, then `PATH`, then `/opt/homebrew/opt/postgresql@17/bin`), runs `initdb -A trust -U postgres --no-locale -E UTF8` in a temporary directory and starts one server for the test binary with `pg_ctl -w` on a free port (socket directory in the temporary directory, `listen_addresses=127.0.0.1`). Each test creates its own uniquely named database and connects to `postgres://postgres@127.0.0.1:<port>/<db>`; the last drop guard runs `pg_ctl stop -m fast` and removes the temporary directory, also on panic. When neither the URL nor the binaries are available each test prints its skip reason and returns; a server that fails to start fails the test. `dynamodb.rs` owns its environment: it uses `TURUL_AI_CATALOG_DYNAMODB_ENDPOINT` when set, otherwise starts DynamoDB Local (`DYNAMODB_LOCAL_JAR`, default `~/.cache/dynamodb-local/DynamoDBLocal.jar`; java from `JAVA_HOME`, `PATH` or `/opt/homebrew/opt/openjdk/bin/java`; `-inMemory`, a free port, a temporary working directory so its metadata file never lands in the repository) and waits until it answers. Each test creates a uniquely named table (`pk` S, `sk` S) through the SDK with dummy credentials; the store never creates it. A drop guard deletes the table and stops the process, also on panic. When neither an endpoint nor a jar and java are available each test prints its skip reason and returns; a started server that never answers fails the test. The tests are `#[ignore]` so a plain `cargo test` stays fast, and run with `--include-ignored`. Each backend file calls every check; a check added to the module and not called fails review, not silently.

### 7. The Lambda (supersedes M2's bundle; changes ADR-010)

- **Serving.** `main` builds the store, then `ServedCatalog::assemble(store: &dyn CatalogStore, catalog: &CatalogId, now: SystemTime) -> Result<ServedCatalog, ColdStartError>`, which calls `assemble` and holds the bytes. An `AssembleError` fails cold start exactly as an invalid bundle did. No `put_entry` through this crate's backends can store a #multi-version-entries conflict (§3 guarantee 6); a store written before the guard existed that holds one fails assembly (§4) and so fails init until the record is removed. `handle(&ServedCatalog, &Request)` stays pure and unchanged in routing, status codes, headers and `Content-Type`; the GET body is the assembled bytes.
- **Freshness.** `SnapshotCache` (lambda lib) holds an `Arc<ServedCatalog>`, the instant it was last assembled or attempted, and an injected clock. On a request, when its age is at least `max_age` or the clock has reached the snapshot's `next_expiry` (ADR-017 §6), `main` re-assembles before calling `handle`; a failed re-assembly keeps serving the last good snapshot, and `SnapshotCache::current` returns the failure beside it; `main` writes it to stderr (CloudWatch Logs on Lambda). A failed refresh restarts the snapshot's age, so while the failure lasts the store is read and the failure logged once per `max_age` per instance, not on every request (`snapshot_refresh::failed_refresh_is_retried_once_per_max_age`). `max_age` is `TURUL_AI_CATALOG_MAX_AGE_SECS`, default 60. The clock lives in `SnapshotCache`, never in `handle`. Trade-off: a registration becomes visible within `max_age` per warm instance, and each refresh is one `GetItem` and one `Query` per instance; cold-start-only assembly would make visibility depend on instance recycling, which defeats M19's runtime registration. `0` re-assembles on every request.
- **Backend selection.** Built with the lambda's `dynamodb` feature (forwarding `turul-ai-catalog-storage/dynamodb`), `main` requires `TURUL_AI_CATALOG_TABLE` and `TURUL_AI_CATALOG_ID` and fails cold start naming a missing one, so a deployed function never serves an empty in-memory store by accident. Built without it, `main` uses an empty `InMemoryCatalogStore` and `TURUL_AI_CATALOG_ID` defaults to `default`; `cargo lambda watch` keeps working and serves the empty valid catalog.
- **Local server (M18).** `turul-ai-catalog-serve --file PATH` parses the file, `import`s it into an `InMemoryCatalogStore`, and serves through `ServedCatalog::assemble` and `handle`. Without `--file` it serves the empty store. It remains the only code in the crate that reads a file or binds a port.
- **IAM.** The deploying repo owns the table, the function and every grant. The serving role needs `dynamodb:GetItem` and `dynamodb:Query` on the table only; writers need `dynamodb:PutItem`, `dynamodb:UpdateItem`, `dynamodb:DeleteItem`, `dynamodb:ConditionCheckItem` (the actions inside `TransactWriteItems`) and `dynamodb:GetItem` (the guard read before a delete). The operator's store commands (`store list`, `store import`, `store purge`, ADR-017 §8) also need `dynamodb:Query`, which `store list` already needs; `purge_catalog` uses only `Query` and `DeleteItem`, and `Condition::Overwrite` only `PutItem`. This repo ships no IAM, SAM or CDK (M2's "handler only" stands).

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
| Version-kind guard key | `RecordKey::guard(&EntryIdentifier)` | every backend's `put_entry` and `delete_entry` |
| Record version | host `Etag::of(&[u8])`; entry `Etag::of_entry` | every backend, via the crate |
| Entry record bytes | `EntryRecord::new` (`validate`, then `Json::to_canonical_vec` of `CatalogEntry::to_json`) | `put_entry` |
| Host record bytes | `HostRecord::new` | `put_host` |
| Record reading | `parse::parse_entry`, `parse::parse_host` (ADR-002) | every backend's reads |
| Store contract | `trait CatalogStore` | four backend types |
| Served document | `assemble` | `ServedCatalog::assemble` |
| Document → records | `import` (`ImportMode::Merge` through ADR-017's `write_owned`; `ImportMode::Replace` through `Condition::Overwrite`, ADR-017 §4.8) | curated loads, `turul-ai-catalog-serve`, `turul-ai-catalog store import` |
| Overwrite at a version regardless of stored provenance | `Condition::Overwrite(expected)` on `put_entry` | `import` in `ImportMode::Replace` |
| Entry listing that tolerates unreadable provenance | `CatalogStore::scan_entries` | `import` in `ImportMode::Replace` |
| Removal of every item of a catalog id | `CatalogStore::purge_catalog` (preview: `count_catalog`) | `turul-ai-catalog store purge` |
| Served snapshot freshness | `turul_ai_catalog_lambda::SnapshotCache` | Lambda `main` |

Schema digests: record bytes are ADR-003 JCS of ADR-002's member model, whose digest is ADR-003's; `RecordKey` and `Etag` are SHA-256 over JCS and record bytes. No new schema. Determinism is proved by `assembly.rs` and the parity check `same_state_and_now_assemble_to_the_same_bytes`.

## Requirement → test

Paths are under `crates/turul-ai-catalog-storage/tests/` unless stated. `parity::x` runs on every backend.

| Requirement | Spec text (short) | Test function |
|---|---|---|
| multi-version-entries-1 | `identifier` + `version` "MUST be unique" | `parity::one_record_per_entry_key` (put twice with `Any` keeps one; versions `1` and `2` of one identifier are two records; an unversioned `b` put twice is one record; the three assemble without error), `parity::same_version_kind_puts_are_accepted`, `record_keys::absent_and_empty_version_are_distinct_keys`, `record_keys::distinct_entry_keys_give_distinct_record_keys` |
| multi-version-entries-2 | absent `version`: `identifier` alone "MUST be unique" | `parity::unversioned_entry_beside_versioned_ones_is_refused`, `parity::versioned_entry_beside_an_unversioned_one_is_refused` (`1` and `""`), `parity::deleting_the_last_record_of_a_kind_frees_the_identifier`, `parity::conditional_puts_keep_their_preconditions`, `parity::racing_version_kinds_leave_one_kind` (16 writers racing `a` against `a@k`, four rounds, end with records of one kind), `record_keys::guard_keys_are_one_per_identifier_and_never_an_entry_or_host_key`; defence in depth for stores written before the guard: `assembly::cross_entry_error_still_fails_assembly`, `cold_start::invalid_store_fails_cold_start`, `snapshot_refresh::cross_entry_error_keeps_the_last_good_snapshot_and_is_reported` (lambda crate) |
| top-level-structure-1 | `specVersion` and `entries` required; `entries` "MAY be empty" | `assembly::empty_store_assembles_to_a_valid_empty_catalog` |
| host-info-1 | Host Info "MUST contain" `displayName` | `records::host_record_refuses_an_invalid_host` (`HostInfo` cannot be built without `displayName`, so the gate's refusals are `validate` errors and non-I-JSON; the test asserts both the refusal and that stored host bytes carry `displayName`) |
| catalog-entry-1 | required members; exactly one of `url` or `data` | `records::entry_record_refuses_an_entry_with_an_error_diagnostic` |
| catalog-entry-2 / catalog-projection-1 | publisher-authorized `urn:air` identifier "MUST preserve … exactly" | `parity::stored_entry_reads_back_byte_identical` (identifier, extensions and unrecognized members kept) |
| catalog-projection-2 | retained source identifiers "do not participate in catalog uniqueness" | `record_keys::extensions_do_not_change_the_record_key` |
| catalog-snapshot-coverage-1 | changing a contribution "requires a new catalog signature" | `import::signed_document_is_refused`, `assembly::assembled_document_has_no_signature` |
| signed-payload-1 | manifest payload is JCS of the manifest | `parity::entry_trust_manifest_payload_survives_storage` (the JCS of each `trustManifests[i]` minus `signature.jws` is equal before and after a round trip) |
| nested-catalogs-1 | each sub-catalog is "an independent AI Catalog document" | `parity::catalogs_are_isolated` |

Keyword sentences in the cited sections owned elsewhere or out of scope:

| Requirement | Owner |
|---|---|
| catalog-projection-3 (MUST NOT infer authorization from URL or unsigned `publisher`) | ADR-017 §3.1 and §3.3 (only `identifier` is checked); verification is ADR-015 |
| catalog-projection-4 (operator-assigned identifier MUST remain stable, MUST NOT be reassigned) | ADR-017 §3.3 and AMB-424 (ownership per record key); the store holds no identifier history |
| multi-version-entries-3 (clients SHOULD sort by `version` or `updatedAt` for latest) | client, ADR-007 |
| well-known-uri-1 (hosts MAY serve at the well-known URI) | ADR-010, `well_known_handler.rs` |
| entry-release-coverage (version or digest change invalidates a manifest) | ADR-015 |
| dynamic-discovery-1 (out of scope) | OOS: statement |

Maintainer decisions and ambiguities owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| M19 (ETag, conditional writes) | `Absent`, `Matches`, `Any`; precondition failures typed | `parity::absent_condition_refuses_an_existing_record`, `parity::matches_condition_refuses_a_stale_etag`, `parity::delete_with_a_stale_etag_is_refused`, `parity::delete_of_a_missing_record_is_not_found` |
| M19 (atomicity) | of concurrent `Absent` puts exactly one wins | `parity::concurrent_absent_puts_have_one_winner` |
| M21 | `put_entry` refuses a new entry beside entries of the other version kind, atomically on every backend | multi-version-entries-2 above, chiefly `parity::racing_version_kinds_leave_one_kind` |
| M19 (read-your-writes, completeness) | a returned write is visible; listing is complete across pages | `parity::writes_are_visible_to_the_next_read`, `parity::list_returns_every_entry` (300 entries; `dynamodb.rs` also runs it with more than 1 MB of entries, so `Query` returns several pages) |
| M19 (parity) | equal state, equal bytes on every backend | `parity::same_state_and_now_assemble_to_the_same_bytes` (asserts against a committed expected byte string, so backends are compared through it) |
| M6 | core crate unchanged in dependencies; storage default adds no runtime or network | `dependency_direction::storage_default_features_add_no_runtime_or_network_dependency` (CLI crate) |
| M2 / M19 (Lambda) | the Lambda serves the assembled document | ADR-010's migrated `well_known_handler.rs`, `cold_start::invalid_store_fails_cold_start`, `cold_start::missing_table_configuration_fails_cold_start` (feature `dynamodb`) |
| AMB-401 | the record gate is the only constructor | `records::entry_record_refuses_non_i_json`, `records::entry_record_bytes_are_canonical` |
| AMB-402 | `Etag` is content-derived and backend-independent | `parity::etag_is_the_digest_of_entry_and_provenance`, `parity::recreated_identical_record_has_the_same_etag` |
| error parity | a stored mode that is missing or unknown, and a put's provenance that would not read back, are `CorruptProvenance` on every backend | `parity::provenance_that_would_not_read_back_is_refused`; `parity::unknown_stored_mode_is_corrupt_provenance`, run by `sqlite.rs`, `postgres.rs` and `dynamodb.rs` after writing such a row raw (the in-memory store holds a typed mode and cannot hold one) |
| bounded retries | DynamoDB write loops stop after 8 attempts with `Contended` | `dynamodb::conflicts_on_every_attempt_are_contended` (create, replace, `Overwrite` and purge-delete paths) |
| M30 purge | every item of the id removed, counted by kind; zero for an id with no items | `parity::purge_removes_every_item_of_the_catalog` (host, curated and leased entries of both version kinds, their guards; afterwards `count_catalog` is zero, `assemble` gives the empty catalog, `get_host` is `None`, and the other version kind of a purged identifier can be created; 300 entries, and `dynamodb.rs` also runs it with more than 1 MB of items so the `Query` of a scan, a count and a purge returns several pages), `parity::purge_of_an_empty_catalog_removes_nothing`, `parity::count_catalog_is_what_purge_removes` |
| M30 purge isolation | a purge changes no other catalog | `parity::purge_leaves_other_catalogs_untouched` (the other catalog's records, `Etag`s and assembled bytes unchanged) |
| M30 purge without reading | items this release cannot read are counted and removed | `parity::unreadable_items_are_counted_and_purged`, run by `sqlite.rs`, `postgres.rs` and `dynamodb.rs` after writing raw an entry whose provenance does not parse (DynamoDB also: an entry item without `provenance` or `mode`, and an item whose `sk` has no known prefix, counted as `other`); the in-memory store holds typed provenance and cannot hold one |
| M30 overwrite | `Condition::Overwrite(expected)` replaces a record of either mode, or one whose provenance does not read, at the expected version; a missing key or another version is `PreconditionFailed` | `parity::overwrite_replaces_a_record_of_either_mode` (over a lease and over another writer's curated record, each at the version read before: `Ok`, `Etag::of_entry` of the new pair, guard count unchanged, so deleting the record frees the identifier), `parity::overwrite_requires_the_stored_version` (a missing key and a stale version: `PreconditionFailed`, nothing created, the stored record unchanged), `parity::overwrite_of_a_host_requires_its_version` (a missing host and a stale version: `PreconditionFailed`; the stored version: replaced), `parity::unreadable_items_are_scanned_and_overwritten` |
| M30 scan | unreadable provenance is a value, not a failure | `parity::scan_entries_matches_list_entries` (same keys and `Etag`s, every provenance `Ok`), `parity::unreadable_items_are_scanned_and_overwritten` (`Err(ProvenanceError)` for the raw item, `Ok` for the others) |
| feature hygiene | the crate builds without default features | CI `cargo clippy -p turul-ai-catalog-storage --no-default-features --all-targets -- -D warnings` |
| AMB-404 | entries are ordered by `EntryKey` | `assembly::entries_are_ordered_by_entry_key` (insertion order reversed, absent version before present) |
| AMB-405 | top-level members the store cannot hold are refused on import | `import::top_level_extensions_are_refused`, `import::unrecognized_top_level_members_are_refused` |
| AMB-407 | a failed refresh keeps the last good snapshot and reports the failure | `snapshot_refresh::stale_snapshot_is_reassembled`, `snapshot_refresh::failed_reassembly_keeps_the_last_good_snapshot` (the store's reads fail, then recover), `snapshot_refresh::cross_entry_error_keeps_the_last_good_snapshot_and_is_reported` (a store written before the guard), `snapshot_refresh::failed_refresh_is_retried_once_per_max_age`, `snapshot_refresh::fresh_snapshot_is_not_reassembled` (lambda crate) |

New ids, block AMB-400 to AMB-419:

- AMB-401: M19 asks that the served document go through `validate`; an entry that cannot pass alone would make every later assembly fail and the Lambda stop refreshing. Decision: gate at write time per record (entry-local rules) and again at assembly (the whole document). The one cross-entry error a store could hold, #multi-version-entries' identifier-alone rule, passes the record gate and is refused by every backend's `put_entry` (§3 guarantee 6, M21); assembly still validates the whole, for stores written before the guard (§4).
- AMB-402: a counter version per record allows ABA after delete and re-create with different content. A content digest makes a match mean "the bytes I read", needs no clock or counter in any backend, and is equal across backends. Trade-off: `sha2` is used directly here; `turul-ai-catalog-trust` also computes SHA-256 for #digest-format, which is a different artifact (an artifact digest, not a store version), and the storage crate must not depend on the trust crate (§1).
- AMB-403: the brief lists `assemble` among the store operations. A trait method with a default body can be overridden per backend, making a second document builder. Decision: a free function over `&dyn CatalogStore`.
- AMB-404: the spec fixes no entry order for an unsigned catalog; it fixes "every entry in order" only for a signature, which assembly never carries. `EntryKey`'s derived `Ord` is total, deterministic and backend-independent; insertion time is not, and would need a clock. Clients choose latest versions themselves (#multi-version-entries).
- AMB-405: silent loss of a top-level member would change what the operator published. Refusing keeps the store's content an exact subset of the spec document. Storing catalog-level `extensions` waits for a writer that needs them.
- AMB-406: `produce` stays file-only; see §5.
- AMB-407: a refresh failure (store unreachable, or a document that fails `validate`, which only a store written before the guard can produce) serving a 5xx would make discovery depend on store health. Serving the last validated snapshot keeps every response a document that passed `validate`. Trade-off: a warm instance serves a snapshot that no longer matches the store for as long as the failure lasts, and new instances fail init on a document that fails `validate`. The failure is returned by `SnapshotCache::current` and written to stderr by `main`, so the deploying repo's log alarms detect it; this repo adds no alarm, metric or health check.

## Consequences

- The Lambda's content comes from a store the deploying repo provisions; the repo builds and tests the handler with the in-memory backend and needs no AWS account for CI.
- The well-known document can no longer carry a catalog-level `signature`. A signed catalog is published as a fixed file elsewhere and referenced by URL.
- Served bytes are canonical JCS, not a hand-formatted file.
- `bundle_provenance` and the default Acme bundle go: a deployment's content is its store, loaded by `import` or by registering writers.
- Assembly costs one host read and one listing per refresh per warm instance. Catalogs with very many entries make each refresh proportionally larger; paging the served document is not defined by the spec and is not built.
- Consumers that register entries at runtime do so through ADR-017's `Registrar`.
- A writer can no longer make the catalog unservable by breaking #multi-version-entries: every backend refuses the put (§3 guarantee 6). Stores written by 0.0.17 hold no guards: `put_entry` cannot see their records' kinds and a conflict they already hold stays, and assembly keeps refusing it (§4). Re-running `import`, or the registering writers, into an empty catalog or table gives them the guarantee; nothing migrates them.
- Every entry write is a transaction over two items (record and guard), and a DynamoDB delete adds one consistent guard read. SQL deployments gain the `catalog_guards` table; DynamoDB writers gain `UpdateItem`, `ConditionCheckItem` and `GetItem` (§7).

## Closed question: runtime self-registration or curated deployment

Closed by M26 (2026-11-17) and decided in ADR-017: both. A curated catalog is written by the producer commands, loaded by `import` and recorded `Curated`; services register leases (or, when granted, curated entries) through the `Registrar`, under a per-writer grant, with expiry applied by `assemble`. The trait, records and keys, assembly order, the Lambda and the backends of this ADR hold for both, with ADR-017's provenance and guarantee 8 added.

## Closed question: version-group conflicts at write time

Closed by M21 (2026-11-17): `put_entry` refuses an entry whose identifier already has stored records of the other version kind, atomically in every backend, through a per-identifier guard (§3 guarantee 6, §6). The documented availability risk is removed for every store written from 0.0.18; assembly keeps validating the whole document for stores written before.

## What this ADR does not do

- No code, tests or `Cargo.toml` changes: they follow in the implementing slice, which also updates ADR-002, ADR-003, ADR-004, ADR-010 and ADR-011 and their revision logs, `CHANGELOG.md` and the version.
- No new maintainer decision: it implements M19 as recorded in `docs/plan/DECISIONS.md`.
- No signing of assembled catalogs, no infrastructure.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created under M19: crate and features, `CatalogId`, `EntryRecord`/`HostRecord` gate, `RecordKey`, content-derived `Etag`, `CatalogStore` trait and backend guarantees, `assemble`, `import`, backend schemas including the DynamoDB `pk`/`sk` shape, parity contract, Lambda changes to ADR-010. AMB-401 to AMB-407 added. |
| 2026-11-17 | host-info-1 test scope: `displayName` is required by the `HostInfo` type, and `validate` raises no host-scoped error for it, so the test covers the non-I-JSON refusal and the stored bytes. |
| 2026-11-17 | sqlite backend: the `sqlite` feature adds `sqlx` only (`runtime-tokio` supplies tokio), so `tokio` is a dev-dependency of the parity tests, not a feature dependency. |
| 2026-11-17 | postgres backend: `sqlx` is a root dependency with `runtime-tokio` only; the `sqlite` and `postgres` features each enable their own driver (`postgres` adds `tls-rustls`). Parity check catalog ids carry a per-run suffix so a re-run against a persistent database starts clean. |
| 2026-11-17 | dynamodb backend: parity tests own their environment (endpoint variable or a spawned DynamoDB Local with a per-test table and drop guard) instead of `TURUL_AI_CATALOG_DYNAMODB_TABLE`; the crate depends on `aws-sdk-dynamodb` only, with the default https client; `DynamoDbCatalogStore::new` takes a built `Client`, so `aws-config` belongs to the deploying binary. |
| 2026-11-17 | Writers and the Lambda: `produce` stays file-only and `Change::Remove` carries an `EntryKey`; the Lambda's `dynamodb` feature adds `aws-config` and `aws-sdk-dynamodb` (the deploying binary builds the `Client`, §1); `ServedCatalog::assemble`, `SnapshotCache`, `catalog_id`, `max_age` and `table_name` are as §7; ADR-010 and ADR-011 updated, `catalog.json`, `BUNDLE` and `bundle_provenance.rs` deleted. |
| 2026-11-17 | §3 guarantee 6 corrected against #multi-version-entries: key uniqueness enforces the `(identifier, version)` rule only; "`identifier` alone MUST be unique" spans records and is enforced by `validate` in `assemble`, so a conflicting put is accepted and refused at assembly, which fails every cold start. `parity::one_record_per_entry_key` no longer stores that spec-invalid state; multi-version-entries-2 maps to the assembly, cold-start and refresh tests that prove the actual behaviour. AMB-401, AMB-407, §7 and Consequences state the availability trade-off; `SnapshotCache::current` returns a refresh failure and `main` logs it. The write-time check is put to the maintainer (open question). `dynamodb.rs` runs DynamoDB Local in a temporary working directory, and its metadata file is git-ignored. |
| 2026-11-17 | Pre-push review: a failed refresh now restarts the snapshot's age, so retries and log lines run once per `max_age` instead of once per request (probe: 1000 requests within `max_age` had made 1000 `list_entries` calls). Pinned by `snapshot_refresh::failed_refresh_is_retried_once_per_max_age`. |
| 2026-11-17 | M21: §3 guarantee 6 now holds for both #multi-version-entries rules. `put_entry` refuses a new entry beside entries of the other version kind with `StoreError::VersionKindConflict { identifier }`, atomically: in-memory under its lock, sqlite and postgres in one transaction locking a `catalog_guards` row, DynamoDB in one `TransactWriteItems` with a `"g:"` guard item (`RecordKey::guard`, the entry-key hash construction over `[identifier]`) holding `versioned` and `entries`; `delete_entry` maintains the guard, so the last delete of a kind frees the identifier. New parity checks for both directions, same-kind puts, freeing, preconditions and racing writers; multi-version-entries-2 remaps to them. `assemble` keeps whole-document validation as defence in depth for stores written before the guard; the cold-start tests and `snapshot_refresh::cross_entry_error_keeps_the_last_good_snapshot_and_is_reported` take such records from a store that lists them, since 0.0.17 stores can still hold them, and the refresh test also asserts the put is refused. Writer IAM gains `UpdateItem`, `ConditionCheckItem` and `GetItem`. The open question is closed. Status stays Proposed: the postgres parity suite has not run against a live service. |
| 2026-11-17 | `tests/postgres.rs` owns its environment like `tests/dynamodb.rs`: `TURUL_AI_CATALOG_POSTGRES_URL` when set, otherwise a temporary `initdb` cluster served by `pg_ctl` for the test binary, one database per test, stopped and deleted by the last drop guard; skips when no URL or binaries. The postgres parity suite now runs against a live server. |
| 2026-11-17 | `import` now runs `validate` on the whole document first and refuses it (`ImportError::Invalid`, listing each error as `code at pointer`) before writing anything. Before this, two entries with one entry key collapsed silently into one record, and the assembled catalog passed with an entry missing. CI now runs the storage tests with the `sqlite` feature, which the acceptance clause requires. Pinned by `import::duplicate_entries_are_refused_and_nothing_is_written`. |
| 2026-11-17 | ADR-017 R1: `put_entry` takes a `Provenance`; `get_entry` and `list_entries` return `StoredEntry`; an entry's `Etag` is `Etag::of_entry` and covers provenance; guarantee 8 (mode exclusion) in all four backends with provenance and mode stored beside the record bytes (SQL columns with a `CHECK`, DynamoDB attributes tested in the write condition); `import` takes a `WriterId`, writes curated and refuses another writer's or a lease's record. `parity::etag_is_the_digest_of_the_record_bytes` is replaced by `parity::etag_is_the_digest_of_entry_and_provenance` for entries. |
| 2026-11-17 | ADR-017 R2: `assemble` takes `now`, leaves expired leases and entries failing the current gate out of the document and reports each in `Assembled::omitted` with its reason, and returns `next_expiry`; `SnapshotCache` takes a clock and refreshes at the earlier of `max_age` and `next_expiry`. The parity check `same_state_assembles_to_the_same_bytes` is renamed `same_state_and_now_assemble_to_the_same_bytes` and also covers a live and an expired lease. |
| 2026-11-17 | Registration review: the trait section states that lifecycle and expiry are ADR-017's and that a missing or unknown stored mode is `CorruptProvenance` on every backend (the SQL backends returned `Backend` text); `StoreError` lists `ModeConflict`, `CorruptProvenance` and the new `Contended`; a put refuses a provenance whose bytes would not read back (an expiry past year 9999) with `CorruptProvenance` before touching the store; `import` is curated writes through `write_owned` with its refusals, and maps exhausted retries to `StoreError::Contended` (was `PreconditionFailed`); DynamoDB write loops are bounded at 8 attempts with jittered exponential backoff, then `Contended`, and the `dynamodb` feature adds `tokio` (`time`) for the timer; the renamed `assembly::cross_entry_error_still_fails_assembly` replaces `records_breaking_identifier_uniqueness_fail_assembly` in the requirement table; the F1 rows and the open question point at ADR-017; expiry and CLI store commands leave "What this ADR does not do". New: `parity::provenance_that_would_not_read_back_is_refused`, `parity::unknown_stored_mode_is_corrupt_provenance`, `dynamodb::conflicts_on_every_attempt_are_contended`, and a CI clippy step without default features. |
| 2026-11-18 | M30: `CatalogStore` gains `purge_catalog` (returns `PurgeReport { removed: ItemCounts }`) and its preview `count_catalog`, which count and remove every item of a catalog id by key alone (host, entries, guards, any other key) without reading record bytes, provenance or guard values: in-memory under its lock, SQL as `DELETE … RETURNING` across `catalog_records` and `catalog_guards` in one transaction, DynamoDB as a paginated key-only `Query` and per-item `DeleteItem` with the bounded retry of every write loop, not atomic, re-run converges. Guarantee 9 states it. For `ImportMode::Replace` (ADR-017 §4.8) the trait also gains `Condition::Overwrite`, exempt from guarantee 8, which replaces a record of either mode or with unreadable provenance without reading it (SQL `UPDATE` of every column, DynamoDB `PutItem` conditioned on existence only), and `scan_entries`, which returns unreadable provenance per entry instead of failing. `import` takes an `ImportMode`. Operator commands need `dynamodb:Query`, already needed by `store list`. New parity checks for purge, overwrite and scan; `dynamodb::conflicts_on_every_attempt_are_contended` covers the new write paths. |
| 2026-11-18 | M31: `Condition::Overwrite` carries the expected `Etag`: it replaces a record of any mode or readability only at that stored version, and a missing key or another version is `PreconditionFailed`, as `Matches`. It no longer creates. Without this, `ImportMode::Replace` overwrote a lease registered or refreshed between its scan and its write. DynamoDB's overwrite is one `PutItem` conditioned on `etag`, with no create path and no guard update. `parity::overwrite_of_a_missing_key_creates_as_any` is replaced by `parity::overwrite_requires_the_stored_version`; `parity::overwrite_of_a_host_requires_its_version` pins the host form; `dynamodb::conflicts_on_every_attempt_are_contended` now covers the overwrite path, as this ADR already claimed. |
