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

A write the DynamoDB backend loses to other writers' transactions is retried at most 8 times, with
jittered exponential backoff, then fails with `StoreError::Contended`.

Every backend guarantees atomic conditional writes, read-your-writes, byte-identical reads, and
that no identifier holds both unversioned and versioned entries, and that no entry is replaced by a
put of the other mode. Each entry is stored with a [`Provenance`]: curated or a lease, and its
writer. The entry's [`Etag`] covers it.

## Quick start

```sh
cargo add turul-ai-catalog-storage
```

Store an entry, then assemble the catalog document it belongs to:

```rust
use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
use turul_ai_catalog::model::{ArtifactSource, CatalogEntry};
use turul_ai_catalog_storage::{
    CatalogId, CatalogStore, Condition, EntryRecord, InMemoryCatalogStore, Provenance, WriterId,
    assemble,
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
let provenance = Provenance::Curated { writer: WriterId::new("operator")? };
store.put_entry(&catalog, &record, &provenance, Condition::Absent).await?;

let served = assemble(&store, &catalog, std::time::SystemTime::now()).await?;
assert!(!served.bytes.is_empty() && served.omitted.is_empty());
# Ok::<(), Box<dyn std::error::Error>>(())
# })?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Leases and what assembly leaves out

[`assemble`] takes `now`; this crate never reads a clock. A lease is served while `now` is before
its `expires_at`. An expired lease, and an entry that fails validation as it stands now, are left
out of the document and listed in `Assembled::omitted` with the reason, so nothing is dropped
silently. `Assembled::next_expiry` is the earliest expiry among the leases served; a caller that
caches the document re-assembles at that instant.

A live lease is never missing from a served snapshot when its owner's refresh commits before expiry:
refresh interval plus clock skew between the registrar's host and the serving host must be less than
`lease_ttl - lease_ttl / 4`. Keep `lease_ttl` at least 4 times the refresh interval. The library does
not check this: the interval and the lease length are configured in different processes.

## Curated writer and upgrading a store

[`import`] writes every entry as curated by the `WriterId` it is given, and refuses an entry whose
key holds a lease or a curated record of another writer. **The curated writer id is permanent
configuration**: changing it fails at the first entry the catalog already holds. If it is lost,
delete the catalog id's items (entry records and `"g:"` guards) with store credentials and import
again with the new id.

Records stored by an earlier release have no provenance and are not read as curated. SQL `connect`
fails on such a table; a DynamoDB assembly fails with `StoreError::CorruptProvenance`. To upgrade a
store that is being served:

1. `import` the curated catalog file with this release and its permanent writer id, under a **new**
   `CatalogId` in the same table (SQL: a new database, SQLite: a new file).
2. Deploy the function with `TURUL_AI_CATALOG_ID` set to the new id.
3. Delete the old id's items with store credentials.

Until step 2 the previous function keeps serving the old id. **Never point the previous release at
a new-format catalog id**: it writes entries without provenance, which this release then reads as
corrupt, and it bypasses the mode exclusion.

## Runtime registration

A [`Registrar`] lets writers the deployer has authenticated register entries as leases or curated
records, under a [`RegistrationPolicy`]: each [`WriterId`] has a [`WriterGrant`] of `urn:air`
publishers and non-`urn:air` identifier prefixes, and optionally must prove its publisher signature
through a [`PublisherVerifier`]. A writer without a grant, or a grant with neither publishers nor
prefixes, is refused. The registrar never reads `url` or `publisher` for authority, only
`identifier`. It holds its store and verifier as `Arc<dyn CatalogStore>` and
`Arc<dyn PublisherVerifier>`, so it can live in a service's static state.

- **Modes are granted.** A grant permits leases only; `WriterGrant::with_curated` also permits
  curated entries, which never expire and are not verified again after their write. A curated
  request under a lease-only grant is refused with `CuratedNotGranted` before the store is read.
- **Co-granted writers share identifiers, not records.** Ownership is per entry key. Writers granted
  the same publisher or prefix may each add versions of an identifier the other registered, and the
  first record fixes its version kind for both; neither can overwrite or remove the other's record.
  Grant disjoint publishers and prefixes for one writer per identifier.

- **Role separation first.** Authority holds only if no registering service's role can write the
  table. Run the registrar under its own role behind the deployer's authenticated channel; only the
  registrar's role and the operator's `import` role hold the writer actions; the serving role reads
  only.
- **When a service registers.** Once it is serving, never during init: a lease claims the artifact
  is reachable, and one left by an init that then fails stays served until it expires.
- **Refresh.** `register` of the same key by the same writer in `Lease` mode refreshes. It writes
  only once less than three quarters of `lease_ttl` remain, so many instances refreshing one lease
  commit one write per quarter. Keep `lease_ttl` at least 4 times the refresh interval.
- **Refresh failures.** A refresh that fails, a `PublisherSignature` refusal because a DID document
  or artifact could not be fetched included, is reported by the service and retried at its next
  interval. It is not fatal while the lease has time left.
- **Removing a stale lease.** Its owner calls `Registrar::unregister`. Otherwise the operator runs `turul-ai-catalog store remove` (backed by `list_registrations` and `remove_registration`) or, with
  store credentials, deletes it by its `EntryKey`, conditioned on the version it read so that a
  refresh in between makes the delete fail rather than remove a live lease:

```no_run
use std::time::SystemTime;
use turul_ai_catalog::{identity::{EntryIdentifier, EntryKey}, timestamp::Timestamp};
use turul_ai_catalog_storage::{CatalogId, CatalogStore, Provenance, StoreError};

async fn remove_expired_lease(store: &dyn CatalogStore, catalog: &CatalogId, key: &EntryKey)
    -> Result<bool, StoreError> {
    let Some(stored) = store.get_entry(catalog, key).await? else { return Ok(false) };
    let Provenance::Lease { expires_at, .. } = &stored.provenance else { return Ok(false) };
    if *expires_at > Timestamp::from_system_time(SystemTime::now()) { return Ok(false) }
    store.delete_entry(catalog, key, Some(&stored.etag)).await?; // PreconditionFailed if refreshed
    Ok(true)
}
```
