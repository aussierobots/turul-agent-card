# ADR-017: Runtime registration: writers, authority, lease and curated modes

- **Status:** Proposed (accepted when every test file below is green, the `parity` registration checks for `in-memory` and `sqlite` in CI and for `postgres` and `dynamodb` against a live service; every entry write in the workspace carries a `Provenance`; no `put_entry` call site, `import` or `ServedCatalog::from_document` writes an entry without one; and `assemble` is the only code that decides whether a stored lease is served)
- **Date:** 2026-11-17
- **Implements:** under `crates/turul-ai-catalog-storage/tests/`: `provenance.rs`, `registration_policy.rs`, `registration.rs`, `operator.rs`, new checks in `parity/mod.rs` called by `in_memory.rs`, `sqlite.rs`, `postgres.rs` and `dynamodb.rs`, and migrated `assembly.rs`, `import.rs`, `records.rs`. Under `crates/turul-ai-catalog-trust/tests/`: `publisher_verifier.rs` (feature `registration`). Under `crates/turul-ai-catalog-lambda/tests/`: migrated `snapshot_refresh.rs`, `cold_start.rs`. Under `crates/turul-ai-catalog-cli/tests/`: `store_commands.rs`. `crates/turul-ai-catalog-cli/tests/dependency_direction.rs` gains `trust_optional_features_add_only_storage_async_trait_tokio_and_the_ssm_client` and keeps `storage_default_features_add_no_runtime_or_network_dependency`.
- **Depends on:** ADR-016 (`CatalogStore`, `CatalogId`, `EntryRecord`, `RecordKey`, `Etag`, `Condition`, `StoreError`, guarantees 1 to 7, `assemble`, `import`, the four backends, the parity suite, `SnapshotCache`; changed by §4 to §7), ADR-015 (`TrustPolicy`, `verify_signature`, `entry_release_coverage`, `publisher_authorized`, `ManifestReport::endorsement`; gains `verify_entry`, §5), ADR-004 (`UrnAir`, `EntryKey`, `EntryIdentifier`), ADR-005 (`validate`, `Timestamp`), ADR-010 (the Lambda), ADR-012 (`DidWebRoot`, whose domain check moves to the core crate, §3.1). Binding decisions: **M26** (this ADR's mandate), **M30** (`import --replace`, `store purge`, the upgrade routes of §4.7; fail forward, no read of an older stored format), M19, M21, M24, M10, M6, M13.
- **Spec:** `target/upstream/ai-catalog/specification/ai-catalog.md` (last commit 2026-11-10).

## Context

M26 approves runtime registration: an opt-in registration API over `CatalogStore` with a per-writer `urn:air` publisher allow-list that refuses when none is configured, an optional signed-Trust-Manifest requirement, two entry modes (a lease that expires, and curated) each refusing writes from the other, and assembly that reports what it left out. ADR-016 left exactly these open: who the writer is, under what authority, and lifecycle (refresh, removal, expiry).

Design input recorded in `docs/plan/TODO.md` F1 came from one operator; it is input, not decision. This ADR decides from the spec text below. It names no consumer: the library serves any deployer.

Spec text this ADR implements:

- #catalog-projection: "A publisher-authorized `urn:air` identifier is one assigned by the artifact publisher or its authorized delegate, with the artifact publisher's domain in the `{publisher}` segment." Rules: (1) a publisher-authorized `urn:air` source identifier "the operator MUST preserve it exactly"; (3) an operator "that is neither the artifact publisher nor its authorized delegate MUST use a non-`urn:air` identifier under its own control". "Operating a catalog or aggregating an entry does not make its operator the artifact publisher. A catalog operator MUST NOT infer publisher authorization from the artifact's URL or an unsigned `publisher` field." An operator-assigned identifier "MUST remain stable across versions, source-coordinate changes, and retrieval-URL changes, and MUST NOT be reassigned to another artifact." "The catalog operator and artifact publisher are distinct roles."
- #catalog-entry: "A catalog operator incorporating an entry for the first time with a publisher-authorized `urn:air` identifier MUST preserve that identifier exactly." "Identifier syntax alone does not verify publisher identity or establish trust."
- #multi-version-entries: "When `version` is present, the combination of `identifier` and `version` MUST be unique within the catalog. When `version` is absent, `identifier` alone MUST be unique. The `identifier` SHOULD be stable across versions."
- #the-did-web-publisher-profile: "The entry MUST use the standard `urn:air` syntax." The `{publisher}` component "MUST satisfy the root `did:web` domain restrictions"; "The authenticated signer identity MUST exactly match `did:web:` followed by that component. Subdomain, suffix, and organizational-ownership heuristics MUST NOT be used." "The signature MUST satisfy Entry Release Coverage, including … verification of the artifact bytes against `entry.digest`." "A valid signature by an assessor … does not satisfy publisher authorization."
- #signature-acceptance-and-time: "Consumers MUST NOT accept an endorsement before `issuedAt` or at or after `expiresAt`"; "An invalid, unsupported, or expired signature MUST NOT count as a verified endorsement."
- #trust-manifest / #entry-release-coverage: a Trust Manifest rides on the entry (`trustManifests`); coverage requires `signer` = `contributor`, the subject equal to the entry (including `digest`, which "The entry MUST contain"), and verified artifact bytes.
- #catalog-snapshot-coverage: "Adding, removing, or changing a contribution therefore changes the catalog snapshot and requires a new catalog signature."
- #dynamic-discovery: dynamic generation "based on a caller's identity or query parameters" is out of scope.

The spec defines no registration protocol, writer identity, lease or expiry. It constrains identity and authorization only, and neither requires nor forbids runtime registration. Everything below that is not quoted above is this library's choice, with its reason.

## Decision

### 1. API shape: a `Registrar` over the store

```rust
pub struct Registrar { /* store, catalog, policy, verifier */ }

impl Registrar {
    pub fn new(
        store: Arc<dyn CatalogStore>,
        catalog: CatalogId,
        policy: RegistrationPolicy,
        verifier: Option<Arc<dyn PublisherVerifier>>,
    ) -> Result<Registrar, PolicyError>;

    pub async fn register(&self, writer: &WriterId, entry: CatalogEntry, mode: ModeRequest, now: SystemTime)
        -> Result<Registered, RegistrationError>;
    pub async fn unregister(&self, writer: &WriterId, key: &EntryKey)
        -> Result<(), RegistrationError>;
}

pub enum ModeRequest { Lease, Curated }
pub struct Registered { pub etag: Etag, pub provenance: Provenance, pub written: bool }
```

- `Registrar` lives in `turul-ai-catalog-storage` (module `registration`), in the default feature set: it adds no dependency (§5 keeps crypto out).
- Authority, policy, clocks and verification are in the `Registrar`, never in `CatalogStore`. The trait stays storage: it gains one stored value per entry (`Provenance`, §4) and one storage invariant (guarantee 8, §4), the same kind of change M21 made for version kinds. An atomic cross-mode refusal cannot be built above the trait without a race (AMB-421), so it is a backend guarantee; whether a writer may write at all is not, so it is not.
- `register` creates, updates and refreshes: there is no separate refresh method. A refresh is a `register` of the same key by the same writer in `Lease` mode (§4).
- `now` is a parameter, as in ADR-015 (AMB-334). Nothing in the storage crate reads the clock (§6).
- One `Registrar` serves one `CatalogId`.
- The `Registrar` owns its store and verifier through `Arc`s, so a service can keep one in static state and share it across tasks; it borrows nothing from its caller.

### 2. Writer identity

- `WriterId::new(impl Into<String>) -> Result<WriterId, RecordError>`: 1 to 128 bytes of `[A-Za-z0-9._-]`, the `CatalogId` grammar, compared exactly. It is assigned by the operator and is opaque.
- The library trusts nothing about it beyond what the deployer authenticated. The deployer authenticates the caller by its own means (an IAM principal, a mutual-TLS certificate, a signed request) and maps that principal to a `WriterId`. The `Registrar` takes the `WriterId` as given.
- A `WriterId` is never derived from a network address, host name, URL or any entry member. Nothing in the API accepts an address. A service that moves to a new address and registers the same identifier with the same `WriterId` updates its entry (§3, #catalog-projection "retrieval-URL changes").
- Enforcement boundary (AMB-420): the `Registrar` constrains a writer only when that writer cannot reach the store without it. A process holding store write credentials can call `put_entry` directly and bypass every check in §3. Store write access is therefore operator power, and authority under this ADR holds only when the `Registrar` runs in a process the operator controls and writers reach it through an authenticated channel. This library ships no such channel (open question 2).

### 3. Authority

#### 3.1 Policy

```rust
pub struct RegistrationPolicy { /* grants: BTreeMap<WriterId, WriterGrant>, lease_ttl: Duration */ }
pub struct WriterGrant { /* publishers: BTreeSet<String>, prefixes: Vec<String>, require_publisher_signature: bool, curated: bool */ }

impl RegistrationPolicy {
    pub fn new(grants: impl IntoIterator<Item = (WriterId, WriterGrant)>, lease_ttl: Duration)
        -> Result<RegistrationPolicy, PolicyError>;
}
impl WriterGrant {
    pub fn new(publishers: impl IntoIterator<Item = String>, prefixes: impl IntoIterator<Item = String>,
               require_publisher_signature: bool) -> Result<WriterGrant, PolicyError>;   // leases only
    pub fn with_curated(self) -> WriterGrant;                                            // also curated
}

#[non_exhaustive]
pub enum PolicyError {
    EmptyGrant,                         // neither publishers nor prefixes
    InvalidPublisher(String),           // not a root did:web domain
    EmptyPrefix,
    UrnAirPrefix(String),               // starts with "urn:air:" in any ASCII case
    ZeroLeaseTtl,
    LeaseTtlOutOfRange,                 // a lease of this length from the Unix epoch ends past year 9999
    DuplicateWriter(WriterId),
    VerifierMissing,                    // a grant requires a signature and no verifier was given
}
```

- **Publishers** are `urn:air` `{publisher}` components, compared byte for byte with `UrnAir::publisher()` (ADR-004, M24). No case folding, no subdomain or suffix match (#the-did-web-publisher-profile; AMB-27). `WriterGrant::new` refuses, with `InvalidPublisher`, a publisher that is not a lowercase ASCII domain under the root `did:web` domain restrictions (#the-did-web-signer-profile: "lowercase ASCII, with no port, IP address, trailing root dot, or Unicode U-label"), which the publisher component "MUST satisfy" (#the-did-web-publisher-profile). The check is the one `DidWebRoot::parse` applies, with one definition: it moves from the trust crate to `turul_ai_catalog::identity::is_root_did_web_domain(&str) -> bool`, which `DidWebRoot::parse` calls after stripping `did:web:` and `WriterGrant::new` calls on each publisher. Storage still does not depend on trust (AMB-433).
- **Prefixes** admit non-`urn:air` identifiers whose text starts with the prefix, byte for byte. A prefix starting with `urn:air:` in any ASCII case is refused, so no prefix can admit an identifier that looks like `urn:air` (#catalog-projection rule 3, AMB-423).
- **No silent allow-all.** A grant must hold at least one publisher or prefix (`EmptyGrant`), and a writer without a grant is refused at `register` and `unregister` (`UnknownWriter`). There is no wildcard publisher or empty prefix (`EmptyPrefix`).
- `lease_ttl` is set by the operator, not requested by the writer, so a writer cannot hold a lease for longer than the operator allows. An expiry is stored as RFC 3339, which writes four-digit years, so `RegistrationPolicy::new` refuses with `LeaseTtlOutOfRange` a length whose lease, written at the Unix epoch, would end past 9999-12-31T23:59:59.999999999Z: no write could ever store it. The policy reads no clock, so the epoch is its reference; a write whose own `now + lease_ttl` ends past year 9999 is refused at §3.2 step 5.
- **Modes are granted.** A grant permits `Lease` only; `with_curated` also permits `Curated`. A curated entry never expires and, under a grant that requires a signature, is not verified again after its write (§5), so letting a writer choose it would let the writer, not the operator, decide whether its entry expires and whether it is re-verified. A `Curated` request under a lease-only grant is refused at §3.2 step 1, before any I/O (AMB-434).
- **Co-granted writers share identifiers, not records.** Ownership is per record key (§3.3). Writers granted the same publisher or a prefix admitting the same identifier are each the operator's configured delegate for it (AMB-422), so each may add a version of an identifier the other registered, and the first record of an identifier fixes its version kind for both (M21). Neither can overwrite or remove the other's record. An operator that needs one writer per identifier grants disjoint publishers and prefixes (AMB-424).
- `Registrar::new` refuses `VerifierMissing` when any grant sets `require_publisher_signature` and `verifier` is `None`, so the requirement can never be configured and silently skipped.

What the allow-list means (AMB-422): configuring publisher `example.com` for a writer is the operator's assertion that this writer is that publisher or its authorized delegate. It is configured policy, not inference. #catalog-projection forbids inferring authorization "from the artifact's URL or an unsigned `publisher` field", and the `Registrar` never reads `url`, `publisher` or any other entry member for authority; only `identifier` is checked. Where the operator wants the publisher itself to prove it, the grant sets `require_publisher_signature`.

#### 3.2 `register` checks, in order

The first failure is returned. Every check that needs no I/O runs before the store read, and the store read runs before any network work, so a request that local policy refuses never causes a fetch (AMB-428).

| Step | Check | Failure |
|---|---|---|
| 1 | `writer` has a grant, and the grant permits the requested mode (`Curated` needs `with_curated`) | `RegistrationError::UnknownWriter`, `CuratedNotGranted` |
| 2 | `EntryRecord::new(entry)` (ADR-016 record gate) | `Record(RecordError)` |
| 3 | identifier: if `UrnAir::parse` succeeds, its publisher is in the grant's publishers; if it fails and the identifier starts with `urn:air:` in any ASCII case, refuse; otherwise the identifier starts with one of the grant's prefixes | `PublisherNotAllowed { publisher }`, `MalformedUrnAir`, `IdentifierNotAllowed` |
| 4 | if the grant requires a signature, the identifier is `urn:air` (the profile's first requirement) | `SignatureRequiresUrnAir` |
| 5 | for `Lease`, `now + lease_ttl` (checked addition) has an RFC 3339 form, before the read; then `get_entry` of the key; when a record exists: its mode kind equals the requested one, and its writer equals `writer` | `ExpiryOutOfRange`, `ModeConflict { stored }`, `OwnedByAnotherWriter` |
| 6 | if the grant requires a signature, `verifier.verify_publisher(entry, now)` | `PublisherSignature(VerifierRefusal)` |
| 7 | the conditional write (§4.3) | `Store(StoreError)`, `Contended` (also when the store exhausted its own retries, `StoreError::Contended`) |

`unregister` runs step 1's grant lookup and step 3 on the key's identifier, then step 5's writer check (mode is not checked: a writer may remove its own entry in either mode), then `delete_entry` with `expected: Some(etag)`. A missing record is `NotFound`. A `PreconditionFailed` re-reads and repeats from step 5, within the attempt bound of §4.3. Authority is evaluated against the current policy: a writer whose grant no longer covers an identifier cannot remove it, and the operator removes it with store credentials. No signature is required to remove: removal makes no claim about the artifact.

#### 3.3 #catalog-projection obligations

| Obligation | How it holds |
|---|---|
| preserve a publisher-authorized `urn:air` identifier exactly | the `Registrar` never assigns, rewrites or normalises an identifier; the record holds the writer's identifier byte for byte (ADR-016 §4) |
| never mint `urn:air` under another domain | the `Registrar` mints nothing; a `urn:air` identifier is accepted only under a publisher the operator granted to that writer, and no prefix can admit one (§3.1) |
| MUST NOT infer authorization from URL or unsigned `publisher` | only `identifier` is checked (§3.1); with a signature required, authorization is the did:web publisher profile (§5) |
| identifier stable, never reassigned to another artifact | a stored record belongs to the writer that created it until it is deleted, expired or not (AMB-424); no other writer can overwrite or remove it. Ownership is per record key `(identifier, version)`, not per identifier: writers granted the same publisher or prefix share that identifier's versions and its version kind (§3.1). Whether two registrations of one identifier, by one writer or by co-granted writers, name the same artifact is not observable to the library; it is the duty of the writers and of the publisher that delegated to them, as ADR-016 §4 states for the store |
| re-registration from a new address is an update | the key is `(identifier, version)` and the owner is the `WriterId`; an address is never an input (§2) |
| operator and publisher are distinct roles | the operator configures grants and writes the host (ADR-016 `put_host`); writers register entries; the `Registrar` never writes the host |

### 4. Modes and provenance in the store

#### 4.1 `Provenance`

```rust
pub enum Provenance {
    Lease { writer: WriterId, expires_at: Timestamp },
    Curated { writer: WriterId },
}
pub enum ModeKind { Lease, Curated }

impl Provenance {
    pub fn kind(&self) -> ModeKind;
    pub fn writer(&self) -> &WriterId;
    pub fn to_json(&self) -> Json;
    pub fn to_bytes(&self) -> Result<Vec<u8>, ProvenanceError>;   // JCS, refused unless it reads back
    pub fn parse(bytes: &[u8]) -> Result<Provenance, ProvenanceError>;
}
```

- Bytes are the JCS of `{"mode":"curated","writer":W}` or `{"expiresAt":T,"mode":"lease","writer":W}`, where `T` is `Timestamp`'s RFC 3339 `Display` (ADR-005), produced by the one canonical writer: `Provenance::to_json` builds the object and ADR-003's `Json::to_canonical_vec` writes it; nothing assembles the bytes by hand. `to_bytes` then refuses bytes that `parse` would refuse, with `parse`'s error: an instant outside the years 0000 to 9999 has no RFC 3339 form and is `ExpiresAt`. `Provenance::parse` accepts exactly these members; anything else is `ProvenanceError`. `expires_at` compares as an instant, never as a string.
- `Provenance` is the one owner of these bytes. The schema is these two object shapes; `provenance::provenance_bytes_are_canonical` pins them against committed byte strings, which serve as its digest.

#### 4.2 Where the mode lives: beside the record bytes, inside the `Etag` (AMB-421)

- **Record bytes are unchanged**: still the JCS of the entry's own JSON, read by `parse::parse_entry` (ADR-016 §2). The served document and the entry's Trust Manifest payloads never see provenance.
- **Provenance is stored beside them**, one value per entry record, with its mode kind as a separate column or attribute so a backend can test it in a write condition.
- **The entry `Etag` covers both**: `Etag::of_entry(&EntryRecord, &Provenance) -> Result<Etag, ProvenanceError>` is the lowercase hex SHA-256 of the JCS of the two-element array `[entry JSON, provenance JSON]`, written by `Json::to_canonical_vec` over `Json::Array([CatalogEntry::to_json, Provenance::to_json])`; it refuses a provenance `to_bytes` refuses, so no version exists for an entry no store can hold. The host `Etag` stays `Etag::of(&[u8])` over host record bytes. Reason: a `Matches(etag)` write must mean "nothing I read has changed", writer and expiry included. With an `Etag` over entry bytes alone, a refresh that changes only `expires_at` leaves the `Etag` unchanged, so a writer that read a lease as expired could overwrite it after its owner refreshed it. Equal stored state still gives equal `Etag`s on every backend (AMB-402 holds, over the wider content).
- `get_entry` and `list_entries` return `StoredEntry { record: EntryRecord, provenance: Provenance, etag: Etag }`; `Stored<R>` stays for the host.
- `put_entry(&self, catalog: &CatalogId, record: &EntryRecord, provenance: &Provenance, condition: Condition) -> Result<Etag, StoreError>`. There is no entry write without a provenance.
- `StoreError` gains `ModeConflict { stored: ModeKind }` and `CorruptProvenance { key: RecordKey, error: ProvenanceError }`. Every backend's `put_entry` takes the provenance bytes and the `Etag` from one crate-private helper, which refuses a provenance whose bytes would not read back with `CorruptProvenance` before the store is touched; a stored mode that is missing or unknown is `CorruptProvenance` on every backend, never `Backend` text.

#### 4.3 Guarantee 8 and the write paths

ADR-016 §3 gains **guarantee 8, mode exclusion**: a `put_entry` that would replace an existing record whose mode kind differs from the new provenance's fails `StoreError::ModeConflict { stored }`, evaluated and written in one atomic step (as guarantee 1). Of two concurrent puts of one key, one `Lease` and one `Curated`, the stored record afterwards has the mode of a put that returned `Ok`, and every put of the other mode that ran after it returned `ModeConflict`. Order of checks: `PreconditionFailed`, then `ModeConflict` (only a replace can conflict), then `VersionKindConflict` (only a create can conflict); the last two are disjoint. Deletion is not a mode write: `delete_entry` is unchanged. `Condition::Overwrite` (ADR-016 §3) is exempt: it is the operator's replace (§4.8), and `write_owned` never uses it.

Writer ownership and expiry are not backend guarantees. They are decided above the trait by read-then-`Matches`, which is atomic because the `Etag` covers provenance. One crate-private function owns this, `write_owned(store, catalog, record, provenance, now)`, used by `Registrar::register` and by `import`:

1. `get_entry`. Absent: `put_entry` with `Condition::Absent`.
2. Present: refuse `ModeConflict` when its mode kind differs, `OwnedByAnotherWriter` when its writer differs.
3. Present, same writer. `Curated`: when the record bytes equal the stored ones, nothing is written; otherwise `put_entry` with `Condition::Matches(stored etag)`. `Lease`, with `slack = lease_ttl / 4`:
   - **Refresh** (record bytes equal the stored ones): when the stored `expires_at` is at or after `now + lease_ttl − slack`, nothing is written. Otherwise `put_entry` with `expires_at = now + lease_ttl` and `Condition::Matches(stored etag)`. A written expiry is later than the stored one by more than `slack`, so a refresh never shortens a lease.
   - **Update** (record bytes differ): `put_entry` with `expires_at` the later of the stored one and `now + lease_ttl`, `Condition::Matches(stored etag)`.

   When nothing is written the result is `Ok` with the stored `Etag` and `Registered.written` is `false`.
4. `PreconditionFailed` re-reads and repeats from 1. Each repeat follows a write another writer committed. After 8 attempts the result is `RegistrationError::Contended`. A backend that exhausts its own bounded retries (DynamoDB, ADR-016 §6) returns `StoreError::Contended`, also reported as `RegistrationError::Contended`.

For a lease, the granted expiry `now + lease_ttl` and the due instant `now + lease_ttl − slack` are computed before step 1 with checked addition, and one that has no RFC 3339 form (past year 9999) is `ExpiryOutOfRange`, without I/O and without a panic.

`Registrar::register` steps 5 to 7 (§3.2) are this function: its read is step 5, the verifier (step 6) runs between the read and the conditional write, and a retry re-reads without verifying again, since the entry has not changed.

**Many instances refreshing one lease (AMB-430).** Refreshes of one lease have equal intent, and the rule makes them compute equal state, whatever `now` each instance passes. Once one refresh commits with `now_w`, the stored expiry is `now_w + lease_ttl`, and every instance whose `now` is less than `now_w + slack` finds the lease not due and returns `Ok` without writing. Two committed refreshes therefore need `now` values more than `slack` apart: at most one refresh write commits per `slack` window. An instance's `Matches` fails only after another refresh committed, so when the racing instances' `now` values lie within one `slack` window, exactly one write commits and each instance makes at most two attempts, however many instances there are. Generally, racing instances whose `now` values span `D` commit at most `⌊D / slack⌋ + 1` writes, so `Contended` needs `D` of at least `7 × slack` (`1.75 × lease_ttl`). Concurrent updates with equal new bytes converge the same way: after the first commits, the rest are refreshes.

#### 4.4 Interaction with M21 (version-kind guard)

- The guard counts records, whatever their mode or expiry. An expired lease still holds its identifier's version kind until the record is deleted, so a registration of the other kind is `VersionKindConflict`. That is the correct reading of #multi-version-entries for the store's state (the record exists), and it is visible: assembly reports the expired record (§6), and its owner or the operator removes it.
- Mode is per record. One identifier may have a curated `1.0.0` and a leased `2.0.0`; both share one version kind.
- `ModeConflict` needs an existing record and `VersionKindConflict` a new one, so a put never meets both.

#### 4.5 Curated writes

- `import(store, catalog, document, writer: &WriterId, mode: ImportMode)` in `ImportMode::Merge` writes every entry with `Provenance::Curated { writer }` through `write_owned`. It refuses an entry whose key holds a lease (`ImportError::Store(StoreError::ModeConflict)`) or a curated record of another writer (`ImportError::OwnedByAnotherWriter`). It runs no allow-list: `import` is the operator's path, as in ADR-016, and its authority is whoever holds the store credentials (§2).
- The CLI `produce` commands still write files only (ADR-016 AMB-406). Their curated route into a store is `produce` → file → `import`, which records `Curated`.
- **The curated writer id is permanent configuration.** `import` in `Merge` refuses curated records of another writer, so an import with a changed writer id fails at the first entry the catalog already holds. If the id changes or is lost, the operator runs `store import --replace` with the new id (§4.8, §8), which overwrites every entry as curated by it; it is refused while the catalog id holds a live lease, and then the operator follows §4.7 Route B with the new writer id. The README states this where it documents `import` (R1, R6).
- `ServedCatalog::from_document` imports, in `ImportMode::Merge`, into a store private to the call with the writer id `from-document`, a constant private to the Lambda crate; its signature does not change.
- `Registrar::register` with `ModeRequest::Curated` is the same write after §3 authority, for a writer whose grant the operator made with `with_curated` (§3.1). A lease-only grant asking for `Curated` is refused with `CuratedNotGranted` at step 1.

#### 4.6 Backend schema and keys

`RecordKey` shapes (`"h"`, `"e:<hex>"`, `"g:<hex>"`) are unchanged. Expiry is not indexed: assembly reads every entry and decides in the crate (§6).

| Backend | Change |
|---|---|
| `in-memory` | the records map value for an entry holds `(record bytes, provenance bytes, ModeKind, Etag)`; the host value is unchanged. Guarantee 8 is checked under the existing write lock |
| `sqlite` | `catalog_records` gains `provenance BLOB` and `mode TEXT`, with `CHECK ((record_key = 'h' AND provenance IS NULL AND mode IS NULL) OR (record_key <> 'h' AND provenance IS NOT NULL AND mode IN ('lease','curated')))`. Inside the existing per-identifier transaction, after the guard row lock, the existing-record read also returns `mode`, and a replace with a different mode is `ModeConflict` |
| `postgres` | the same columns with `provenance BYTEA`, the same check and the same transaction step |
| `dynamodb` | entry items gain `provenance` (B) and `mode` (S). The `Any` replace `PutItem` condition becomes `attribute_exists(pk) AND #mode = :m` with `ReturnValuesOnConditionCheckFailure = ALL_OLD`: an old item with another `mode` is `ModeConflict`, no old item is the create path. Creates (`TransactWriteItems`) and `Matches` (`etag = :e`, which covers provenance) are otherwise unchanged. No TTL attribute is set (open question 1) |

`connect` for sqlite and postgres probes `SELECT provenance, mode FROM catalog_records LIMIT 0` after `CREATE TABLE IF NOT EXISTS`, and fails with `StoreError::Backend` naming the missing column on a table created by an earlier release. A DynamoDB entry item without `provenance` reads as `StoreError::CorruptProvenance`.

#### 4.7 Records stored by earlier releases, and upgrading a live store (AMB-427, AMB-431)

Every record stored before this ADR is curated in fact, but has no writer. It is not read as curated: a record without a writer cannot satisfy ownership (§3.3), and reading a missing value as a default is a compatibility path, which this repository does not keep. Until replaced or purged the store fails loudly: SQL `connect` fails, and a DynamoDB assembly fails with `CorruptProvenance`, so the Lambda fails cold start or keeps its last good snapshot (ADR-016 AMB-407). No migration code is written: both routes below replace or delete stored data and never read an older format (M30).

Upgrading a store being served takes one of two routes. Each runs the **new** release's CLI with store credentials (§8).

**Route A, same catalog id.** For an id that holds no live lease (§4.8 refuses one).

1. Stop every writer of the id that runs the previous release (it would write the old format into the id, see below).
2. `turul-ai-catalog store import BACKEND --catalog-id LIVE --file CATALOG --writer WRITER --replace` with the permanent writer id (§4.5). The id then holds exactly the document, in the new format.
3. Deploy the new function on the same `TURUL_AI_CATALOG_ID`. Writers move to the new release and register again.

During step 2 the previous function keeps serving the id; what it observes is stated in §4.8. The new function is deployed only after step 2 has returned, so it never assembles an id that still holds an item it cannot read. There is no rollback by redeploy: the id is in the new format from step 2.

**Route B, new catalog id.** For an id with live leases, or when rollback must stay a redeploy.

1. `turul-ai-catalog store import BACKEND --catalog-id NEW --file CATALOG --writer WRITER` under a new `CatalogId` in the same table. Every writer moves to the new release and registers into the new id.
2. Repoint the function: deploy the new function with `TURUL_AI_CATALOG_ID=NEW`.
3. `turul-ai-catalog store purge BACKEND --catalog-id OLD --yes`, with `TURUL_AI_CATALOG_ID=NEW` in the operator's environment, so the command's live-id refusal (§8) protects the new id.

Until step 2 the previous function serves the old id unchanged; from step 2 the new function serves the new id. Neither assembly reads the other's items (ADR-016 guarantee 5), and the purge reads none of the old items it removes (ADR-016 guarantee 9); `dynamodb::old_and_new_format_catalog_ids_share_a_table` pins both. The test is DynamoDB's own, not a `parity` check: only DynamoDB can hold both formats in one table, since in-memory stores never outlive a release and SQL `connect` refuses an earlier table. For a format change that makes SQL `connect` refuse the earlier table, as this ADR's did, neither route runs in that database: a SQL store follows Route B with a new database (sqlite: a new file) in place of the new catalog id, and the old database is dropped by its owner.

A release that changes the stored format says in its `CHANGELOG.md` entry which routes apply to which backends, and whether the previous release's function can still read the items Route A writes. A release that does not change the stored format needs neither.

**The previous release must never be pointed at a new-format catalog id.** It knows nothing of provenance: on DynamoDB its `Any` replace writes an entry item without `provenance` or `mode`, which then reads as `CorruptProvenance` and fails every assembly of that id by the new release, and it bypasses guarantee 8; on SQL its entry writes either fail the new `CHECK` or replace record bytes without regard to provenance, so the entry `Etag` and guarantee 8 no longer hold. Nothing in the previous release can detect this.

The R1 release's `CHANGELOG.md` entry starts with the line `**Breaking for anyone holding a store:** re-import under a new catalog id before deploying (see ADR-017).`

#### 4.8 Replace import (M30, AMB-435)

```rust
pub enum ImportMode { Merge, Replace { now: SystemTime } }
```

`import(store, catalog, document, writer, ImportMode::Replace { now })` makes the catalog id hold exactly the document: its entries, each curated by `writer`, and its host (one exception, step 3). `now` is the caller's, as everywhere in this crate (§6). Steps, in order:

1. **Gate, no I/O.** The checks of `Merge`: a signed document, top-level members the store cannot hold, `validate` errors, and each entry's and the host's record gate. A failure is the same `ImportError` and nothing is written.
2. **Scan, reads only.** `scan_entries` (ADR-016 §3). Record bytes that do not parse fail with `ImportError::Store(StoreError::Corrupt)`; the operator uses §4.7 Route B, since a purge reads nothing. Every scanned entry whose provenance reads as `Lease` with `now < expires_at` is collected; when any is, `import` returns `ImportError::LiveLeases { leases: Vec<(EntryKey, Provenance)> }` (every such lease, ordered by `EntryKey`) and nothing is written. Registered entries are not the importer's to overwrite (§3.3); their owners unregister them, or let them expire, or the operator takes Route B. An entry whose provenance does not read is never a live lease: this release writes only readable provenance.
3. **Host.** `put_host` with `Condition::Any`; the stored host is overwritten. The exception: a document without `host` leaves a stored host in place, since the trait has no `delete_host` (ADR-016 §3) and adding one for this case is not needed by any upgrade; an operator who wants no host uses Route B, whose new id starts without one.
4. **Overwrite or add.** Each document entry, in `EntryKey` order, as `Provenance::Curated { writer }`: a key the scan held is written with `Condition::Overwrite(scanned etag)`, which replaces the record whatever its mode, writer or readability, provided it is still the version scanned, and never reads the old record (ADR-016 §6); a key the scan lacked is written with `Condition::Absent`. A `PreconditionFailed` means the key was written after the scan (a lease registered or refreshed, a writer's record changed or created); the import stops with `ImportError::Store(StoreError::PreconditionFailed)` and the writes so far stand. A create refused with `VersionKindConflict` (the identifier still holds records of the other kind, all absent from the document, which passed `validate`) is deferred to step 6.
5. **Delete absent entries.** Each scanned entry whose key is not in the document: `delete_entry` with `expected: Some(scanned etag)`. A `PreconditionFailed` means the record changed after the scan (a writer wrote it); the import stops with `ImportError::Store(StoreError::PreconditionFailed)` and the writes so far stand. A record deleted meanwhile is not an error: its conditioned delete also fails `PreconditionFailed`, so the import reads the key once with `get_entry` and goes on when it holds no record.
6. **Deferred creates.** Each entry deferred at step 4: `put_entry` with `Condition::Absent`. A `VersionKindConflict` now means a record of the other kind was written after the scan, and stops the import with that error.

It returns the `Etag`s of the document's entries, in document order, as `Merge` does. It is not atomic across records: a failure part-way leaves the records written and deleted so far, and re-running converges (or refuses a lease registered meanwhile).

**What a concurrent assembly observes.** Every document entry whose key the id held before the import is present throughout, as its old record until step 4 overwrites it and as the new one after. Entries absent from the document stay until step 5, so between steps 4 and 5 an assembly serves a superset of the document; no write empties the id. An identifier whose version kind changes is missing from step 5 until step 6, which assembly serves without error. An assembly by this release fails with `CorruptProvenance` while the id still holds an item whose provenance does not read, that is until step 4 or step 5 has overwritten or deleted the last one; this is why Route A deploys the new function only after the import returns. An assembly by the previous release reads the items step 4 writes only if that release's reader accepts them, which the release changing the format states (§4.7); if it does not, the previous function keeps its last good snapshot (ADR-016 AMB-407) until the new function replaces it.

**Concurrent writers.** The import is not isolated from writers of the same id, but never overwrites their work: the live-lease refusal covers leases committed before the scan, and every write and delete after it is conditioned on what the scan saw (step 4 on the scanned version or on absence, step 5 on the scanned version), so a lease registered or refreshed under a document key after the scan stops the import with `PreconditionFailed`, and re-running refuses it as a live lease. A lease registered under a key the document lacks after the scan survives, and the id then holds it beside the document. Route A still stops the id's writers first (§4.7), since writers of the previous release would write the old format into the id.

**Authority.** `Replace` overwrites records of other writers and expired leases, which `Merge` and the `Registrar` never do (AMB-424). It runs no allow-list and is the operator's path with store credentials, as `Merge` is (§4.5): a holder of those credentials can already delete any record (§2).

### 5. Optional publisher signature

- Storage defines the seam, the trust crate implements it, and storage never depends on the trust crate (ADR-016 §1 and its `dependency_direction` pin stand):

```rust
// turul-ai-catalog-storage
#[async_trait]
pub trait PublisherVerifier: Send + Sync {
    async fn verify_publisher(&self, entry: &CatalogEntry, now: SystemTime) -> Result<(), VerifierRefusal>;
}
pub struct VerifierRefusal(pub String);   // the verifier's reason, as text
```

- The trust crate gains feature `registration` (off by default), which adds exactly `turul-ai-catalog-storage` (`default-features = false`, so no backend; that set is the core crate, `async-trait` and `sha2`), `async-trait` and `tokio` (`rt` only), and the type `DidWebPublisherVerifier { policy: TrustPolicy }` implementing `PublisherVerifier`. The trust crate's default dependency set is unchanged. There is no cycle: storage → core; trust → core, client and, under the feature, storage. Crypto and network stay out of every storage feature.
- The trust crate gains `verify_entry(entry: &CatalogEntry, base: Option<&str>, policy: &TrustPolicy, now: SystemTime) -> EntryReport<'_>`: resolve the distinct signers of the entry's manifests that pass `precheck`, then the same crate-private `entry_report` that `verify_resolved` uses. One per-entry procedure serves both; `verify` keeps its tree walk. ADR-015 records this in its revision log in the implementing slice.
- `DidWebPublisherVerifier::verify_publisher` runs `verify_entry(entry, None, &policy, now)` on a blocking thread (`tokio::task::spawn_blocking`, since resolution and integrity fetch block, ADR-015 §5) and accepts when at least one manifest's `endorsement()` is `Endorsement::Publisher`: an accepted `did-web-v1` signature in its time window (#signature-acceptance-and-time), Entry Release Coverage including `digest` and verified artifact bytes (#entry-release-coverage), and `publisher_authorized` (the root `did:web` domain rule and exact `did:web:` + publisher equality, #the-did-web-publisher-profile). Otherwise the refusal names each manifest's outcome. An assessor's signature never passes. `base` is `None`: a relative `url` cannot be fetched for integrity and so is refused.
- The verification is a write-time gate (AMB-425). It runs on every write by a grant that requires it, refreshes included, so a manifest that has expired since the last refresh stops the lease from being extended, and the entry leaves the catalog when its lease runs out. A curated entry verified at write is not re-verified, which is why curated registration is the operator's grant, never the writer's choice (§3.1). Consumers still verify for themselves (ADR-015): a stored entry passed this gate once and is never presented as endorsed by the store or the served document.

### 6. Expiry and assembly

- `assemble(store: &dyn CatalogStore, catalog: &CatalogId, now: SystemTime) -> Result<Assembled, AssembleError>`. A lease is live while `now < expires_at`, the half-open window the spec uses for `expiresAt`; at `expires_at` it is expired. No skew is applied: one clock (the assembler's) decides.
- `Assembled { bytes, report, omitted: Vec<Omission>, next_expiry: Option<Timestamp> }`, `Omission { key: EntryKey, provenance: Provenance, reason: OmissionReason }`, `OmissionReason::{LeaseExpired { expires_at: Timestamp }, Invalid(Report)}`, all `#[non_exhaustive]`. Every listed entry record is either in `bytes` or in `omitted`; assembly never drops one silently. `next_expiry` is the earliest `expires_at` among the leases in `bytes` (`None` when it holds none), taken from the provenance assembly already reads: no extra store read.
- ADR-016 §4 steps change as follows: after listing, expired leases are moved to `omitted`; the rest are sorted and validated as one document. When that document has error diagnostics, each remaining entry is run through the record gate's validation; entries that fail it are moved to `omitted` as `Invalid` with their own report, and the document is validated again. Errors that remain (cross-entry, such as #multi-version-entries' identifier-alone rule in a store this crate did not write) still fail with `AssembleError::Invalid`, because the spec does not say which of the conflicting entries to drop (AMB-426). `Invalid` covers records that passed an earlier release's gate and fail the current one.
- The result is a pure function of committed records and `now`: equal state and equal `now` give equal bytes on every backend. A lease expiring changes the assembled snapshot; the served document still carries no catalog signature (#catalog-snapshot-coverage; ADR-016 §4). Expiry depends on the clock, never on the caller, so this is not #dynamic-discovery.
- The clock is read only by callers: `SnapshotCache` (Lambda), a registrar's host, tests. `registration::storage_source_never_reads_the_clock` scans the storage crate's `src/` for `SystemTime::now` and `Instant::now`.
- **Lambda.** `SnapshotCache` passes its clock's `now` to `assemble`. `main` writes one stderr line per omission per assembly (key, writer, reason), so an omission is reported once per refresh per warm instance. ADR-010 is updated in the implementing slice.
- **Snapshot freshness (AMB-432).** `SnapshotCache` refreshes at the earlier of `max_age` after assembly and the snapshot's `next_expiry`: a request at or after either re-assembles before `handle`. An expired lease is therefore never served past its expiry by the cache's clock, where a refresh by `max_age` alone would serve it up to a whole `max_age` late. One exception stands from ADR-016 AMB-407: a failed re-assembly keeps the last good snapshot, which can hold a lease past its expiry, and schedules the next attempt `max_age` later; the failure is logged. The cache is in-process only: each warm instance has its own, and no cross-process check of cache age exists (not built).
- **Lease length (AMB-429).** A registration appears up to `max_age` late per warm instance; an expired lease leaves at the first request at or after its expiry. A live lease is never missing from a served snapshot when its owner's due refresh commits before expiry: refresh interval + clock skew between the registrar's and the Lambda's hosts < `lease_ttl − slack` (§4.3), which `lease_ttl ≥ 4 ×` the refresh interval (§7) satisfies for skew below `lease_ttl / 2`. `max_age` no longer enters the rule. The README states it. The library does not check it: `lease_ttl` lives in the `RegistrationPolicy` of a registrar process, the refresh interval in each service, and no shared configuration exists between them; adding one would be a new control.
- **Expired records stay** until their owner unregisters them, the operator deletes them (§7), or a write to the same key by the same writer replaces them. A reaper is a new control and is not built (open question 1).

### 7. Deployment

Authority holds only if no registering service's role can write the table; the registrar runs under its own role. Registering services reach the registrar through the deployer's authenticated channel (§2); only the registrar's role and the operator's `import` role hold ADR-016 §7's writer actions, and the serving role reads only.

- **When a service registers.** A service registers once it is serving, never during init: a lease claims the artifact is reachable, and one left by an init that then fails stays served until it expires.
- **Refresh failures.** A refresh that fails, including a `PublisherSignature` refusal because the DID document or the artifact could not be fetched, is reported by the service and retried at its next interval; it is not fatal while the lease has time left. Recommended: `lease_ttl ≥ 4 ×` the refresh interval. Then the first due refresh after a write comes while at least `lease_ttl / 2` remains, so at least two consecutive failed due refreshes pass before the lease expires, and the next successful refresh after expiry restores the entry.
- **Removing a stale lease.** Its owner calls `Registrar::unregister(&writer, &key)`. Otherwise the operator, with store credentials, deletes it by its `EntryKey` (an `Omission.key` from assembly, §6), conditioned on the version it read so that a refresh in between makes the delete fail rather than remove a live lease:

```rust
use std::time::SystemTime;
use turul_ai_catalog::{identity::{EntryIdentifier, EntryKey}, timestamp::Timestamp};
use turul_ai_catalog_storage::{CatalogId, CatalogStore, Provenance, StoreError};

async fn remove_expired_lease(store: &dyn CatalogStore, catalog: &CatalogId, key: &EntryKey)
    -> Result<bool, StoreError> {
    let Some(stored) = store.get_entry(catalog, key).await? else { return Ok(false) };
    let Provenance::Lease { expires_at, .. } = &stored.provenance else { return Ok(false) };
    if *expires_at > Timestamp::from_system_time(SystemTime::now()) { return Ok(false) }
    store.delete_entry(catalog, key, Some(&stored.etag)).await?;   // PreconditionFailed if refreshed
    Ok(true)
}

let key = EntryKey::new(EntryIdentifier::new("urn:air:example.com:a2a:billing"), Some("1.0.0".into()));
remove_expired_lease(&store, &catalog, &key).await?;
```

The operator's command for the same removal is §8. The README carries this section (R3).

### 8. Operator store command

Maintainer decisions M27 and M30. `turul-ai-catalog store` lists the entries of a store with their provenance and expiry state, removes one entry by `EntryKey`, imports a catalog file, and purges a catalog id. It is an operator tool run with store credentials; it grants no authority a holder of those credentials lacks (§2, §7).

```
turul-ai-catalog store list   BACKEND --catalog-id ID
turul-ai-catalog store remove BACKEND --catalog-id ID --identifier IDENTIFIER [--version VERSION]
turul-ai-catalog store import BACKEND --catalog-id ID --file PATH --writer WRITER [--replace]
turul-ai-catalog store purge  BACKEND --catalog-id ID [--yes]
BACKEND: --sqlite PATH | --postgres URL | --table NAME
```

`import` sits under `store` with the other commands that take a backend, so the backend group and its environment variables have one definition (AMB-437).

- **Backend and catalog.** Exactly one backend, by a required `clap` group; each flag also reads an environment variable (a flag wins over its own variable; two different backends set at once are a usage error). `--table` (DynamoDB) reads `TURUL_AI_CATALOG_TABLE` and `--catalog-id` reads `TURUL_AI_CATALOG_ID`, the Lambda's variables (ADR-010); `--sqlite` reads `TURUL_AI_CATALOG_SQLITE` and `--postgres` reads `TURUL_AI_CATALOG_POSTGRES_URL`. SQLite is always built; `--postgres` needs the CLI feature `postgres` and `--table` the feature `dynamodb`. Credentials for DynamoDB come from the AWS default chain, as in the Lambda; the in-memory store is not selectable, since it holds nothing across processes. The CLI connects (the constructors of ADR-016) and runs a current-thread `tokio` runtime; it holds no other logic.
- **Owner.** `turul_ai_catalog_storage::list_registrations(store, catalog, now)` returns one `Registration { key, provenance, expiry }` per stored entry, ordered by `EntryKey`, with `expiry` `Never` (curated), `Live(expires_at)` or `Expired(expires_at)` by the rule of §6 (a lease is live while `now` is before `expires_at`). `remove_registration(store, catalog, key)` reads the entry, deletes it conditioned on the read `Etag` (so a refresh in between fails the delete, §7) and returns its `Provenance`; it removes an entry of either mode. `RemoveError::{NotFound, Store}`. Both are over `&dyn CatalogStore`, so every backend behaves alike; `Registration` has a `Display` form of one line, `identifier  version-or-"-"  mode  writer  state`, which is for people and not a contract.
- **`list`** writes one line per entry to stdout, through `DisplayText`, and exits 0, also for an empty store and for expired leases.
- **`remove`** matches the `EntryKey` exactly: no `--version` is the unversioned key, never every version (ADR-011 AMB-344). It writes `removed <identifier>[@version]` to stdout.
- **`import`** reads `--file` through `turul_ai_catalog_client::load` (`Source::File`) with `FetchPolicy::max_requests` 0, so no request is sent and nested catalogs are imported as the entries they are, never followed; the CLI's `src/` keeps no `std::fs` (ADR-011 §1). `--writer` is parsed by `WriterId::new`. It calls `turul_ai_catalog_storage::import` on `ResolvedCatalog::document` with `ImportMode::Merge`, or with `--replace` `ImportMode::Replace { now }` where `now` is the CLI's clock (§4.8). It writes `imported <n> entries` to stdout. Load warnings about children not fetched are not printed: the store holds the nested entry, not the child.
- **`purge`** without `--yes` is a preview: it calls `count_catalog` and writes one line per item kind, `host <n>`, `entries <n>`, `guards <n>`, `other <n>`, then `nothing removed; run again with --yes to remove`, and removes nothing. With `--yes` it calls `purge_catalog` and writes the same four lines for `PurgeReport::removed`, then `removed <total> items`. Neither reads record bytes or provenance (ADR-016 guarantee 9), so it works on an id this release cannot read. An id with no items reports zeros and exits 0.
- **Live-id refusal.** `purge` refuses, before connecting to the store, when its catalog id equals the value of `TURUL_AI_CATALOG_ID` in its environment, with a message naming `TURUL_AI_CATALOG_ID`. Since `--catalog-id` falls back to that variable, a purge whose id comes from the environment is always refused: the id to purge must be given as a flag. `purge --yes` also refuses while the variable is unset, naming it, since with nothing to compare any id would pass; the preview needs no variable. This guards the operator's slip, not authority (§2): store credentials can delete any item.
- **Exit codes** (ADR-011): 0 done, including a preview and a purge that removed nothing; 1 only for `import` refused with `ImportError::Invalid` (the loaded document has an error diagnostic, ADR-011's meaning of 1); 2 usage error (`clap`), no backend selected or two selected, an invalid catalog id or writer id, a file that `load` cannot read or parse, a backend failure, no entry under the key (`NotFound`), a delete refused because the entry changed since it was read, every other `ImportError` (`Signed`, `Unstorable`, `LiveLeases`, `OwnedByAnotherWriter`, `Store`), and a purge refused for the live id. Failures go to stderr through `DisplayText`; `LiveLeases` lists each lease's key, writer and expiry.

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Writer identity | `turul_ai_catalog_storage::WriterId::new` | deployer → `Registrar::register`, `unregister`, `import` |
| Writer authority | `RegistrationPolicy::new`, `WriterGrant::new` | `Registrar::new` |
| Registration and removal | `Registrar::register`, `Registrar::unregister` | deployer's authenticated endpoint |
| Ownership, refresh and mode checks before a write | crate-private `write_owned` | `Registrar::register`, `import` |
| Entry mode and writer | `Provenance` (`to_json`, `to_bytes`, `parse`) | `put_entry`, every backend's reads |
| Entry version | `Etag::of_entry(&EntryRecord, &Provenance)`; host `Etag::of` | every backend, via the crate |
| Mode exclusion | `CatalogStore::put_entry`, guarantee 8 | four backends |
| Publisher signature seam | `PublisherVerifier` (storage); `DidWebPublisherVerifier` (trust, feature `registration`) | `Registrar::register` step 6 |
| Per-entry trust evaluation | `turul_ai_catalog_trust::verify_entry` (shares `entry_report` with `verify_resolved`) | `DidWebPublisherVerifier` |
| Served document, omissions and next expiry | `assemble(store, catalog, now)` | `ServedCatalog::assemble`, `SnapshotCache` |
| Served snapshot refresh time | `turul_ai_catalog_lambda::SnapshotCache` (earlier of `max_age` and `Assembled::next_expiry`) | Lambda `main` |
| `urn:air` grammar | `turul_ai_catalog::identity::UrnAir` (ADR-004) | `Registrar` step 3, `publisher_authorized` |
| Root `did:web` domain rule | `turul_ai_catalog::identity::is_root_did_web_domain` (moved from `DidWebRoot::parse`) | `DidWebRoot::parse`, `WriterGrant::new` |
| Instant | `turul_ai_catalog::Timestamp` (ADR-005) | `Provenance`, `assemble` |
| Operator listing and removal | `list_registrations`, `remove_registration` (storage) | `turul-ai-catalog store list`, `store remove` |
| Replace import | `import` with `ImportMode::Replace` (§4.8) over `scan_entries`, `Condition::Overwrite(etag)` and `Condition::Absent` (ADR-016) | `turul-ai-catalog store import --replace` |
| Catalog purge | `CatalogStore::purge_catalog`, preview `count_catalog` (ADR-016 guarantee 9) | `turul-ai-catalog store purge` |
| Live-id refusal of a purge | the CLI's `store purge` argument check against `TURUL_AI_CATALOG_ID` | `turul-ai-catalog store purge` |

Schema digests: provenance bytes are pinned by `provenance::provenance_bytes_are_canonical`; entry record bytes are unchanged (ADR-016). Determinism: `parity::same_state_and_now_assemble_to_the_same_bytes`.

## Requirement → test

Paths are under `crates/turul-ai-catalog-storage/tests/` unless stated. `parity::x` runs on every backend: `in_memory.rs` and `sqlite.rs` in CI, `postgres.rs` and `dynamodb.rs` against the live services they start (ADR-016 §6). `registration.rs` drives the `Registrar` over the in-memory store with a stub `PublisherVerifier` that records its calls.

| Requirement | Spec text or M26 clause (short) | Test function |
|---|---|---|
| catalog-projection-1 / catalog-entry-2 | publisher-authorized `urn:air` "MUST preserve it exactly" | `parity::registered_identifier_reads_back_exactly` |
| catalog-projection-3 | operator neither publisher nor delegate "MUST use a non-`urn:air` identifier" | `registration_policy::urn_air_prefix_is_refused` (`urn:air:`, `URN:AIR:`, `Urn:Air:x`), `registration::malformed_urn_air_is_refused` (`urn:air:example.com:only-two`, `URN:AIR:example.com:a:b` against prefix `URN`) |
| catalog-projection-4 | "MUST NOT infer publisher authorization from the artifact's URL or an unsigned `publisher` field" | `registration::url_and_publisher_field_never_grant_authority` (`url` on a granted domain and `publisher.identifier` `did:web:` of a granted domain, identifier under another publisher: `PublisherNotAllowed`) |
| catalog-projection-5 | identifier "MUST remain stable … MUST NOT be reassigned" | `parity::another_writer_cannot_take_over_an_entry` (live and expired lease, curated), `registration::reregistration_with_a_new_url_is_an_update` (one record, new `Etag`) |
| multi-version-entries-2 | absent `version`: `identifier` alone "MUST be unique" | `parity::expired_lease_keeps_its_version_kind` |
| the-did-web-publisher-profile-1 | "The entry MUST use the standard `urn:air` syntax" | `registration::signature_required_refuses_non_urn_air` (verifier not called) |
| the-did-web-publisher-profile-2/3/4 | domain restrictions; exact `did:web:` match; no subdomain or suffix heuristics | trust `publisher_verifier::publisher_signature_is_accepted`, `publisher_verifier::subdomain_or_suffix_signer_is_refused`, `publisher_verifier::assessor_signature_is_refused`, `publisher_verifier::ip_spelled_publisher_is_refused` |
| the-did-web-publisher-profile-5 | "MUST satisfy Entry Release Coverage" incl. artifact bytes | trust `publisher_verifier::digest_mismatch_is_refused`, `publisher_verifier::entry_without_digest_is_refused` |
| signature-acceptance-and-time-3 | not "before `issuedAt` or at or after `expiresAt`" | trust `publisher_verifier::expired_publisher_signature_is_refused` (`now` = `expiresAt`), `registration::refresh_with_an_expired_signature_does_not_extend_the_lease` |
| catalog-snapshot-coverage-1 | changing a contribution changes the snapshot | `assembly::lease_expiry_changes_the_assembled_bytes_and_adds_no_signature` |
| dynamic-discovery-1 | out of scope | `assembly::assembly_depends_only_on_records_and_now` |
| M26 allow-list | per-writer `urn:air` publisher allow-list | `registration::urn_air_outside_the_writers_publishers_is_refused`, `registration::publisher_match_is_exact` (`Example.com`, `sub.example.com`, `example.com.evil`) |
| M26 allow-list (non-`urn:air`) | operator-configured prefixes | `registration::non_urn_air_outside_every_prefix_is_refused`, `registration::non_urn_air_under_a_prefix_is_accepted` |
| M26 refuse when none | no silent allow-all | `registration_policy::empty_grant_is_refused`, `registration_policy::empty_prefix_is_refused`, `registration::unknown_writer_is_refused` |
| M26 optional signature | signed Trust Manifest required when configured | `registration::required_signature_is_checked_by_the_verifier`, `registration::signature_not_required_never_calls_the_verifier`, `registration_policy::signature_required_without_verifier_is_refused` |
| M26 modes | lease and curated refuse each other | `parity::lease_and_curated_puts_refuse_each_other` (`Any` in both directions), `parity::racing_cross_mode_puts_leave_one_mode` (16 writers, four rounds), `registration::curated_registration_over_a_lease_is_refused`, `import::import_refuses_a_lease_record`, `import::import_records_curated_provenance`, `import::import_under_another_writer_id_is_refused` |
| M26 lease refresh | idempotent conditional write, many instances | `parity::many_instances_refreshing_one_lease_all_succeed` (a counting wrapper over the backend; 64 instances, more than the 8-attempt bound, each passing a distinct `now` within one `slack` window, against a due lease: all `Ok`, exactly one committed write, stored expiry = that writer's `now + lease_ttl`, record bytes unchanged; repeated over four consecutive `slack` windows: at most one committed write per window), `registration::refresh_is_a_no_op_until_due` (stored expiry equal to `now + lease_ttl − slack`: `Ok`, stored `Etag`, `written: false`, no `put_entry`; `now` one nanosecond later: one write with `expires_at = now + lease_ttl`), `registration::refresh_never_shortens_a_lease` |
| M26 assembly report | expired or omitted entries reported | `assembly::expired_lease_is_omitted_and_reported`, `assembly::lease_is_live_until_expires_at` (live one nanosecond before, omitted at), `assembly::entry_failing_the_current_gate_is_omitted_and_reported`, `assembly::cross_entry_error_still_fails_assembly`, `assembly::every_listed_entry_is_served_or_omitted`, `parity::same_state_and_now_assemble_to_the_same_bytes`; lambda `snapshot_refresh::omissions_are_reported_on_each_assembly`, `snapshot_refresh::expired_lease_leaves_the_served_catalog_at_the_next_refresh` |
| M26 unregister | removal by the same writer, with authority | `parity::unregister_by_another_writer_is_refused`, `parity::unregister_frees_the_identifier`, `registration::unregister_needs_current_authority`, `registration::unregister_of_a_missing_entry_is_not_found` |
| M27 list | list entries with provenance and expiry state | `operator::list_reports_provenance_and_expiry_state` (in-memory, sqlite); cli `store_commands::list_prints_provenance_and_expiry_state` |
| M27 remove | remove by entry key; exact key; conditioned on the read version | `operator::remove_deletes_exactly_the_named_key`, `operator::remove_of_a_missing_key_is_not_found` (in-memory, sqlite); cli `store_commands::remove_deletes_the_entry_and_exits_0`, `store_commands::remove_without_a_match_exits_2` |
| M27 backend selection and exit codes | one backend by flag or environment; exit 0 or 2 (ADR-011) | cli `store_commands::no_backend_is_a_usage_error`, `store_commands::two_backends_are_a_usage_error`, `store_commands::backend_and_catalog_id_come_from_the_environment` |
| M27 layering | the CLI parses arguments and connects; listing and removal live in storage | cli `dependency_direction::cli_dependencies_are_exactly_the_allowed_set` |
| M30 replace: exactly the document | each entry curated by the import writer, overwriting any record under its key; host overwritten; absent entries deleted | `import::replace_makes_the_catalog_hold_exactly_the_document` (store holding another writer's curated records, an expired lease, entries absent from the document and another host: afterwards `list_entries` is the document's entries, each `Curated` by the writer, `get_host` is the document's host, and `assemble` equals `assemble` of a fresh `Merge` import), `import::replace_overwrites_an_expired_lease`, `import::replace_changes_an_identifiers_version_kind` (stored `a` and `b@1`, document `a@1` and `b`), `import::replace_of_a_hostless_document_keeps_the_stored_host` |
| M30 replace: live leases | refused before writing anything when the id holds a readable, unexpired lease | `import::replace_refuses_a_live_lease_and_writes_nothing` (every lease listed; every stored record, `Etag` and the host unchanged; a lease with `now` equal to `expires_at` is not live and is replaced), `import::replace_with_corrupt_record_bytes_writes_nothing` (raw sqlite row) |
| M30 replace: write order | overwrite or add first, delete absent entries last; never empties the id | `import::replace_writes_before_it_deletes` (a recording store wrapper over the in-memory store logs every write and assembles after each: every `put_entry` precedes every `delete_entry` except the deferred creates, which follow them; after each step every document entry whose key was stored before, and whose identifier keeps its kind, is in the assembled bytes, and no step leaves the catalog without entries), `import::replace_stops_when_a_record_changed_after_the_scan` (a wrapper that rewrites one absent record between the scan and its delete: `PreconditionFailed`, that record kept), `import::replace_tolerates_an_absent_entry_deleted_after_the_scan` (a wrapper that deletes one absent record between the scan and its delete: `Ok`, the id holds exactly the document) |
| M31 replace: writes after the scan | a lease registered or refreshed under a document key after the scan is not overwritten | `import::replace_does_not_overwrite_a_lease_registered_after_the_scan` (key absent at the scan: `PreconditionFailed`, the lease kept), `import::replace_does_not_overwrite_a_lease_refreshed_after_the_scan` (expired lease at the scan, refreshed before the write: `PreconditionFailed`, the refreshed lease kept); ADR-016 `parity::overwrite_requires_the_stored_version` |
| M30 replace: unreadable records | an existing record is overwritten without reading its provenance | ADR-016 `parity::unreadable_items_are_scanned_and_overwritten`; `dynamodb::replace_upgrades_an_old_format_catalog_in_place` (old-format items written raw under id A, guards included, as the previous release writes them; `import` with `Replace` under A: `assemble(A)` equals the committed expected bytes and no item under A lacks `provenance`) |
| M30 merge unchanged | `ImportMode::Merge` is the earlier behaviour | the existing `import.rs` tests, migrated to pass `ImportMode::Merge` |
| M30 purge (CLI) | preview by default, `--yes` removes, counted by kind | cli `store_commands::purge_without_yes_previews_and_removes_nothing`, `store_commands::purge_with_yes_removes_every_item` (afterwards `store list` prints nothing), `store_commands::purge_of_an_empty_catalog_reports_zeros_and_exits_0` |
| M30 purge live-id refusal | refuses the id equal to `TURUL_AI_CATALOG_ID`, naming it | cli `store_commands::purge_refuses_the_live_catalog_id` (flag equal to the variable, and flag absent with the variable set: exit 2, stderr names `TURUL_AI_CATALOG_ID`, nothing removed), `store_commands::purge_of_another_id_with_the_live_id_set_succeeds`, `store_commands::purge_with_yes_refuses_while_the_live_id_is_unset` (exit 2, stderr names `TURUL_AI_CATALOG_ID`, nothing removed) |
| M30 import (CLI) | `store import` and `--replace`, exit codes per ADR-011 | cli `store_commands::import_writes_curated_entries_and_exits_0`, `store_commands::import_replace_removes_absent_entries`, `store_commands::import_of_a_document_with_an_error_exits_1`, `store_commands::import_replace_with_a_live_lease_exits_2` (stderr lists the lease), `store_commands::import_follows_no_nested_catalog` (a `url`-nested child on a loopback server records no request) |
| M30 upgrade routes | Route A in place; Route B by new id, then purge | `dynamodb::replace_upgrades_an_old_format_catalog_in_place` (Route A), `dynamodb::old_and_new_format_catalog_ids_share_a_table` (Route B, ending in `purge_catalog(A)`) |

Decisions owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| AMB-420 | writer id is opaque, operator-assigned, never from an address | `registration::writer_id_grammar` (accepts `[A-Za-z0-9._-]{1,128}`; refuses `10.0.0.1:443`, empty, 129 bytes) |
| AMB-421 | provenance beside record bytes, inside the entry `Etag`; guarantee 8 atomic | `provenance::provenance_bytes_are_canonical`, `provenance::provenance_round_trips`, `provenance::etag_covers_entry_and_provenance`, `parity::provenance_reads_back_exactly`, `parity::etag_is_the_digest_of_entry_and_provenance` (replaces ADR-016's `etag_is_the_digest_of_the_record_bytes` for entries), `parity::stored_entry_reads_back_byte_identical` (record bytes unchanged) |
| AMB-422 | the allow-list is configured delegation; only `identifier` is read | `registration::url_and_publisher_field_never_grant_authority` |
| AMB-423 | no prefix can admit `urn:air`; malformed `urn:air` refused | `registration_policy::urn_air_prefix_is_refused`, `registration::malformed_urn_air_is_refused` |
| AMB-424 | ownership per key until deleted, no takeover after expiry; co-granted writers share an identifier's versions, never each other's records | `parity::another_writer_cannot_take_over_an_entry`, `registration::co_granted_writers_share_an_identifier_but_not_its_records` |
| AMB-425 | signature gate on every write by a requiring grant; seam trait in storage, implementation in trust | `registration::refresh_with_an_expired_signature_does_not_extend_the_lease`; cli `dependency_direction::trust_optional_features_add_only_storage_async_trait_tokio_and_the_ssm_client` (also pins storage without default features and `tokio` with `rt` only), `dependency_direction::storage_default_features_add_no_runtime_or_network_dependency` (kept: no storage feature depends on trust or client) |
| AMB-426 | per-entry gate failures omitted; cross-entry errors still fail | `assembly::entry_failing_the_current_gate_is_omitted_and_reported`, `assembly::cross_entry_error_still_fails_assembly` |
| AMB-427 | earlier records are not read as curated; re-import | `sqlite::table_from_an_earlier_release_fails_connect`, `postgres::table_from_an_earlier_release_fails_connect`, `dynamodb::item_without_provenance_is_corrupt` |
| AMB-428 | check order of §3.2; local refusals do no I/O, state refusals no network | `registration::checks_run_in_the_documented_order` (one request per step, each failing only that step; the stub verifier is called only at step 6, the store only from step 5) |
| AMB-429 | refresh interval and clock skew against lease length are documented, not checked | none (no behaviour) |
| AMB-430 | a refresh writes only when due (`slack = lease_ttl / 4`) | `parity::many_instances_refreshing_one_lease_all_succeed`, `registration::refresh_is_a_no_op_until_due` |
| AMB-431 | upgrade under a new catalog id in the same table; formats do not disturb each other | `dynamodb::old_and_new_format_catalog_ids_share_a_table` (old-format items written raw under id A as the previous release writes them, guards included; `import` under id B: `assemble(B)` equals the committed expected bytes, A's items are byte-identical afterwards, `assemble(A)` fails `CorruptProvenance` and `assemble(B)` is unchanged; `purge_catalog(A)` reports A's items by kind, leaves no item under A, and leaves `assemble(B)` and every item under B unchanged) |
| AMB-435 | replace orders writes so no document entry goes missing; refuses live leases first | `import::replace_writes_before_it_deletes`, `import::replace_refuses_a_live_lease_and_writes_nothing` |
| AMB-436 | purge and its preview act on keys only; the CLI refuses the live id | ADR-016 `parity::unreadable_items_are_counted_and_purged`; cli `store_commands::purge_refuses_the_live_catalog_id` |
| AMB-437 | `import` is a `store` subcommand | cli `store_commands::import_writes_curated_entries_and_exits_0` |
| AMB-432 | the snapshot refreshes at the earlier of `max_age` and the next lease expiry | `assembly::next_expiry_is_the_earliest_served_lease_expiry` (expired and curated records do not count; `None` without leases); lambda `snapshot_refresh::snapshot_is_reassembled_at_the_next_lease_expiry` (`max_age` one hour, a lease expiring in ten seconds: a request at its expiry re-assembles and the lease is gone), `snapshot_refresh::failed_refresh_after_an_expiry_is_retried_once_per_max_age` |
| AMB-434 | curated registration is granted by the operator; a lease-only grant asking for it is refused before any I/O | `registration::curated_registration_needs_a_curated_grant`, `registration::checks_run_in_the_documented_order` |
| AMB-433 | a grant's publishers pass the root `did:web` domain rule, one definition shared with `DidWebRoot` | `registration_policy::publisher_that_is_not_a_root_did_web_domain_is_refused` (`Example.com`, `example.com.`, `example.com:8443`, `10.0.0.1`, `0x7f.1`, `bücher.example`, `-a.example`, empty: `InvalidPublisher`; `xn--bcher-kva.example` accepted); trust `did_web_signer.rs` stays green on the moved check |
| deployment | register once serving; refresh failures reported, not fatal; `lease_ttl ≥ 4 ×` refresh interval; operator removal by `EntryKey` | none (deployer guidance, §7) |
| clock | injected `now`; storage never reads the clock | `registration::storage_source_never_reads_the_clock` |
| contention | bounded retries, then `Contended` | `registration::contention_beyond_the_attempt_bound_is_reported` (a store double that always fails `Matches`), `registration::store_contention_is_reported_as_contended` (a store double that returns `StoreError::Contended`), `dynamodb::conflicts_on_every_attempt_are_contended` |
| lease bound | no stored expiry past year 9999; no panic on overflow | `registration_policy::lease_ttl_past_year_9999_is_refused`, `registration::lease_ending_past_year_9999_is_refused_before_any_io`, `provenance::provenance_that_would_not_read_back_is_refused`, `parity::provenance_that_would_not_read_back_is_refused` |
| registrar ownership | the `Registrar` owns its store and verifier | the `Registrar` doc example (an `Arc<dyn CatalogStore>`); every `registration.rs` and `parity` registrar is built from `Arc`s |

New ids, block AMB-420 to AMB-439:

- AMB-420: the spec has no writer concept. An operator-assigned opaque id lets any authentication scheme sit in front, and keeping addresses out of the API makes "re-registration from a new address is an update" true by construction. The `Registrar` cannot authenticate; claiming otherwise would make store credentials look like authority, which the F1 input rejects.
- AMB-421: three placements were weighed. Inside the record bytes would change what `parse_entry` reads and what assembly serves. Beside the bytes and outside the `Etag` leaves a race: a lease refreshed with unchanged content keeps its `Etag`, so a stale reader's `Matches` overwrites a refreshed lease. Beside the bytes and inside the `Etag` keeps served bytes and Trust Manifest payloads unchanged and makes `Matches` cover writer and expiry. Mode exclusion must also hold for `Condition::Any` writers, which have no read to compare, so it is backend guarantee 8; it needs one more predicate in each backend's existing conditional step.
- AMB-422: #catalog-projection defines a publisher-authorized identifier as one "assigned by the artifact publisher or its authorized delegate" and forbids inference from URL or `publisher`. An operator's grant is an explicit, out-of-band configuration of who is a delegate, not an inference. It is weaker than a signature, so the signature requirement is available per grant.
- AMB-423: rule 3 requires a non-publisher, non-delegate operator to use non-`urn:air` identifiers. A prefix is the operator's own namespace, so it must not reach into `urn:air`. ADR-004 compares identifiers exactly, but some consumers may treat the URN scheme and NID case-insensitively (RFC 8141), so `URN:AIR:` is refused too.
- AMB-424: allowing a writer the operator did not configure for an identifier to take over an expired lease would let that identifier name another artifact without the operator acting, against "MUST NOT be reassigned". Keeping ownership of a record until deletion leaves reassignment of that record an explicit operator act (a store-credential delete). The unit of ownership is the record key, not the identifier. #catalog-projection places authority over a publisher-authorized `urn:air` identifier with "the artifact publisher or its authorized delegate", and the grant is the operator's configuration of who is a delegate (AMB-422); two writers granted one publisher are both its delegates, so the spec gives neither a stronger claim to the identifier than the other, and keeping their versions of it consistent is the delegating publisher's duty. Per-identifier ownership (the writer stored on the version-kind guard and checked in the backends' atomic step) was weighed and not built: it is a new backend guarantee in four backends, nothing in the spec requires it, and an operator that wants one writer per identifier gets it by granting disjoint publishers and prefixes. The consequence is stated, not hidden: a co-granted writer can add a version under an identifier another writer registered, which the catalog serves once the first writer's lease lapses, and the first record fixes the identifier's version kind for both (M21).
- AMB-425: a storage feature depending on the trust crate would put JWS, `did:web` and network dependencies into storage and break ADR-016's pin. A trait in storage implemented by the trust crate under its own feature keeps storage free of crypto, has no cycle, and gives the trust crate one per-entry procedure for both report and gate. Verifying on refresh costs one DID document fetch and, for `url` entries, one artifact fetch per refresh per instance; no cache is built (a cache would be a new control).
- AMB-426: an entry gated by an earlier release can fail a rule added later. Failing the whole assembly for it would take every other entry offline; omitting and reporting it keeps the rest served and visible. A cross-entry conflict has no spec-defined loser, so assembly still refuses it.
- AMB-427: under this repository's no-compatibility rule, re-import replaces migration. Reading a missing provenance as `Curated` would be a permanent legacy path with no owner to check.
- AMB-428: as ADR-015 AMB-331: checks that need nothing come first, so a refused request never reaches the store or the network, and each refusal has one deterministic reason. The store read precedes verification so that a mode or ownership refusal never causes a fetch.
- AMB-429: §6.
- AMB-430: with "the later of stored and `now + lease_ttl`", N instances passing distinct `now` values each compute a distinct later expiry, so up to N writes chain and any N above the 8-attempt bound can return `Contended`; whether they all return `Ok` depended on timing. Writing only when due makes equal-intent refreshes compute equal state, bounds commits by the spread of clocks rather than the number of instances, and cuts write cost to at most one per `slack` per lease. `slack = lease_ttl / 4` keeps a written lease at least three quarters of its length ahead of the writer's clock.
- AMB-431: an in-place `Merge` re-import of a live catalog id cannot finish: `write_owned` reads each existing record, whose provenance does not read. A new id in the same table (Route B) keeps the old function serving until the new one is deployed, needs no new table or grant (the grants cover the table, ADR-016 §7), and makes rollback a redeploy until the old id is purged. Since M30, Route A replaces in place (§4.8), with the new function deployed only after the replace returns, at the cost of rollback by redeploy; Route B's last step is `store purge`, which reads none of the old items, instead of a hand-written delete by partition key.
- AMB-432: `max_age` alone serves an expired lease up to a whole `max_age` after it expires. Assembly already reads every expiry, so refreshing at the next one costs no extra read and no new control. Keeping the last good snapshot on a failed refresh (AMB-407) is kept, at the cost stated in §6.
- AMB-434: M26's two modes differ in who ends an entry: a lease ends unless refreshed, a curated entry stays until removed and, under a signature-requiring grant, is never verified again (AMB-425). If any granted writer could pick `Curated`, a runtime writer could make its entry permanent and escape re-verification, and the operator could not prevent it. A grant therefore permits leases by default, and curated registration is an explicit operator setting. The check needs only the grant and the request, so it joins step 1 and runs before any I/O (AMB-428).
- AMB-433: a grant naming a publisher that fails the domain rule can never be backed by a publisher signature (#the-did-web-publisher-profile), so it is a configuration error, refused at construction. Copying the check into storage would be a second definition, and depending on trust would break ADR-016's pin; the core crate is the one both already depend on.
- AMB-435: a replace that deletes first (purge, then import) serves an empty or partial catalog while it runs. Overwriting in place first keeps every document entry served, and deleting absent entries last only removes what the document no longer holds. Overwrite must not read the old record, because the case it exists for is a record this release cannot read; it is therefore a store condition (ADR-016 `Condition::Overwrite`), not a read-then-`Matches` above the trait. Identifying absent records needs their `EntryKey`, which `scan_entries` takes from the record bytes (one format since ADR-016) while reporting unreadable provenance instead of failing. A live lease is a registered claim the operator granted (§3); overwriting it as curated would end a writer's entry without the writer or an explicit removal, so it is refused, and Route B or removal is the operator's explicit act. A version-kind change cannot be written before the other kind is gone (M21); deferring those creates past the deletions is the one point where an identifier is briefly missing, which assembly serves without error.
- AMB-436: a purge exists to remove items this release cannot read, so it may read nothing but keys; counting by key kind gives the operator a preview that cannot fail on old data. The live-id check is a front-end refusal of the most damaging slip (emptying the id the function serves); it compares with the environment the Lambda and the store commands already share, and is not authority. Route B no longer needs a hand-written delete by partition key.
- AMB-437: M30 names `import --replace`; the CLI had no import command. Placing it under `store` reuses the one backend group and its environment variables (`--catalog-id` included) instead of a second definition at top level.

## Consequences

- Every entry write now states a mode and a writer, and every entry `Etag` changes value (it now covers provenance). Callers of `put_entry`, `get_entry`, `list_entries`, `import` and `assemble` migrate in the same slices; ADR-016's parity checks that compare `Etag` with record bytes are rewritten for entries.
- Existing stores must be replaced in place or re-imported under a new id, by a route of §4.7. A new-release function pointed at a catalog id neither replaced nor re-imported fails cold start, or keeps its last good snapshot; the previous release must never be pointed at a new-format id.
- The curated writer id is permanent configuration (§4.5); losing it costs an `import --replace`, or Route B while live leases exist.
- `import` takes an `ImportMode`; every caller passes `Merge` except `store import --replace`. The trait gains `scan_entries`, `count_catalog`, `purge_catalog` and `Condition::Overwrite` (ADR-016), which an implementation outside this crate must add.
- SQL deployments gain two columns; DynamoDB writers need no new IAM action (`GetItem` and conditional `PutItem` are already granted).
- A runtime registration costs one read and one conditional write, plus verification when required. Assembly costs are unchanged apart from a second validation pass when the first finds errors.
- Expired leases keep their keys and version kinds until removed; a long-dead service's identifier cannot be registered in the other version kind until its record is deleted.
- Authority is only as strong as the boundary around store credentials (§2). The library cannot make a writer with store credentials obey a grant.
- The trust crate gains an optional dependency on storage under `registration`; its default build is unchanged.

## Open questions (for the maintainer; not built)

1. **Reaper.** Expired leases remain until removed (§6). Removing them automatically (a periodic sweep, or a DynamoDB TTL attribute) is a new control: it deletes records nobody asked to delete. Without it, nothing breaks today: expired leases are omitted from every assembly and reported. The sentence to approve: "Add a reaper that deletes lease records expired for longer than a configured grace period."
2. **Registration endpoint.** The library has no transport. A deployer builds the authenticated channel in front of the `Registrar` (§2). Shipping one (an HTTP handler in the Lambda crate, say) is a new feature and needs its own approval.
3. **CLI store command.** Approved (M27) and specified in §8.

The earlier question of a `max_age` check is withdrawn: with §6's snapshot rule, `max_age` no longer bounds how long an expired lease is served.

## Slices

Each slice is ADR updates, then tests from this ADR, then code, then gates, then a patch bump with `CHANGELOG.md`.

| Slice | Content | Updates |
|---|---|---|
| R1 | `WriterId`, `Provenance`, `ModeKind`, `Etag::of_entry`, `StoredEntry`; `put_entry` takes a provenance; guarantee 8 in all four backends with the schema changes of §4.6 and the earlier-table probe; `write_owned` (curated paths; its lease rule is exercised in R3); `import` takes a writer, records `Curated` and refuses another writer's records (`import_under_another_writer_id_is_refused`); `ServedCatalog::from_document` passes its constant; parity checks for provenance, mode exclusion and racing modes; `dynamodb::old_and_new_format_catalog_ids_share_a_table` | ADR-016 (trait, guarantee 8, §6 schemas, `Etag`, `import`), ADR-010 (`from_document`), README (the §4.7 upgrade sequence, the warning against the previous release, the permanent curated writer id and its recovery where `import` is documented), `CHANGELOG.md` opening with the §4.7 breaking line, revision logs |
| R2 | `assemble(.., now)` with lease expiry, `Omission`, `next_expiry`, the `Invalid` second pass; `SnapshotCache` passes `now`, refreshes at the earlier of `max_age` and `next_expiry`, and logs omissions; assembly, parity and lambda tests including AMB-432's | ADR-016 §4 and §7 (freshness), ADR-010, README (the AMB-429 rule) |
| R3 | `is_root_did_web_domain` moved to the core crate and called by `DidWebRoot::parse`; `RegistrationPolicy`, `WriterGrant` (domain check), `PolicyError`, `PublisherVerifier`, `Registrar::register` and `unregister` with the §3.2 order, the due-refresh rule of §4.3 and `Contended`; `registration_policy.rs`, `registration.rs`, registration parity checks on every backend including `many_instances_refreshing_one_lease_all_succeed` | ADR-004 (`is_root_did_web_domain`), ADR-012 (`DidWebRoot::parse` calls it), README (§7 deployment: role separation first, registration timing, refresh failures and `lease_ttl ≥ 4 ×` interval, removal snippet), this ADR's revision log if anything changes |
| R4 | trust `verify_entry`; feature `registration` with `DidWebPublisherVerifier`; `publisher_verifier.rs`; `dependency_direction` pin | ADR-015 (`verify_entry`), ADR-011 (`dependency_direction`) |
| R5 | `list_registrations`, `remove_registration`; `turul-ai-catalog store list` and `store remove` (§8); `operator.rs`, `store_commands.rs`, `dependency_direction` update | ADR-011 (the `store` command), README of the CLI and storage, this ADR's revision log |
| R6 (M30, 0.0.23) | `scan_entries`, `count_catalog`, `purge_catalog`, `Condition::Overwrite` in all four backends (ADR-016 §6); `ImportMode` and the §4.8 replace; `store import` and `store purge` with the live-id refusal (§8); the M30 parity, `import.rs`, `dynamodb.rs` and `store_commands.rs` tests | ADR-011 (`store import` reads through `load`; exit 1 for `ImportError::Invalid`), README of the CLI and storage (§4.7 routes replace the delete-by-partition-key step; writer-id recovery by `--replace`), `CHANGELOG.md`, the version |
| R7 (M31, 0.0.24) | `Condition::Overwrite(Etag)` in all four backends; §4.8 step 4 conditioned on the scan (`Overwrite(scanned etag)` or `Absent`), step 6 `Absent`; `store purge --yes` refused while `TURUL_AI_CATALOG_ID` is unset (§8); the M31 parity, `import.rs` and `store_commands.rs` tests | ADR-016 (§3, §6, tests), CLI README, `CHANGELOG.md`, the version |

R1 and R2 change behaviour for every deployer (re-import, new `assemble` signature); R3 and R4 add API only; R6 changes `import`'s signature and adds trait methods; R7 changes `Condition::Overwrite` to carry the expected version.

## What this ADR does not do

- No code, tests or `Cargo.toml` changes: they follow in R1 to R7.
- No reaper, no TTL attribute, no registration endpoint or transport, no `max_age` check, no verification cache, no authentication of writers, no cross-process check of snapshot age, no migration of earlier records.
- No change to who may write a host: `put_host` and `import` stay operator paths.
- No serving of omissions or provenance to clients: the served document is unchanged in shape.
- No identifier assignment, rewriting or migration mechanism (#catalog-projection: "requires an explicit migration mechanism, which this specification does not define").
- Names no consumer of the library and decides nothing in any other repository.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created under M26: `Registrar` over `CatalogStore` with `RegistrationPolicy` and `WriterGrant` (publisher and prefix allow-lists, refusal when neither, optional signature); opaque `WriterId`; `Provenance` (lease or curated, with writer) stored beside record bytes and covered by the entry `Etag`; backend guarantee 8 (mode exclusion) and schema changes for all four backends; idempotent lease refresh by read-then-`Matches`; `assemble` with injected `now` and an omission report; `PublisherVerifier` seam implemented by the trust crate under feature `registration` through a new `verify_entry`; re-import for earlier stores. AMB-420 to AMB-429 added. Reaper, endpoint and `max_age` check left to the maintainer. |
| 2026-11-17 | Review, upgrading a live store: §4.7 states the supported sequence (import under a new `CatalogId` in the same table, deploy the function on that id, delete the old id's items), that the previous release must never be pointed at a new-format id, and that the R1 `CHANGELOG.md` entry opens with a breaking line for store holders; SQL uses a new database. AMB-431, pinned by `dynamodb::old_and_new_format_catalog_ids_share_a_table` (DynamoDB only: no other backend can hold both formats). |
| 2026-11-17 | Review, refresh under many instances: a lease refresh writes only when the stored expiry is earlier than `now + lease_ttl − slack`, `slack = lease_ttl / 4`, otherwise returns `Ok` with the stored `Etag`; an update keeps the later-of rule. The claim that N instances all return `Ok`, which held only by timing, is replaced by the derivation in §4.3. AMB-430; `parity::concurrent_lease_refreshes_converge` and `registration::identical_refresh_writes_nothing` replaced by `parity::many_instances_refreshing_one_lease_all_succeed` (64 instances, distinct `now`) and `registration::refresh_is_a_no_op_until_due`. |
| 2026-11-17 | Review, registration timing: new §7 states that a service registers once serving, never during init, that a refresh failing on an unreachable DID document or artifact is reported, not fatal, while the lease has time left, and recommends `lease_ttl ≥ 4 ×` the refresh interval. |
| 2026-11-17 | Review, curated writer id: §4.5 states it is permanent configuration, that `import` refuses another writer's curated records, and the recovery (delete the catalog id's items with store credentials, re-import). Pinned by `import::import_under_another_writer_id_is_refused`; the README documents it with `import`. |
| 2026-11-17 | Review, stale lease removal: §7 gives the operator's removal by `EntryKey` with `CatalogStore::delete_entry` conditioned on the read `Etag`, or the owner's `Registrar::unregister`. A CLI store command is open question 3, not built; the `max_age` check question is withdrawn (see the snapshot row). |
| 2026-11-17 | Review, publisher grants: `WriterGrant::new` refuses a publisher that is not a lowercase ASCII root `did:web` domain, through `turul_ai_catalog::identity::is_root_did_web_domain`, moved from `DidWebRoot::parse`, which calls it. AMB-433, pinned by `registration_policy::publisher_that_is_not_a_root_did_web_domain_is_refused`; ADR-004 and ADR-012 change in R3. |
| 2026-11-17 | Review, deployer guidance: §7 opens with "authority holds only if no registering service's role can write the table; the registrar runs under its own role". |
| 2026-11-17 | Review, snapshot freshness: `Assembled` gains `next_expiry` from data assembly already reads; `SnapshotCache` refreshes at the earlier of `max_age` and `next_expiry`, so an expired lease is not served past its expiry (except while a refresh fails, AMB-407). In-process only; no cross-process check. AMB-432 and AMB-429 restated; pinned by `assembly::next_expiry_is_the_earliest_served_lease_expiry` and two `snapshot_refresh` tests. |
| 2026-11-17 | Review, slices: every change above is assigned (R1: upgrade sequence, breaking line, writer-id permanence; R2: snapshot freshness; R3: refresh rule, domain check, deployment guidance and removal). |
| 2026-11-17 | R2 landed: `SnapshotCache::new` takes the clock whose reading it passes to `assemble`, and `Omission` implements `Display` for the per-assembly log line. `entry_failing_the_current_gate_is_omitted_and_reported` writes its invalid row raw into a sqlite file, since no public constructor builds a record that fails the gate; it runs with feature `sqlite`. |
| 2026-11-17 | R3 landed: `write_owned` takes the mode (`Curated`, or `Lease` with the policy's length), the optional verifier and `now`, and owns the refresh and update rules of §4.3, `import` passing `Curated`. The verifier of §3.2 step 6 runs once, immediately before the first put that would commit; a refresh that is not due, or a curated record with equal bytes, writes nothing and verifies nothing, since nothing is claimed or extended. Retries do not verify again. `parity::many_instances_refreshing_one_lease_all_succeed` takes `Arc<dyn CatalogStore>` and wraps it in a counting store. |
| 2026-11-17 | R4 landed: trust `verify_entry` and `verify_entry_resolved` (the per-entry procedure, DID documents injected, so acceptance is testable without the network), feature `registration` with `DidWebPublisherVerifier::new(policy)` and `publisher_verdict(&EntryReport)`, which the verifier applies to `verify_entry` on a blocking thread; the feature is in CI and docs.rs metadata; `dependency_direction` pins the feature's dependency set. `PublisherVerifier` and `VerifierRefusal` were already in storage from R3. |
| 2026-11-17 | Operator store command (M27): new §8 specifies `turul-ai-catalog store list` and `store remove`, backend by flag or the Lambda's environment variables, exit codes 0 and 2, and the storage owners `list_registrations` and `remove_registration`; open question 3 is answered and §7 points to it. Slice R5. |
| 2026-11-17 | Review, curated mode is granted: `WriterGrant` gains `curated`, false from `WriterGrant::new` and set by `with_curated`; §3.2 step 1 refuses a `Curated` request under a lease-only grant with `RegistrationError::CuratedNotGranted` before any I/O. Before this, every granted writer could pick `Curated`, making its entry permanent and, under a signature-requiring grant, never re-verified. §3.1, §3.2, §4.5 and §5 updated. AMB-434, pinned by `registration::curated_registration_needs_a_curated_grant` and a step-1 case in `registration::checks_run_in_the_documented_order`. |
| 2026-11-17 | Review, ownership unit: ownership stays per record key. #catalog-projection gives authority over a publisher-authorized `urn:air` identifier to "the artifact publisher or its authorized delegate", and a grant configures delegates (AMB-422), so writers granted one publisher share its identifiers' versions and version kind and cannot overwrite or remove each other's records. §3.1 and §3.3 and AMB-424 now say so plainly; per-identifier ownership on the guard record was weighed and not built (a new backend guarantee the spec does not require; disjoint grants give one writer per identifier). Pinned by `registration::co_granted_writers_share_an_identifier_but_not_its_records`. |
| 2026-11-17 | Registration review. Lease bound: `RegistrationPolicy::new` refuses `PolicyError::LeaseTtlOutOfRange` for a length whose lease from the Unix epoch ends past year 9999; `write_owned` computes the expiry with checked addition and returns `RegistrationError::ExpiryOutOfRange` before any I/O instead of panicking; `Provenance::to_bytes` and `Etag::of_entry` return `Result` and refuse a provenance whose bytes would not read back, and every backend's put refuses it with `CorruptProvenance`. One canonical form: provenance bytes and the entry `Etag` preimage go through `Provenance::to_json`, `CatalogEntry::to_json` and `Json::to_canonical_vec`; the hand-built strings are deleted, and the bytes are unchanged (they were already JCS). `Registrar` owns `Arc<dyn CatalogStore>` and `Option<Arc<dyn PublisherVerifier>>` and has no lifetime. A store's own exhausted retries (`StoreError::Contended`) are `RegistrationError::Contended`. §5's dependency statement was right and the code was not: the trust `registration` feature took storage with its default features and `tokio` with `macros`; the root dependency on storage now has `default-features = false` (the Lambda asks for `in-memory`), the root `tokio` has `rt` only (`macros` is asked for where used), and `dependency_direction` pins both. |
| 2026-11-18 | M30: §4.7 gives two upgrade routes. Route A keeps the catalog id: stop the previous release's writers, `turul-ai-catalog store import --replace` against the live id, then deploy the new function. Route B takes a new id: import under it, repoint the function, then `turul-ai-catalog store purge --catalog-id OLD --yes`. The hand-written delete by partition key is gone, and a release changing the stored format states which route applies. New §4.8: `ImportMode::{Merge, Replace { now }}`; `Replace` gates as `Merge`, scans with `scan_entries`, refuses with `ImportError::LiveLeases` before any write when a readable unexpired lease is stored, overwrites the host, writes each entry curated with `Condition::Overwrite` (never reading the old record), deletes absent entries conditioned on their scanned `Etag`, and only then creates entries whose version kind changed; what a concurrent assembly observes is stated. §8 adds `store import [--replace]` (under `store`, AMB-437) and `store purge` (preview by default, `--yes` to remove, refused when the id equals `TURUL_AI_CATALOG_ID`), with ADR-011 exit codes (1 only for `ImportError::Invalid`). §4.5's writer-id recovery uses `--replace`. AMB-431 revised; AMB-435 to AMB-437 added; slice R6. Review: §4.8 step 5 states that a record deleted after the scan fails its conditioned delete as `PreconditionFailed`, and the import confirms it absent with one `get_entry` before going on (it had stopped the import); the M30 rows cite the parity checks by their names. |
| 2026-11-18 | M31: §4.8 step 4 conditions each write on the scan: `Condition::Overwrite(scanned etag)` for a key the scan held, `Condition::Absent` for one it lacked; step 6 uses `Absent`. A lease registered or refreshed after the scan under a document key now stops the import with `PreconditionFailed` instead of being overwritten; "Concurrent writers" restated. §8: `purge --yes` refuses while `TURUL_AI_CATALOG_ID` is unset, so the live-id refusal no longer depends on the operator having set it. New tests `import::replace_does_not_overwrite_a_lease_registered_after_the_scan`, `import::replace_does_not_overwrite_a_lease_refreshed_after_the_scan`, `store_commands::purge_with_yes_refuses_while_the_live_id_is_unset`. |
| 2026-11-18 | The trust-features dependency test is renamed `dependency_direction::trust_optional_features_add_only_storage_async_trait_tokio_and_the_ssm_client`, since ADR-018's `ssm` feature joins `registration` in it. |
