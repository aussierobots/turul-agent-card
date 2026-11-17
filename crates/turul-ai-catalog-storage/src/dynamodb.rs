use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::time::Duration;

use async_trait::async_trait;
use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::error::{DisplayErrorContext, ProvideErrorMetadata, SdkError};
use aws_sdk_dynamodb::operation::put_item::PutItemError;
use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError;
use aws_sdk_dynamodb::primitives::Blob;
use aws_sdk_dynamodb::types::{
    AttributeValue, ConditionCheck, Delete, Put, ReturnValuesOnConditionCheckFailure,
    TransactWriteItem, Update,
};
use turul_ai_catalog::identity::EntryKey;

use crate::ids::{CatalogId, Etag, RecordKey};
use crate::owned::ATTEMPTS;
use crate::provenance::{ModeKind, Provenance, ProvenanceError};
use crate::records::{EntryRecord, HostRecord};
use crate::store::{
    CatalogStore, Condition, ItemCounts, PurgeReport, ScannedEntry, StoreError, Stored,
    StoredEntry, corrupt_provenance, entry_write, key_error, kind_conflict, scanned_entry,
    stored_entry,
};

const ITEM_LIMIT: &str = "record exceeds the DynamoDB item size limit of 400 KB";

/// Guard item attributes: the identifier's version kind (BOOL) and its entry count (N).
const VERSIONED: &str = "versioned";
const ENTRIES: &str = "entries";

const CONDITION_FAILED: &str = "ConditionalCheckFailed";
const TRANSACTION_CONFLICT: &str = "TransactionConflict";

/// Entry item attributes: the provenance bytes (B) and the mode name (S), which a write condition
/// can test.
const PROVENANCE: &str = "provenance";
const MODE: &str = "mode";

/// The longest wait before the first retry; each later retry may wait twice as long.
const BACKOFF_BASE: Duration = Duration::from_millis(20);

/// Waits before retry `retry` (from 1) of a write that conflicted with another writer's
/// transaction: a uniformly random delay below `BACKOFF_BASE × 2^(retry − 1)`, so writers that
/// conflicted once spread their next attempts apart.
async fn back_off(retry: usize) {
    let cap = BACKOFF_BASE.saturating_mul(1 << (retry - 1).min(16));
    let draw = RandomState::new().build_hasher().finish();
    let nanos = u64::try_from(cap.as_nanos()).unwrap_or(u64::MAX).max(1);
    tokio::time::sleep(Duration::from_nanos(draw % nanos)).await;
}

/// An entry item ready to write: its key, record, provenance bytes and mode, and its version.
struct EntryItem<'a> {
    key: &'a RecordKey,
    record: &'a EntryRecord,
    provenance: &'a [u8],
    mode: ModeKind,
    etag: &'a Etag,
}

/// Records in one DynamoDB table with partition key `pk` (S) and sort key `sk` (S). Beside each
/// identifier's entry items sits a guard item (`sk` = `"g:…"`) holding its version kind and entry
/// count; a new entry and its guard change in one transaction. The table belongs to the deployer:
/// the store neither creates nor verifies it.
pub struct DynamoDbCatalogStore {
    client: Client,
    table: String,
}

fn backend<E, R>(e: SdkError<E, R>) -> StoreError
where
    E: ProvideErrorMetadata + std::error::Error + 'static,
    R: std::fmt::Debug,
{
    if e.message().is_some_and(|m| m.contains("Item size")) {
        return StoreError::Backend(ITEM_LIMIT.to_string());
    }
    StoreError::Backend(DisplayErrorContext(&e).to_string())
}

fn condition_failed<E, R>(e: &SdkError<E, R>) -> bool
where
    E: ProvideErrorMetadata,
{
    e.code() == Some("ConditionalCheckFailedException")
}

fn build_error(e: aws_sdk_dynamodb::error::BuildError) -> StoreError {
    StoreError::Backend(e.to_string())
}

/// The code of each item's cancellation reason, in request order, and the reasons' messages.
struct Cancelled {
    codes: Vec<String>,
    messages: Vec<String>,
}

impl Cancelled {
    fn failed(&self, item: usize) -> bool {
        self.codes.get(item).is_some_and(|c| c == CONDITION_FAILED)
    }

    fn conflicted(&self) -> bool {
        self.codes.iter().any(|c| c == TRANSACTION_CONFLICT)
    }

