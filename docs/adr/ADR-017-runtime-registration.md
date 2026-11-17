# ADR-017: Runtime registration: writers, authority, lease and curated modes

- **Status:** Proposed (accepted when every test file below is green, the `parity` registration checks for `in-memory` and `sqlite` in CI and for `postgres` and `dynamodb` against a live service; every entry write in the workspace carries a `Provenance`; no `put_entry` call site, `import` or `ServedCatalog::from_document` writes an entry without one; and `assemble` is the only code that decides whether a stored lease is served)
- **Date:** 2026-11-17
- **Implements:** under `crates/turul-ai-catalog-storage/tests/`: `provenance.rs`, `registration_policy.rs`, `registration.rs`, new checks in `parity/mod.rs` called by `in_memory.rs`, `sqlite.rs`, `postgres.rs` and `dynamodb.rs`, and migrated `assembly.rs`, `import.rs`, `records.rs`. Under `crates/turul-ai-catalog-trust/tests/`: `publisher_verifier.rs` (feature `registration`). Under `crates/turul-ai-catalog-lambda/tests/`: migrated `snapshot_refresh.rs`, `cold_start.rs`. `crates/turul-ai-catalog-cli/tests/dependency_direction.rs` gains `trust_registration_feature_adds_only_storage_async_trait_and_tokio` and keeps `storage_default_features_add_no_runtime_or_network_dependency`.
- **Depends on:** ADR-016 (`CatalogStore`, `CatalogId`, `EntryRecord`, `RecordKey`, `Etag`, `Condition`, `StoreError`, guarantees 1 to 7, `assemble`, `import`, the four backends, the parity suite, `SnapshotCache`; changed by §4 to §7), ADR-015 (`TrustPolicy`, `verify_signature`, `entry_release_coverage`, `publisher_authorized`, `ManifestReport::endorsement`; gains `verify_entry`, §5), ADR-004 (`UrnAir`, `EntryKey`, `EntryIdentifier`), ADR-005 (`validate`, `Timestamp`), ADR-010 (the Lambda). Binding decisions: **M26** (this ADR's mandate), M19, M21, M24, M10, M6, M13.
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
pub struct Registrar<'s> { /* store, catalog, policy, verifier */ }

impl<'s> Registrar<'s> {
    pub fn new(
        store: &'s dyn CatalogStore,
        catalog: CatalogId,
        policy: RegistrationPolicy,
        verifier: Option<&'s dyn PublisherVerifier>,
    ) -> Result<Registrar<'s>, PolicyError>;

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

### 2. Writer identity

- `WriterId::new(impl Into<String>) -> Result<WriterId, RecordError>`: 1 to 128 bytes of `[A-Za-z0-9._-]`, the `CatalogId` grammar, compared exactly. It is assigned by the operator and is opaque.
- The library trusts nothing about it beyond what the deployer authenticated. The deployer authenticates the caller by its own means (an IAM principal, a mutual-TLS certificate, a signed request) and maps that principal to a `WriterId`. The `Registrar` takes the `WriterId` as given.
- A `WriterId` is never derived from a network address, host name, URL or any entry member. Nothing in the API accepts an address. A service that moves to a new address and registers the same identifier with the same `WriterId` updates its entry (§3, #catalog-projection "retrieval-URL changes").
- Enforcement boundary (AMB-420): the `Registrar` constrains a writer only when that writer cannot reach the store without it. A process holding store write credentials can call `put_entry` directly and bypass every check in §3. Store write access is therefore operator power, and authority under this ADR holds only when the `Registrar` runs in a process the operator controls and writers reach it through an authenticated channel. This library ships no such channel (open question 2).

### 3. Authority

#### 3.1 Policy

```rust
pub struct RegistrationPolicy { /* grants: BTreeMap<WriterId, WriterGrant>, lease_ttl: Duration */ }
pub struct WriterGrant { /* publishers: BTreeSet<String>, prefixes: Vec<String>, require_publisher_signature: bool */ }

impl RegistrationPolicy {
    pub fn new(grants: impl IntoIterator<Item = (WriterId, WriterGrant)>, lease_ttl: Duration)
        -> Result<RegistrationPolicy, PolicyError>;
}
impl WriterGrant {
    pub fn new(publishers: impl IntoIterator<Item = String>, prefixes: impl IntoIterator<Item = String>,
               require_publisher_signature: bool) -> Result<WriterGrant, PolicyError>;
}

#[non_exhaustive]
pub enum PolicyError {
    EmptyGrant,                         // neither publishers nor prefixes
    InvalidPublisher(String),           // empty, or contains ':'
    EmptyPrefix,
    UrnAirPrefix(String),               // starts with "urn:air:" in any ASCII case
    ZeroLeaseTtl,
    DuplicateWriter(WriterId),
    VerifierMissing,                    // a grant requires a signature and no verifier was given
}
```

- **Publishers** are `urn:air` `{publisher}` components, compared byte for byte with `UrnAir::publisher()` (ADR-004, M24). No case folding, no subdomain or suffix match (#the-did-web-publisher-profile; AMB-27). A publisher that is empty or contains `:` could never equal a parsed component and is refused at construction. The root `did:web` domain restrictions are not applied here: they belong to `DidWebRoot` in the trust crate, which storage does not depend on; a grant that requires a signature gets them through the verifier (§5).
- **Prefixes** admit non-`urn:air` identifiers whose text starts with the prefix, byte for byte. A prefix starting with `urn:air:` in any ASCII case is refused, so no prefix can admit an identifier that looks like `urn:air` (#catalog-projection rule 3, AMB-423).
- **No silent allow-all.** A grant must hold at least one publisher or prefix (`EmptyGrant`), and a writer without a grant is refused at `register` and `unregister` (`UnknownWriter`). There is no wildcard publisher or empty prefix (`EmptyPrefix`).
- `lease_ttl` is set by the operator, not requested by the writer, so a writer cannot hold a lease for longer than the operator allows.
- `Registrar::new` refuses `VerifierMissing` when any grant sets `require_publisher_signature` and `verifier` is `None`, so the requirement can never be configured and silently skipped.

What the allow-list means (AMB-422): configuring publisher `example.com` for a writer is the operator's assertion that this writer is that publisher or its authorized delegate. It is configured policy, not inference. #catalog-projection forbids inferring authorization "from the artifact's URL or an unsigned `publisher` field", and the `Registrar` never reads `url`, `publisher` or any other entry member for authority; only `identifier` is checked. Where the operator wants the publisher itself to prove it, the grant sets `require_publisher_signature`.

#### 3.2 `register` checks, in order

The first failure is returned. Every check that needs no I/O runs before the store read, and the store read runs before any network work, so a request that local policy refuses never causes a fetch (AMB-428).

| Step | Check | Failure |
|---|---|---|
| 1 | `writer` has a grant | `RegistrationError::UnknownWriter` |
| 2 | `EntryRecord::new(entry)` (ADR-016 record gate) | `Record(RecordError)` |
| 3 | identifier: if `UrnAir::parse` succeeds, its publisher is in the grant's publishers; if it fails and the identifier starts with `urn:air:` in any ASCII case, refuse; otherwise the identifier starts with one of the grant's prefixes | `PublisherNotAllowed { publisher }`, `MalformedUrnAir`, `IdentifierNotAllowed` |
| 4 | if the grant requires a signature, the identifier is `urn:air` (the profile's first requirement) | `SignatureRequiresUrnAir` |
| 5 | `get_entry` of the key; when a record exists: its mode kind equals the requested one, and its writer equals `writer` | `ModeConflict { stored }`, `OwnedByAnotherWriter` |
| 6 | if the grant requires a signature, `verifier.verify_publisher(entry, now)` | `PublisherSignature(VerifierRefusal)` |
| 7 | the conditional write (§4.3) | `Store(StoreError)`, `Contended` |

`unregister` runs steps 1 and 3 on the key's identifier, then step 5's writer check (mode is not checked: a writer may remove its own entry in either mode), then `delete_entry` with `expected: Some(etag)`. A missing record is `NotFound`. A `PreconditionFailed` re-reads and repeats from step 5, within the attempt bound of §4.3. Authority is evaluated against the current policy: a writer whose grant no longer covers an identifier cannot remove it, and the operator removes it with store credentials. No signature is required to remove: removal makes no claim about the artifact.

#### 3.3 #catalog-projection obligations

| Obligation | How it holds |
|---|---|
| preserve a publisher-authorized `urn:air` identifier exactly | the `Registrar` never assigns, rewrites or normalises an identifier; the record holds the writer's identifier byte for byte (ADR-016 §4) |
| never mint `urn:air` under another domain | the `Registrar` mints nothing; a `urn:air` identifier is accepted only under a publisher the operator granted to that writer, and no prefix can admit one (§3.1) |
| MUST NOT infer authorization from URL or unsigned `publisher` | only `identifier` is checked (§3.1); with a signature required, authorization is the did:web publisher profile (§5) |
| identifier stable, never reassigned to another artifact | a stored entry belongs to the writer that created it until it is deleted, expired or not (AMB-424); no other writer can overwrite it. Whether two registrations of one identifier name the same artifact is not observable to the library; it is the writer's duty, as ADR-016 §4 states for the store |
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
    pub fn to_bytes(&self) -> Vec<u8>;                         // JCS
    pub fn parse(bytes: &[u8]) -> Result<Provenance, ProvenanceError>;
}
```

- Bytes are the JCS (ADR-003 `Json::to_canonical_vec`) of `{"mode":"curated","writer":W}` or `{"expiresAt":T,"mode":"lease","writer":W}`, where `T` is `Timestamp`'s RFC 3339 `Display` (ADR-005). `Provenance::parse` accepts exactly these members; anything else is `ProvenanceError`. `expires_at` compares as an instant, never as a string.
- `Provenance` is the one owner of these bytes. The schema is these two object shapes; `provenance::provenance_bytes_are_canonical` pins them against committed byte strings, which serve as its digest.

#### 4.2 Where the mode lives: beside the record bytes, inside the `Etag` (AMB-421)

- **Record bytes are unchanged**: still the JCS of the entry's own JSON, read by `parse::parse_entry` (ADR-016 §2). The served document and the entry's Trust Manifest payloads never see provenance.
- **Provenance is stored beside them**, one value per entry record, with its mode kind as a separate column or attribute so a backend can test it in a write condition.
- **The entry `Etag` covers both**: `Etag::of_entry(&EntryRecord, &Provenance)` is the lowercase hex SHA-256 of the JCS of the two-element array `[entry JSON, provenance JSON]`. The host `Etag` stays `Etag::of(&[u8])` over host record bytes. Reason: a `Matches(etag)` write must mean "nothing I read has changed", writer and expiry included. With an `Etag` over entry bytes alone, a refresh that changes only `expires_at` leaves the `Etag` unchanged, so a writer that read a lease as expired could overwrite it after its owner refreshed it. Equal stored state still gives equal `Etag`s on every backend (AMB-402 holds, over the wider content).
- `get_entry` and `list_entries` return `StoredEntry { record: EntryRecord, provenance: Provenance, etag: Etag }`; `Stored<R>` stays for the host.
- `put_entry(&self, catalog: &CatalogId, record: &EntryRecord, provenance: &Provenance, condition: Condition) -> Result<Etag, StoreError>`. There is no entry write without a provenance.
- `StoreError` gains `ModeConflict { stored: ModeKind }` and `CorruptProvenance { key: RecordKey, error: ProvenanceError }`.

#### 4.3 Guarantee 8 and the write paths

ADR-016 §3 gains **guarantee 8, mode exclusion**: a `put_entry` that would replace an existing record whose mode kind differs from the new provenance's fails `StoreError::ModeConflict { stored }`, evaluated and written in one atomic step (as guarantee 1). Of two concurrent puts of one key, one `Lease` and one `Curated`, the stored record afterwards has the mode of a put that returned `Ok`, and every put of the other mode that ran after it returned `ModeConflict`. Order of checks: `PreconditionFailed`, then `ModeConflict` (only a replace can conflict), then `VersionKindConflict` (only a create can conflict); the last two are disjoint. Deletion is not a mode write: `delete_entry` is unchanged.

Writer ownership and expiry are not backend guarantees. They are decided above the trait by read-then-`Matches`, which is atomic because the `Etag` covers provenance. One crate-private function owns this, `write_owned(store, catalog, record, provenance, now)`, used by `Registrar::register` and by `import`:

1. `get_entry`. Absent: `put_entry` with `Condition::Absent`.
2. Present: refuse `ModeConflict` when its mode kind differs, `OwnedByAnotherWriter` when its writer differs.
3. Present, same writer: for `Lease`, the new `expires_at` is the later of the stored one and `now + lease_ttl`, so a refresh never shortens a lease, whatever order instances run in. When the record bytes and the new provenance equal the stored ones, nothing is written and `Registered.written` is `false` (idempotent). Otherwise `put_entry` with `Condition::Matches(stored etag)`.
4. `PreconditionFailed` re-reads and repeats from 1. Each repeat follows a write another writer committed. After 8 attempts the result is `RegistrationError::Contended`.

`Registrar::register` steps 5 to 7 (§3.2) are this function: its read is step 5, the verifier (step 6) runs between the read and the conditional write, and a retry re-reads without verifying again, since the entry has not changed.

A lease refresh is therefore an idempotent conditional write with no counter: N instances refreshing one lease at once all return `Ok`, at most one write per distinct later expiry commits, and the stored expiry is the latest any of them asked for.

#### 4.4 Interaction with M21 (version-kind guard)

- The guard counts records, whatever their mode or expiry. An expired lease still holds its identifier's version kind until the record is deleted, so a registration of the other kind is `VersionKindConflict`. That is the correct reading of #multi-version-entries for the store's state (the record exists), and it is visible: assembly reports the expired record (§6), and its owner or the operator removes it.
- Mode is per record. One identifier may have a curated `1.0.0` and a leased `2.0.0`; both share one version kind.
- `ModeConflict` needs an existing record and `VersionKindConflict` a new one, so a put never meets both.

#### 4.5 Curated writes

- `import(store, catalog, document, writer: &WriterId)` writes every entry with `Provenance::Curated { writer }` through `write_owned`. It refuses an entry whose key holds a lease (`ImportError::Store(StoreError::ModeConflict)`) or a curated record of another writer (`ImportError::OwnedByAnotherWriter`). It runs no allow-list: `import` is the operator's path, as in ADR-016, and its authority is whoever holds the store credentials (§2).
- The CLI `produce` commands still write files only (ADR-016 AMB-406). Their curated route into a store is `produce` → file → `import`, which records `Curated`.
- `ServedCatalog::from_document` imports into a store private to the call with the writer id `from-document`, a constant private to the Lambda crate; its signature does not change.
- `Registrar::register` with `ModeRequest::Curated` is the same write after §3 authority, for an operator that grants curated registration through an authenticated channel.

#### 4.6 Backend schema and keys

`RecordKey` shapes (`"h"`, `"e:<hex>"`, `"g:<hex>"`) are unchanged. Expiry is not indexed: assembly reads every entry and decides in the crate (§6).

| Backend | Change |
|---|---|
| `in-memory` | the records map value for an entry holds `(record bytes, provenance bytes, ModeKind, Etag)`; the host value is unchanged. Guarantee 8 is checked under the existing write lock |
| `sqlite` | `catalog_records` gains `provenance BLOB` and `mode TEXT`, with `CHECK ((record_key = 'h' AND provenance IS NULL AND mode IS NULL) OR (record_key <> 'h' AND provenance IS NOT NULL AND mode IN ('lease','curated')))`. Inside the existing per-identifier transaction, after the guard row lock, the existing-record read also returns `mode`, and a replace with a different mode is `ModeConflict` |
| `postgres` | the same columns with `provenance BYTEA`, the same check and the same transaction step |
| `dynamodb` | entry items gain `provenance` (B) and `mode` (S). The `Any` replace `PutItem` condition becomes `attribute_exists(pk) AND #mode = :m` with `ReturnValuesOnConditionCheckFailure = ALL_OLD`: an old item with another `mode` is `ModeConflict`, no old item is the create path. Creates (`TransactWriteItems`) and `Matches` (`etag = :e`, which covers provenance) are otherwise unchanged. No TTL attribute is set (open question 1) |

`connect` for sqlite and postgres probes `SELECT provenance, mode FROM catalog_records LIMIT 0` after `CREATE TABLE IF NOT EXISTS`, and fails with `StoreError::Backend` naming the missing column on a table created by an earlier release. A DynamoDB entry item without `provenance` reads as `StoreError::CorruptProvenance`.

#### 4.7 Records stored by earlier releases (AMB-427)

Every record stored before this ADR is curated in fact, but has no writer. It is not read as curated: a record without a writer cannot satisfy ownership (§3.3), and reading a missing value as a default is a compatibility path, which this repository does not keep. A deployer re-runs `import` of its curated catalog file, with its writer id, into an empty catalog or a new table or database. Until then the store fails loudly: SQL `connect` fails, and a DynamoDB assembly fails with `CorruptProvenance`, so the Lambda fails cold start or keeps its last good snapshot (ADR-016 AMB-407). No migration code is written.

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

- The trust crate gains feature `registration` (off by default), which adds exactly `turul-ai-catalog-storage` (`default-features = false`; that set is the core crate, `async-trait` and `sha2`), `async-trait` and `tokio` (`rt`), and the type `DidWebPublisherVerifier { policy: TrustPolicy }` implementing `PublisherVerifier`. The trust crate's default dependency set is unchanged. There is no cycle: storage → core; trust → core, client and, under the feature, storage. Crypto and network stay out of every storage feature.
- The trust crate gains `verify_entry(entry: &CatalogEntry, base: Option<&str>, policy: &TrustPolicy, now: SystemTime) -> EntryReport<'_>`: resolve the distinct signers of the entry's manifests that pass `precheck`, then the same crate-private `entry_report` that `verify_resolved` uses. One per-entry procedure serves both; `verify` keeps its tree walk. ADR-015 records this in its revision log in the implementing slice.
- `DidWebPublisherVerifier::verify_publisher` runs `verify_entry(entry, None, &policy, now)` on a blocking thread (`tokio::task::spawn_blocking`, since resolution and integrity fetch block, ADR-015 §5) and accepts when at least one manifest's `endorsement()` is `Endorsement::Publisher`: an accepted `did-web-v1` signature in its time window (#signature-acceptance-and-time), Entry Release Coverage including `digest` and verified artifact bytes (#entry-release-coverage), and `publisher_authorized` (the root `did:web` domain rule and exact `did:web:` + publisher equality, #the-did-web-publisher-profile). Otherwise the refusal names each manifest's outcome. An assessor's signature never passes. `base` is `None`: a relative `url` cannot be fetched for integrity and so is refused.
- The verification is a write-time gate (AMB-425). It runs on every write by a grant that requires it, refreshes included, so a manifest that has expired since the last refresh stops the lease from being extended, and the entry leaves the catalog when its lease runs out. A curated entry verified at write is not re-verified. Consumers still verify for themselves (ADR-015): a stored entry passed this gate once and is never presented as endorsed by the store or the served document.

### 6. Expiry and assembly

- `assemble(store: &dyn CatalogStore, catalog: &CatalogId, now: SystemTime) -> Result<Assembled, AssembleError>`. A lease is live while `now < expires_at`, the half-open window the spec uses for `expiresAt`; at `expires_at` it is expired. No skew is applied: one clock (the assembler's) decides.
- `Assembled { bytes, report, omitted: Vec<Omission> }`, `Omission { key: EntryKey, provenance: Provenance, reason: OmissionReason }`, `OmissionReason::{LeaseExpired { expires_at: Timestamp }, Invalid(Report)}`, all `#[non_exhaustive]`. Every listed entry record is either in `bytes` or in `omitted`; assembly never drops one silently.
- ADR-016 §4 steps change as follows: after listing, expired leases are moved to `omitted`; the rest are sorted and validated as one document. When that document has error diagnostics, each remaining entry is run through the record gate's validation; entries that fail it are moved to `omitted` as `Invalid` with their own report, and the document is validated again. Errors that remain (cross-entry, such as #multi-version-entries' identifier-alone rule in a store this crate did not write) still fail with `AssembleError::Invalid`, because the spec does not say which of the conflicting entries to drop (AMB-426). `Invalid` covers records that passed an earlier release's gate and fail the current one.
- The result is a pure function of committed records and `now`: equal state and equal `now` give equal bytes on every backend. A lease expiring changes the assembled snapshot; the served document still carries no catalog signature (#catalog-snapshot-coverage; ADR-016 §4). Expiry depends on the clock, never on the caller, so this is not #dynamic-discovery.
- The clock is read only by callers: `SnapshotCache` (Lambda), a registrar's host, tests. `registration::storage_source_never_reads_the_clock` scans the storage crate's `src/` for `SystemTime::now` and `Instant::now`.
- **Lambda.** `SnapshotCache` passes its clock's `now` to `assemble`. `main` writes one stderr line per omission per assembly (key, writer, reason), so an omission is reported once per `max_age` per warm instance. ADR-010 is updated in the implementing slice.
- **`max_age` versus lease length (AMB-429).** A registration appears, and an expired lease disappears, up to `max_age` late per warm instance. A live lease is never missing from a served snapshot when the writer's refresh interval + `max_age` + clock skew between hosts is less than `lease_ttl`; the README states this rule. The library does not check it: `lease_ttl` lives in the `RegistrationPolicy` of a registrar process and `max_age` in the Lambda's environment, and no shared configuration exists between them; adding one would be a new control.
- **Expired records stay** until their owner unregisters them, the operator deletes them, or a write to the same key by the same writer replaces them. A reaper is a new control and is not built (open question 1).

## Owning symbols and entrypoints

| Key or artifact | Owner | Entrypoint |
|---|---|---|
| Writer identity | `turul_ai_catalog_storage::WriterId::new` | deployer → `Registrar::register`, `unregister`, `import` |
| Writer authority | `RegistrationPolicy::new`, `WriterGrant::new` | `Registrar::new` |
| Registration and removal | `Registrar::register`, `Registrar::unregister` | deployer's authenticated endpoint |
| Ownership, refresh and mode checks before a write | crate-private `write_owned` | `Registrar::register`, `import` |
| Entry mode and writer | `Provenance` (`to_bytes`, `parse`) | `put_entry`, every backend's reads |
| Entry version | `Etag::of_entry(&EntryRecord, &Provenance)`; host `Etag::of` | every backend, via the crate |
| Mode exclusion | `CatalogStore::put_entry`, guarantee 8 | four backends |
| Publisher signature seam | `PublisherVerifier` (storage); `DidWebPublisherVerifier` (trust, feature `registration`) | `Registrar::register` step 6 |
| Per-entry trust evaluation | `turul_ai_catalog_trust::verify_entry` (shares `entry_report` with `verify_resolved`) | `DidWebPublisherVerifier` |
| Served document and omissions | `assemble(store, catalog, now)` | `ServedCatalog::assemble`, `SnapshotCache` |
| `urn:air` grammar | `turul_ai_catalog::identity::UrnAir` (ADR-004) | `Registrar` step 3, `publisher_authorized` |
| Instant | `turul_ai_catalog::Timestamp` (ADR-005) | `Provenance`, `assemble` |

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
| M26 modes | lease and curated refuse each other | `parity::lease_and_curated_puts_refuse_each_other` (`Any` in both directions), `parity::racing_cross_mode_puts_leave_one_mode` (16 writers, four rounds), `registration::curated_registration_over_a_lease_is_refused`, `import::import_refuses_a_lease_record`, `import::import_records_curated_provenance` |
| M26 lease refresh | idempotent conditional write, many instances | `parity::concurrent_lease_refreshes_converge` (16 instances, one key: all `Ok`, record bytes unchanged, stored expiry = the latest requested), `registration::refresh_never_shortens_a_lease`, `registration::identical_refresh_writes_nothing` |
| M26 assembly report | expired or omitted entries reported | `assembly::expired_lease_is_omitted_and_reported`, `assembly::lease_is_live_until_expires_at` (live one nanosecond before, omitted at), `assembly::entry_failing_the_current_gate_is_omitted_and_reported`, `assembly::cross_entry_error_still_fails_assembly`, `assembly::every_listed_entry_is_served_or_omitted`, `parity::same_state_and_now_assemble_to_the_same_bytes`; lambda `snapshot_refresh::omissions_are_reported_on_each_assembly`, `snapshot_refresh::expired_lease_leaves_the_served_catalog_at_the_next_refresh` |
| M26 unregister | removal by the same writer, with authority | `parity::unregister_by_another_writer_is_refused`, `parity::unregister_frees_the_identifier`, `registration::unregister_needs_current_authority`, `registration::unregister_of_a_missing_entry_is_not_found` |

Decisions owned here:

| Id | Decision (short) | Test function |
|---|---|---|
| AMB-420 | writer id is opaque, operator-assigned, never from an address | `registration::writer_id_grammar` (accepts `[A-Za-z0-9._-]{1,128}`; refuses `10.0.0.1:443`, empty, 129 bytes) |
| AMB-421 | provenance beside record bytes, inside the entry `Etag`; guarantee 8 atomic | `provenance::provenance_bytes_are_canonical`, `provenance::provenance_round_trips`, `provenance::etag_covers_entry_and_provenance`, `parity::provenance_reads_back_exactly`, `parity::etag_is_the_digest_of_entry_and_provenance` (replaces ADR-016's `etag_is_the_digest_of_the_record_bytes` for entries), `parity::stored_entry_reads_back_byte_identical` (record bytes unchanged) |
| AMB-422 | the allow-list is configured delegation; only `identifier` is read | `registration::url_and_publisher_field_never_grant_authority` |
| AMB-423 | no prefix can admit `urn:air`; malformed `urn:air` refused | `registration_policy::urn_air_prefix_is_refused`, `registration::malformed_urn_air_is_refused` |
| AMB-424 | ownership per key until deleted, no takeover after expiry | `parity::another_writer_cannot_take_over_an_entry` |
| AMB-425 | signature gate on every write by a requiring grant; seam trait in storage, implementation in trust | `registration::refresh_with_an_expired_signature_does_not_extend_the_lease`; cli `dependency_direction::trust_registration_feature_adds_only_storage_async_trait_and_tokio`, `dependency_direction::storage_default_features_add_no_runtime_or_network_dependency` (kept: no storage feature depends on trust or client) |
| AMB-426 | per-entry gate failures omitted; cross-entry errors still fail | `assembly::entry_failing_the_current_gate_is_omitted_and_reported`, `assembly::cross_entry_error_still_fails_assembly` |
| AMB-427 | earlier records are not read as curated; re-import | `sqlite::table_from_an_earlier_release_fails_connect`, `postgres::table_from_an_earlier_release_fails_connect`, `dynamodb::item_without_provenance_is_corrupt` |
| AMB-428 | check order of §3.2; local refusals do no I/O, state refusals no network | `registration::checks_run_in_the_documented_order` (one request per step, each failing only that step; the stub verifier is called only at step 6, the store only from step 5) |
| AMB-429 | `max_age` against lease length is documented, not checked | none (no behaviour) |
| clock | injected `now`; storage never reads the clock | `registration::storage_source_never_reads_the_clock` |
| contention | bounded retries, then `Contended` | `registration::contention_beyond_the_attempt_bound_is_reported` (a store double that always fails `Matches`) |

New ids, block AMB-420 to AMB-439:

- AMB-420: the spec has no writer concept. An operator-assigned opaque id lets any authentication scheme sit in front, and keeping addresses out of the API makes "re-registration from a new address is an update" true by construction. The `Registrar` cannot authenticate; claiming otherwise would make store credentials look like authority, which the F1 input rejects.
- AMB-421: three placements were weighed. Inside the record bytes would change what `parse_entry` reads and what assembly serves. Beside the bytes and outside the `Etag` leaves a race: a lease refreshed with unchanged content keeps its `Etag`, so a stale reader's `Matches` overwrites a refreshed lease. Beside the bytes and inside the `Etag` keeps served bytes and Trust Manifest payloads unchanged and makes `Matches` cover writer and expiry. Mode exclusion must also hold for `Condition::Any` writers, which have no read to compare, so it is backend guarantee 8; it needs one more predicate in each backend's existing conditional step.
- AMB-422: #catalog-projection defines a publisher-authorized identifier as one "assigned by the artifact publisher or its authorized delegate" and forbids inference from URL or `publisher`. An operator's grant is an explicit, out-of-band configuration of who is a delegate, not an inference. It is weaker than a signature, so the signature requirement is available per grant.
- AMB-423: rule 3 requires a non-publisher, non-delegate operator to use non-`urn:air` identifiers. A prefix is the operator's own namespace, so it must not reach into `urn:air`. ADR-004 compares identifiers exactly, but some consumers may treat the URN scheme and NID case-insensitively (RFC 8141), so `URN:AIR:` is refused too.
- AMB-424: allowing another writer to take over an expired lease would let one identifier name a different writer's artifact without the operator acting, against "MUST NOT be reassigned". Keeping ownership until deletion leaves reassignment an explicit operator act (a store-credential delete).
- AMB-425: a storage feature depending on the trust crate would put JWS, `did:web` and network dependencies into storage and break ADR-016's pin. A trait in storage implemented by the trust crate under its own feature keeps storage free of crypto, has no cycle, and gives the trust crate one per-entry procedure for both report and gate. Verifying on refresh costs one DID document fetch and, for `url` entries, one artifact fetch per refresh per instance; no cache is built (a cache would be a new control).
- AMB-426: an entry gated by an earlier release can fail a rule added later. Failing the whole assembly for it would take every other entry offline; omitting and reporting it keeps the rest served and visible. A cross-entry conflict has no spec-defined loser, so assembly still refuses it.
- AMB-427: under this repository's no-compatibility rule, re-import replaces migration. Reading a missing provenance as `Curated` would be a permanent legacy path with no owner to check.
- AMB-428: as ADR-015 AMB-331: checks that need nothing come first, so a refused request never reaches the store or the network, and each refusal has one deterministic reason. The store read precedes verification so that a mode or ownership refusal never causes a fetch.
- AMB-429: §6.

## Consequences

- Every entry write now states a mode and a writer, and every entry `Etag` changes value (it now covers provenance). Callers of `put_entry`, `get_entry`, `list_entries`, `import` and `assemble` migrate in the same slices; ADR-016's parity checks that compare `Etag` with record bytes are rewritten for entries.
- Existing stores must be re-imported (§4.7). A deployed Lambda over an un-reimported DynamoDB table fails cold start, or keeps its last good snapshot, until then.
- SQL deployments gain two columns; DynamoDB writers need no new IAM action (`GetItem` and conditional `PutItem` are already granted).
- A runtime registration costs one read and one conditional write, plus verification when required. Assembly costs are unchanged apart from a second validation pass when the first finds errors.
- Expired leases keep their keys and version kinds until removed; a long-dead service's identifier cannot be registered in the other version kind until its record is deleted.
- Authority is only as strong as the boundary around store credentials (§2). The library cannot make a writer with store credentials obey a grant.
- The trust crate gains an optional dependency on storage under `registration`; its default build is unchanged.

## Open questions (for the maintainer; not built)

1. **Reaper.** Expired leases remain until removed (§6). Removing them automatically (a periodic sweep, or a DynamoDB TTL attribute) is a new control: it deletes records nobody asked to delete. Without it, nothing breaks today: expired leases are omitted from every assembly and reported. The sentence to approve: "Add a reaper that deletes lease records expired for longer than a configured grace period."
2. **Registration endpoint.** The library has no transport. A deployer builds the authenticated channel in front of the `Registrar` (§2). Shipping one (an HTTP handler in the Lambda crate, say) is a new feature and needs its own approval.
3. **`max_age` check.** Whether the Lambda should know `lease_ttl` and refuse a `max_age` that is too long (§6, AMB-429). Nothing breaks without it when the documented rule is followed.

## Slices

Each slice is ADR updates, then tests from this ADR, then code, then gates, then a patch bump with `CHANGELOG.md`.

| Slice | Content | Updates |
|---|---|---|
| R1 | `WriterId`, `Provenance`, `ModeKind`, `Etag::of_entry`, `StoredEntry`; `put_entry` takes a provenance; guarantee 8 in all four backends with the schema changes of §4.6 and the earlier-table probe; `write_owned`; `import` takes a writer and records `Curated`; `ServedCatalog::from_document` passes its constant; parity checks for provenance, mode exclusion and racing modes | ADR-016 (trait, guarantee 8, §6 schemas, `Etag`, `import`), ADR-010 (`from_document`), revision logs |
| R2 | `assemble(.., now)` with lease expiry, `Omission`, the `Invalid` second pass; `SnapshotCache` passes `now` and logs omissions; assembly, parity and lambda tests | ADR-016 §4, ADR-010, README (`max_age` rule) |
| R3 | `RegistrationPolicy`, `WriterGrant`, `PolicyError`, `PublisherVerifier`, `Registrar::register` and `unregister` with the §3.2 order, lease refresh and `Contended`; `registration_policy.rs`, `registration.rs`, registration parity checks on every backend | this ADR's revision log if anything changes |
| R4 | trust `verify_entry`; feature `registration` with `DidWebPublisherVerifier`; `publisher_verifier.rs`; `dependency_direction` pin | ADR-015 (`verify_entry`), ADR-011 (`dependency_direction`) |

R1 and R2 change behaviour for every deployer (re-import, new `assemble` signature); R3 and R4 add API only.

## What this ADR does not do

- No code, tests or `Cargo.toml` changes: they follow in R1 to R4.
- No reaper, no TTL attribute, no registration endpoint or transport, no `max_age` check, no verification cache, no authentication of writers.
- No change to who may write a host: `put_host` and `import` stay operator paths.
- No serving of omissions or provenance to clients: the served document is unchanged in shape.
- No identifier assignment, rewriting or migration mechanism (#catalog-projection: "requires an explicit migration mechanism, which this specification does not define").
- Names no consumer of the library and decides nothing in any other repository.

## Revision log

| Date | Change |
|---|---|
| 2026-11-17 | Created under M26: `Registrar` over `CatalogStore` with `RegistrationPolicy` and `WriterGrant` (publisher and prefix allow-lists, refusal when neither, optional signature); opaque `WriterId`; `Provenance` (lease or curated, with writer) stored beside record bytes and covered by the entry `Etag`; backend guarantee 8 (mode exclusion) and schema changes for all four backends; idempotent lease refresh by read-then-`Matches`; `assemble` with injected `now` and an omission report; `PublisherVerifier` seam implemented by the trust crate under feature `registration` through a new `verify_entry`; re-import for earlier stores. AMB-420 to AMB-429 added. Reaper, endpoint and `max_age` check left to the maintainer. |
