use async_trait::async_trait;
use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::error::{DisplayErrorContext, ProvideErrorMetadata, SdkError};
use aws_sdk_dynamodb::operation::transact_write_items::TransactWriteItemsError;
use aws_sdk_dynamodb::primitives::Blob;
use aws_sdk_dynamodb::types::{
    AttributeValue, ConditionCheck, Delete, Put, TransactWriteItem, Update,
};
use turul_ai_catalog::identity::EntryKey;

use crate::ids::{CatalogId, Etag, RecordKey};
use crate::records::{EntryRecord, HostRecord};
use crate::store::{
    CatalogStore, Condition, StoreError, Stored, corrupt, key_error, kind_conflict,
};

const ITEM_LIMIT: &str = "record exceeds the DynamoDB item size limit of 400 KB";

/// Guard item attributes: the identifier's version kind (BOOL) and its entry count (N).
const VERSIONED: &str = "versioned";
const ENTRIES: &str = "entries";

const CONDITION_FAILED: &str = "ConditionalCheckFailed";
const TRANSACTION_CONFLICT: &str = "TransactionConflict";

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
            Condition::Matches(expected) => {
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

    /// Replaces an existing entry item, with `expected` its required version. Fails
    /// `PreconditionFailed` when the item is absent or its version differs.
    async fn replace(
        &self,
        catalog: &CatalogId,
        key: &RecordKey,
        bytes: &[u8],
        expected: Option<&Etag>,
    ) -> Result<Etag, StoreError> {
        let etag = Etag::of(bytes);
        loop {
            let request = self
                .client
                .put_item()
                .table_name(&self.table)
                .item("pk", s(catalog.as_str()))
                .item("sk", s(key.as_str()))
                .item("record", AttributeValue::B(Blob::new(bytes)))
                .item("etag", s(etag.as_str()));
            let request = match expected {
                None => request.condition_expression("attribute_exists(pk)"),
                Some(e) => request
                    .condition_expression("etag = :e")
                    .expression_attribute_values(":e", s(e.as_str())),
            };
            match request.send().await {
                Ok(_) => return Ok(etag),
                Err(e) if condition_failed(&e) => return Err(StoreError::PreconditionFailed),
                // The item is inside another writer's transaction.
                Err(e) if e.code() == Some("TransactionConflictException") => continue,
                Err(e) => return Err(backend(e)),
            }
        }
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
        condition: Condition,
    ) -> Result<Etag, StoreError> {
        let key = record.record_key().map_err(key_error)?;
        if let Condition::Matches(expected) = &condition {
            return self
                .replace(catalog, &key, record.bytes(), Some(expected))
                .await;
        }
        let guard = RecordKey::guard(&record.entry().identifier).map_err(key_error)?;
        let etag = Etag::of(record.bytes());
        loop {
            let create = Put::builder()
                .table_name(&self.table)
                .item("pk", s(catalog.as_str()))
                .item("sk", s(key.as_str()))
                .item("record", AttributeValue::B(Blob::new(record.bytes())))
                .item("etag", s(etag.as_str()))
                .condition_expression("attribute_not_exists(pk)")
                .build()
                .map_err(build_error)?;
            // A new entry is refused when its identifier's guard holds the other version kind.
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
                    AttributeValue::Bool(record.entry().version.is_some()),
                )
                .expression_attribute_values(":one", AttributeValue::N("1".to_string()))
                .build()
                .map_err(build_error)?;
            let items = vec![
                TransactWriteItem::builder().put(create).build(),
                TransactWriteItem::builder().update(count).build(),
            ];
            let Some(cancelled) = self.transact(items).await? else {
                return Ok(etag);
            };
            if cancelled.failed(0) {
                if condition == Condition::Absent {
                    return Err(StoreError::PreconditionFailed);
                }
                // `Any` on an existing entry is a replace, which leaves the kinds unchanged; an
                // entry deleted meanwhile is created on the next attempt.
                match self.replace(catalog, &key, record.bytes(), None).await {
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
    }

    async fn get_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
    ) -> Result<Option<Stored<EntryRecord>>, StoreError> {
        let record_key = RecordKey::entry(key).map_err(key_error)?;
        let Some((bytes, etag)) = self.get(catalog, &record_key).await? else {
            return Ok(None);
        };
        let record = EntryRecord::from_stored(bytes).map_err(|e| corrupt(&record_key, e))?;
        Ok(Some(Stored { record, etag }))
    }

    async fn delete_entry(
        &self,
        catalog: &CatalogId,
        key: &EntryKey,
        expected: Option<&Etag>,
    ) -> Result<(), StoreError> {
        let record_key = RecordKey::entry(key).map_err(key_error)?;
        let guard = RecordKey::guard(key.identifier()).map_err(key_error)?;
        loop {
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
    }

    async fn list_entries(
        &self,
        catalog: &CatalogId,
    ) -> Result<Vec<Stored<EntryRecord>>, StoreError> {
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
                let etag = Etag::of(&bytes);
                let record = EntryRecord::from_stored(bytes)
                    .map_err(|e| corrupt(&RecordKey::from_stored_name(name), e))?;
                listed.push(Stored { record, etag });
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
        let record = HostRecord::from_stored(bytes).map_err(|e| corrupt(&RecordKey::HOST, e))?;
        Ok(Some(Stored { record, etag }))
    }
}