    fn into_error(self) -> StoreError {
        if self.messages.iter().any(|m| m.contains("Item size")) {
            return StoreError::Backend(ITEM_LIMIT.to_string());
        }
        StoreError::Backend(format!("transaction cancelled: {}", self.codes.join(", ")))
    }
}

fn s(value: &str) -> AttributeValue {
    AttributeValue::S(value.to_string())
}

fn optional_provenance(
    item: &std::collections::HashMap<String, AttributeValue>,
) -> Option<Vec<u8>> {
    match item.get(PROVENANCE) {
        Some(AttributeValue::B(blob)) => Some(blob.as_ref().to_vec()),
        _ => None,
    }
}

fn record_bytes(
    item: &std::collections::HashMap<String, AttributeValue>,
) -> Result<Vec<u8>, StoreError> {
    match item.get("record") {
        Some(AttributeValue::B(blob)) => Ok(blob.as_ref().to_vec()),
        _ => Err(StoreError::Backend(
            "item has no binary `record` attribute".to_string(),
        )),
    }
}

/// Why a conditional replace failed, from the item the failed condition returned: no item or a
/// version other than `expected` is `PreconditionFailed`, then another mode is `ModeConflict`.
fn replace_refusal(
    key: &RecordKey,
    old: Option<&std::collections::HashMap<String, AttributeValue>>,
    expected: Option<&Etag>,
) -> StoreError {
    let Some(old) = old.filter(|o| !o.is_empty()) else {
        return StoreError::PreconditionFailed;
    };
    if let Some(e) = expected
        && !matches!(old.get("etag"), Some(AttributeValue::S(stored)) if stored == e.as_str())
    {
        return StoreError::PreconditionFailed;
    }
    match old.get(MODE) {
        Some(AttributeValue::S(name)) => match ModeKind::from_name(name) {
            Some(stored) => StoreError::ModeConflict { stored },
            None => corrupt_provenance(key, ProvenanceError::Mode),
        },
        _ => corrupt_provenance(key, ProvenanceError::Missing),
    }
}

impl DynamoDbCatalogStore {
    /// Creates a store over the existing table `table_name`, which the store neither creates nor
    /// verifies.
    pub fn new(client: Client, table_name: impl Into<String>) -> Self {
        Self {
            client,
            table: table_name.into(),
        }
    }

    async fn put(
        &self,
        catalog: &CatalogId,
        key: &RecordKey,
        bytes: &[u8],
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        let etag = Etag::of(bytes);
        let mut request = self
            .client
            .put_item()
            .table_name(&self.table)
            .item("pk", s(catalog.as_str()))
            .item("sk", s(key.as_str()))
            .item("record", AttributeValue::B(Blob::new(bytes)))
            .item("etag", s(etag.as_str()));
        match &condition {
            Condition::Any => {}
            Condition::Absent => {
                request = request.condition_expression("attribute_not_exists(pk)");
            }
            Condition::Matches(expected) | Condition::Overwrite(expected) => {
                request = request
                    .condition_expression("etag = :e")
                    .expression_attribute_values(":e", s(expected.as_str()));
            }
        }
        match request.send().await {
            Ok(_) => Ok(etag),
            Err(e) if condition_failed(&e) => Err(StoreError::PreconditionFailed),
            Err(e) => Err(backend(e)),
        }
    }

