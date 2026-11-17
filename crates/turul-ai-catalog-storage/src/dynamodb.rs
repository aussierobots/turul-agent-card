use async_trait::async_trait;
use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::error::{DisplayErrorContext, ProvideErrorMetadata, SdkError};
use aws_sdk_dynamodb::primitives::Blob;
use aws_sdk_dynamodb::types::AttributeValue;
use turul_ai_catalog::identity::EntryKey;

use crate::ids::{CatalogId, Etag, RecordKey};
use crate::records::{EntryRecord, HostRecord};
use crate::store::{CatalogStore, Condition, StoreError, Stored, corrupt, key_error};

const ITEM_LIMIT: &str = "record exceeds the DynamoDB item size limit of 400 KB";

/// Records in one DynamoDB table with partition key `pk` (S) and sort key `sk` (S). The table
/// belongs to the deployer: the store neither creates nor verifies it.
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
        self.put(catalog, &key, record.bytes(), condition).await
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
        let mut request = self
            .client
            .delete_item()
            .table_name(&self.table)
            .key("pk", s(catalog.as_str()))
            .key("sk", s(record_key.as_str()));
        request = match expected {
            None => request.condition_expression("attribute_exists(pk)"),
            Some(etag) => request
                .condition_expression("etag = :e")
                .expression_attribute_values(":e", s(etag.as_str())),
        };
        match request.send().await {
            Ok(_) => Ok(()),
            Err(e) if condition_failed(&e) => Err(match expected {
                None => StoreError::NotFound,
                Some(_) => StoreError::PreconditionFailed,
            }),
            Err(e) => Err(backend(e)),
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