    /// Replaces an existing entry item of the same mode, with `expected` its required version.
    /// Fails `PreconditionFailed` when the item is absent or its version differs, and
    /// `ModeConflict` when it is in the other mode. An item inside another writer's transaction
    /// is retried with backoff, up to [`ATTEMPTS`] attempts, then `Contended`.
    async fn replace(
        &self,
        catalog: &CatalogId,
        item: &EntryItem<'_>,
        expected: Option<&Etag>,
    ) -> Result<Etag, StoreError> {
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                back_off(attempt).await;
            }
            let request = self
                .client
                .put_item()
                .table_name(&self.table)
                .item("pk", s(catalog.as_str()))
                .item("sk", s(item.key.as_str()))
                .item("record", AttributeValue::B(Blob::new(item.record.bytes())))
                .item(PROVENANCE, AttributeValue::B(Blob::new(item.provenance)))
                .item(MODE, s(item.mode.as_str()))
                .item("etag", s(item.etag.as_str()))
                .expression_attribute_names("#m", MODE)
                .expression_attribute_values(":m", s(item.mode.as_str()))
                .return_values_on_condition_check_failure(
                    ReturnValuesOnConditionCheckFailure::AllOld,
                );
            let request = match expected {
                None => request.condition_expression("attribute_exists(pk) AND #m = :m"),
                Some(e) => request
                    .condition_expression("etag = :e AND #m = :m")
                    .expression_attribute_values(":e", s(e.as_str())),
            };
            match request.send().await {
                Ok(_) => return Ok(item.etag.clone()),
                Err(e) => match e.as_service_error() {
                    Some(PutItemError::ConditionalCheckFailedException(failed)) => {
                        return Err(replace_refusal(item.key, failed.item(), expected));
                    }
                    // The item is inside another writer's transaction.
                    _ if e.code() == Some("TransactionConflictException") => {}
                    _ => return Err(backend(e)),
                },
            }
        }
        Err(StoreError::Contended)
    }

    /// Replaces the entry item under `item.key` when its stored version is `expected`, whatever its
    /// mode and whether its provenance reads. The old item is never read: the put is conditioned on
    /// the `etag` attribute alone, and `PutItem` replaces every attribute; a replace keeps the key,
    /// so the guard count is unchanged. An item inside another writer's transaction is retried with
    /// backoff, up to [`ATTEMPTS`] attempts, then `Contended`.
    async fn overwrite(
        &self,
        catalog: &CatalogId,
        item: &EntryItem<'_>,
        expected: &Etag,
    ) -> Result<Etag, StoreError> {
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                back_off(attempt).await;
            }
            let put = self
                .client
                .put_item()
                .table_name(&self.table)
                .item("pk", s(catalog.as_str()))
                .item("sk", s(item.key.as_str()))
                .item("record", AttributeValue::B(Blob::new(item.record.bytes())))
                .item(PROVENANCE, AttributeValue::B(Blob::new(item.provenance)))
                .item(MODE, s(item.mode.as_str()))
                .item("etag", s(item.etag.as_str()))
                .condition_expression("etag = :e")
                .expression_attribute_values(":e", s(expected.as_str()));
            match put.send().await {
                Ok(_) => return Ok(item.etag.clone()),
                Err(e) if condition_failed(&e) => return Err(StoreError::PreconditionFailed),
                // The item is inside another writer's transaction.
                Err(e) if e.code() == Some("TransactionConflictException") => {}
                Err(e) => return Err(backend(e)),
            }
        }
        Err(StoreError::Contended)
    }

    /// The transaction that creates an entry item and counts it in its identifier's guard, refused
    /// when the guard holds the other version kind.
    fn create_items(
        &self,
        catalog: &CatalogId,
        item: &EntryItem<'_>,
        guard: &RecordKey,
    ) -> Result<Vec<TransactWriteItem>, StoreError> {
        let create = Put::builder()
            .table_name(&self.table)
            .item("pk", s(catalog.as_str()))
            .item("sk", s(item.key.as_str()))
            .item("record", AttributeValue::B(Blob::new(item.record.bytes())))
            .item(PROVENANCE, AttributeValue::B(Blob::new(item.provenance)))
            .item(MODE, s(item.mode.as_str()))
            .item("etag", s(item.etag.as_str()))
            .condition_expression("attribute_not_exists(pk)")
            .build()
            .map_err(build_error)?;
        let count = Update::builder()
            .table_name(&self.table)
            .key("pk", s(catalog.as_str()))
            .key("sk", s(guard.as_str()))
            .update_expression("SET #v = :v ADD #n :one")
            .condition_expression("attribute_not_exists(pk) OR #v = :v")
            .expression_attribute_names("#v", VERSIONED)
            .expression_attribute_names("#n", ENTRIES)
            .expression_attribute_values(
                ":v",
                AttributeValue::Bool(item.record.entry().version.is_some()),
            )
            .expression_attribute_values(":one", AttributeValue::N("1".to_string()))
            .build()
            .map_err(build_error)?;
        Ok(vec![
            TransactWriteItem::builder().put(create).build(),
            TransactWriteItem::builder().update(count).build(),
        ])
    }

    /// One page of the item keys (`sk`) of `catalog` and the key to resume from, `None` after the
    /// last page. Fetches no other attribute.
    async fn key_page(
        &self,
        catalog: &CatalogId,
        start: Option<HashMap<String, AttributeValue>>,
    ) -> Result<(Vec<String>, Option<HashMap<String, AttributeValue>>), StoreError> {
        let out = self
            .client
            .query()
            .table_name(&self.table)
            .key_condition_expression("pk = :c")
            .expression_attribute_values(":c", s(catalog.as_str()))
            .projection_expression("pk, sk")
            .consistent_read(true)
            .set_exclusive_start_key(start)
            .send()
            .await
            .map_err(backend)?;
        let keys = out
            .items()
            .iter()
            .map(|item| match item.get("sk") {
                Some(AttributeValue::S(name)) => Ok(name.clone()),
                _ => Err(StoreError::Backend(
                    "item has no string `sk` attribute".to_string(),
                )),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((keys, out.last_evaluated_key().map(ToOwned::to_owned)))
    }

    /// Deletes the item under `sk` if it exists; `false` when it did not (another writer removed
    /// it). An item inside another writer's transaction, or a throttled request, is retried with
    /// backoff, up to [`ATTEMPTS`] attempts, then `Contended`.
    async fn delete_existing(&self, catalog: &CatalogId, sk: &str) -> Result<bool, StoreError> {
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                back_off(attempt).await;
            }
            match self
                .client
                .delete_item()
                .table_name(&self.table)
                .key("pk", s(catalog.as_str()))
                .key("sk", s(sk))
                .condition_expression("attribute_exists(pk)")
                .send()
                .await
            {
                Ok(_) => return Ok(true),
                Err(e) if condition_failed(&e) => return Ok(false),
                Err(e)
                    if matches!(
                        e.code(),
                        Some(
                            "TransactionConflictException"
                                | "ThrottlingException"
                                | "ProvisionedThroughputExceededException"
                                | "RequestLimitExceeded"
                        )
                    ) => {}
                Err(e) => return Err(backend(e)),
            }
        }
        Err(StoreError::Contended)
    }

    /// Runs one transaction; `Some` when DynamoDB cancelled it.
    async fn transact(
        &self,
        items: Vec<TransactWriteItem>,
    ) -> Result<Option<Cancelled>, StoreError> {
        match self
            .client
            .transact_write_items()
            .set_transact_items(Some(items))
            .send()
            .await
        {
            Ok(_) => Ok(None),
            Err(e) => match e.as_service_error() {
                Some(TransactWriteItemsError::TransactionCanceledException(c)) => {
                    let reasons = c.cancellation_reasons();
                    Ok(Some(Cancelled {
                        codes: reasons
                            .iter()
                            .map(|r| r.code().unwrap_or("None").to_string())
                            .collect(),
                        messages: reasons
                            .iter()
                            .filter_map(|r| r.message().map(str::to_string))
                            .collect(),
                    }))
                }
                _ => Err(backend(e)),
            },
        }
    }

    /// The entry count of a guard, read consistently; `None` when the guard is absent.
    async fn guard_entries(
        &self,
        catalog: &CatalogId,
        guard: &RecordKey,
    ) -> Result<Option<String>, StoreError> {
        let out = self
            .client
            .get_item()
            .table_name(&self.table)
            .key("pk", s(catalog.as_str()))
            .key("sk", s(guard.as_str()))
            .consistent_read(true)
            .send()
            .await
            .map_err(backend)?;
        out.item()
            .map(|item| match item.get(ENTRIES) {
                Some(AttributeValue::N(n)) => Ok(n.clone()),
                _ => Err(StoreError::Backend(
                    "guard item has no numeric `entries` attribute".to_string(),
                )),
            })
            .transpose()
    }

    async fn get(
        &self,
        catalog: &CatalogId,
        key: &RecordKey,
    ) -> Result<Option<(Vec<u8>, Etag)>, StoreError> {
        let out = self
            .client
            .get_item()
            .table_name(&self.table)
            .key("pk", s(catalog.as_str()))
            .key("sk", s(key.as_str()))
            .consistent_read(true)
            .send()
            .await
            .map_err(backend)?;
        out.item()
            .map(|item| {
                let bytes = record_bytes(item)?;
                let etag = Etag::of(&bytes);
                Ok((bytes, etag))
            })
            .transpose()
    }
}

#[async_trait]
impl CatalogStore for DynamoDbCatalogStore {
    fn backend_name(&self) -> &'static str {
        "dynamodb"
    }

    async fn put_entry(
        &self,
        catalog: &CatalogId,
        record: &EntryRecord,
        provenance: &Provenance,
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        let key = record.record_key().map_err(key_error)?;
        let (provenance_bytes, etag) = entry_write(&key, record, provenance)?;
        let item = EntryItem {
            key: &key,
            record,
            provenance: &provenance_bytes,
            mode: provenance.kind(),
            etag: &etag,
        };
        if let Condition::Matches(expected) = &condition {
            return self.replace(catalog, &item, Some(expected)).await;
        }
        if let Condition::Overwrite(expected) = &condition {
            return self.overwrite(catalog, &item, expected).await;
        }
        let guard = RecordKey::guard(&record.entry().identifier).map_err(key_error)?;
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                back_off(attempt).await;
            }
            let items = self.create_items(catalog, &item, &guard)?;
            let Some(cancelled) = self.transact(items).await? else {
                return Ok(etag);
            };
            if cancelled.failed(0) {
                if condition == Condition::Absent {
                    return Err(StoreError::PreconditionFailed);
                }
                // `Any` on an existing entry is a replace, which leaves the kinds unchanged; an
                // entry deleted meanwhile is created on the next attempt.
                match self.replace(catalog, &item, None).await {
                    Err(StoreError::PreconditionFailed) => continue,
                    done => return done,
                }
            }
            if cancelled.failed(1) {
                return Err(kind_conflict(record));
            }
            if !cancelled.conflicted() {
                return Err(cancelled.into_error());
            }
        }
        Err(StoreError::Contended)
    }

    async fn get_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
    ) -> Result<Option<StoredEntry>, StoreError> {
        let record_key = RecordKey::entry(key).map_err(key_error)?;
        let out = self
            .client
            .get_item()
            .table_name(&self.table)
            .key("pk", s(catalog.as_str()))
            .key("sk", s(record_key.as_str()))
            .consistent_read(true)
            .send()
            .await
            .map_err(backend)?;
        out.item()
            .map(|item| {
                stored_entry(
                    &record_key,
                    record_bytes(item)?,
                    optional_provenance(item).as_deref(),
                )
            })
            .transpose()
    }

    async fn delete_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
        expected: Option<&Etag>,
    ) -> Result<(), StoreError> {
        let record_key = RecordKey::entry(key).map_err(key_error)?;
        let guard = RecordKey::guard(key.identifier()).map_err(key_error)?;
        for attempt in 0..ATTEMPTS {
            if attempt > 0 {
                back_off(attempt).await;
            }
            let remove = Delete::builder()
                .table_name(&self.table)
                .key("pk", s(catalog.as_str()))
                .key("sk", s(record_key.as_str()));
            let remove = match expected {
                None => remove.condition_expression("attribute_exists(pk)"),
                Some(etag) => remove
                    .condition_expression("etag = :e")
                    .expression_attribute_values(":e", s(etag.as_str())),
            }
            .build()
            .map_err(build_error)?;
            // The guard changes only if it still holds the count read here; otherwise retry.
            let guard_item = match self.guard_entries(catalog, &guard).await? {
                // A record stored without a guard was never counted.
                None => TransactWriteItem::builder()
                    .condition_check(
                        ConditionCheck::builder()
                            .table_name(&self.table)
                            .key("pk", s(catalog.as_str()))
                            .key("sk", s(guard.as_str()))
                            .condition_expression("attribute_not_exists(pk)")
                            .build()
                            .map_err(build_error)?,
                    )
                    .build(),
                Some(n) if n == "1" => TransactWriteItem::builder()
                    .delete(
                        Delete::builder()
                            .table_name(&self.table)
                            .key("pk", s(catalog.as_str()))
                            .key("sk", s(guard.as_str()))
                            .condition_expression("#n = :n")
                            .expression_attribute_names("#n", ENTRIES)
                            .expression_attribute_values(":n", AttributeValue::N(n))
                            .build()
                            .map_err(build_error)?,
                    )
                    .build(),
                Some(n) => TransactWriteItem::builder()
                    .update(
                        Update::builder()
                            .table_name(&self.table)
                            .key("pk", s(catalog.as_str()))
                            .key("sk", s(guard.as_str()))
                            .update_expression("ADD #n :minus")
                            .condition_expression("#n = :n")
                            .expression_attribute_names("#n", ENTRIES)
                            .expression_attribute_values(":n", AttributeValue::N(n))
                            .expression_attribute_values(
                                ":minus",
                                AttributeValue::N("-1".to_string()),
                            )
                            .build()
                            .map_err(build_error)?,
                    )
                    .build(),
            };
            let items = vec![
                TransactWriteItem::builder().delete(remove).build(),
                guard_item,
            ];
            let Some(cancelled) = self.transact(items).await? else {
                return Ok(());
            };
            if cancelled.failed(0) {
                return Err(match expected {
                    None => StoreError::NotFound,
                    Some(_) => StoreError::PreconditionFailed,
                });
            }
            if !cancelled.failed(1) && !cancelled.conflicted() {
                return Err(cancelled.into_error());
            }
        }
        Err(StoreError::Contended)
    }

    async fn list_entries(&self, catalog: &CatalogId) -> Result<Vec<StoredEntry>, StoreError> {
        let mut listed = Vec::new();
        let mut start = None;
        loop {
            let out = self
                .client
                .query()
                .table_name(&self.table)
                .key_condition_expression("pk = :c AND begins_with(sk, :p)")
                .expression_attribute_values(":c", s(catalog.as_str()))
                .expression_attribute_values(":p", s(RecordKey::ENTRY_PREFIX))
                .consistent_read(true)
                .set_exclusive_start_key(start)
                .send()
                .await
                .map_err(backend)?;
            for item in out.items() {
                let bytes = record_bytes(item)?;
                let name = match item.get("sk") {
                    Some(AttributeValue::S(name)) => name.clone(),
                    _ => String::new(),
                };
                listed.push(stored_entry(
                    &RecordKey::from_stored_name(name),
                    bytes,
                    optional_provenance(item).as_deref(),
                )?);
            }
            start = out.last_evaluated_key().map(ToOwned::to_owned);
            if start.is_none() {
                return Ok(listed);
            }
        }
    }

    async fn put_host(
        &self,
        catalog: &CatalogId,
        record: &HostRecord,
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        self.put(catalog, &RecordKey::HOST, record.bytes(), condition)
            .await
    }

    async fn get_host(
        &self,
        catalog: &CatalogId,
    ) -> Result<Option<Stored<HostRecord>>, StoreError> {
        let Some((bytes, etag)) = self.get(catalog, &RecordKey::HOST).await? else {
            return Ok(None);
        };
        let record = HostRecord::from_stored(bytes)
            .map_err(|e| crate::store::corrupt(&RecordKey::HOST, e))?;
        Ok(Some(Stored { record, etag }))
    }

    async fn scan_entries(&self, catalog: &CatalogId) -> Result<Vec<ScannedEntry>, StoreError> {
        let mut scanned = Vec::new();
        let mut start = None;
        loop {
            let out = self
                .client
                .query()
                .table_name(&self.table)
                .key_condition_expression("pk = :c AND begins_with(sk, :p)")
                .expression_attribute_values(":c", s(catalog.as_str()))
                .expression_attribute_values(":p", s(RecordKey::ENTRY_PREFIX))
                .consistent_read(true)
                .set_exclusive_start_key(start)
                .send()
                .await
                .map_err(backend)?;
            for item in out.items() {
                let name = match item.get("sk") {
                    Some(AttributeValue::S(name)) => name.clone(),
                    _ => String::new(),
                };
                let etag = match item.get("etag") {
                    Some(AttributeValue::S(etag)) => Etag::from_stored(etag.clone()),
                    _ => {
                        return Err(StoreError::Backend(
                            "item has no string `etag` attribute".to_string(),
                        ));
                    }
                };
                scanned.push(scanned_entry(
                    &RecordKey::from_stored_name(name),
                    record_bytes(item)?,
                    etag,
                    optional_provenance(item).as_deref(),
                )?);
            }
            start = out.last_evaluated_key().map(ToOwned::to_owned);
            if start.is_none() {
                return Ok(scanned);
            }
        }
    }

    async fn count_catalog(&self, catalog: &CatalogId) -> Result<ItemCounts, StoreError> {
        let mut counts = ItemCounts::default();
        let mut start = None;
        loop {
            let (keys, next) = self.key_page(catalog, start).await?;
            for key in &keys {
                counts.add_key(key);
            }
            if next.is_none() {
                return Ok(counts);
            }
            start = next;
        }
    }

    async fn purge_catalog(&self, catalog: &CatalogId) -> Result<PurgeReport, StoreError> {
        let mut removed = ItemCounts::default();
        let mut start = None;
        loop {
            let (keys, next) = self.key_page(catalog, start).await?;
            for key in &keys {
                if self.delete_existing(catalog, key).await? {
                    removed.add_key(key);
                }
            }
            if next.is_none() {
                return Ok(PurgeReport { removed });
            }
            start = next;
        }
    }
}
