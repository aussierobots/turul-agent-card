#![cfg(feature = "dynamodb")]

mod common;
mod parity;

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use aws_sdk_dynamodb::Client;
use aws_sdk_dynamodb::config::{Credentials, Region};
use aws_sdk_dynamodb::primitives::Blob;
use aws_sdk_dynamodb::types::AttributeValue;
use aws_sdk_dynamodb::types::{
    AttributeDefinition, BillingMode, KeySchemaElement, KeyType, ScalarAttributeType,
};
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_storage::{
    AssembleError, CatalogId, CatalogStore, Condition, DynamoDbCatalogStore, EntryRecord, Etag,
    ImportMode, ProvenanceError, StoreError, assemble, import,
};

const ENDPOINT: &str = "TURUL_AI_CATALOG_DYNAMODB_ENDPOINT";
const JAR: &str = "DYNAMODB_LOCAL_JAR";

/// One DynamoDB Local process shared by the tests of this file; the last guard to drop kills it.
struct Server {
    users: usize,
    child: Option<Child>,
    endpoint: String,
}

static SERVER: Mutex<Server> = Mutex::new(Server {
    users: 0,
    child: None,
    endpoint: String::new(),
});

fn jar() -> Option<PathBuf> {
    let jar = std::env::var_os(JAR).map(PathBuf::from).or_else(|| {
        std::env::var_os("HOME")
            .map(|h| PathBuf::from(h).join(".cache/dynamodb-local/DynamoDBLocal.jar"))
    })?;
    jar.is_file().then_some(jar)
}

fn java() -> Option<PathBuf> {
    let from_home = std::env::var_os("JAVA_HOME").map(|h| PathBuf::from(h).join("bin/java"));
    let from_path = std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|d| d.join("java"))
            .find(|c| c.is_file())
    });
    from_home
        .into_iter()
        .chain(from_path)
        .chain([PathBuf::from("/opt/homebrew/opt/openjdk/bin/java")])
        .find(|c| {
            // The macOS `/usr/bin/java` stub exists without a runtime behind it.
            c.is_file()
                && Command::new(c)
                    .arg("-version")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success())
        })
}

/// The server's working directory. DynamoDB Local writes its metadata file there, so it is never
/// the crate.
fn workdir() -> PathBuf {
    std::env::temp_dir().join(format!(
        "turul-ai-catalog-dynamodb-local-{}",
        std::process::id()
    ))
}

/// Starts (or joins) the shared server and returns its endpoint, or why it cannot run.
fn acquire_server() -> Result<String, String> {
    let mut server = SERVER.lock().unwrap_or_else(|e| e.into_inner());
    if server.child.is_none() {
        let jar = jar().ok_or_else(|| format!("no DynamoDB Local jar (set {JAR})"))?;
        let java = java().ok_or("no java (set JAVA_HOME)")?;
        let port = TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map_err(|e| e.to_string())?
            .port();
        std::fs::create_dir_all(workdir()).map_err(|e| e.to_string())?;
        let mut child = Command::new(java)
            .current_dir(workdir())
            .arg(format!(
                "-Djava.library.path={}",
                jar.with_file_name("DynamoDBLocal_lib").display()
            ))
            .arg("-jar")
            .arg(&jar)
            .args(["-inMemory", "-port", &port.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("cannot start DynamoDB Local: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            if Instant::now() > deadline || child.try_wait().ok().flatten().is_some() {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "DynamoDB Local started from {} did not answer",
                    jar.display()
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        server.child = Some(child);
        server.endpoint = format!("http://127.0.0.1:{port}");
    }
    server.users += 1;
    Ok(server.endpoint.clone())
}

fn release_server() {
    let mut server = SERVER.lock().unwrap_or_else(|e| e.into_inner());
    server.users -= 1;
    if server.users == 0
        && let Some(mut child) = server.child.take()
    {
        let _ = child.kill();
        let _ = child.wait();
        let _ = std::fs::remove_dir_all(workdir());
    }
}

fn client(endpoint: &str) -> Client {
    let config = aws_sdk_dynamodb::Config::builder()
        .behavior_version_latest()
        .endpoint_url(endpoint)
        .region(Region::new("us-east-1"))
        .credentials_provider(Credentials::new("test", "test", None, None, "test"))
        .build();
    Client::from_conf(config)
}

/// A uniquely named table and, unless the endpoint was supplied, its own server share. Dropping
/// it deletes the table and releases the server, also when the test panics.
struct Env {
    client: Client,
    table: String,
    owns_server: bool,
}

impl Env {
    async fn start(test: &str) -> Option<Env> {
        let (endpoint, owns_server) = match std::env::var(ENDPOINT) {
            Ok(endpoint) => (endpoint, false),
            Err(_) => match acquire_server() {
                Ok(endpoint) => (endpoint, true),
                Err(reason) => {
                    println!("skipping {test}: {reason}; set {ENDPOINT} or install DynamoDB Local");
                    return None;
                }
            },
        };
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let table = format!("catalog-{}-{nanos}-{test}", std::process::id());
        let env = Env {
            client: client(&endpoint),
            table,
            owns_server,
        };
        env.create_table().await;
        Some(env)
    }

    async fn create_table(&self) {
        let key = |name: &str, kind: KeyType| {
            KeySchemaElement::builder()
                .attribute_name(name)
                .key_type(kind)
                .build()
                .unwrap()
        };
        let attribute = |name: &str| {
            AttributeDefinition::builder()
                .attribute_name(name)
                .attribute_type(ScalarAttributeType::S)
                .build()
                .unwrap()
        };
        self.client
            .create_table()
            .table_name(&self.table)
            .key_schema(key("pk", KeyType::Hash))
            .key_schema(key("sk", KeyType::Range))
            .attribute_definitions(attribute("pk"))
            .attribute_definitions(attribute("sk"))
            .billing_mode(BillingMode::PayPerRequest)
            .send()
            .await
            .expect("create the test table");
    }

    fn store(&self) -> DynamoDbCatalogStore {
        DynamoDbCatalogStore::new(self.client.clone(), self.table.clone())
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let client = self.client.clone();
        let table = self.table.clone();
        // Drop can run inside a runtime, so the delete gets a runtime of its own.
        let _ = std::thread::spawn(move || {
            if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                let _ = runtime.block_on(client.delete_table().table_name(table).send());
            }
        })
        .join();
        if self.owns_server {
            release_server();
        }
    }
}

macro_rules! parity_checks {
    ($($name:ident),* $(,)?) => {
        $(
            #[tokio::test]
            #[ignore = "needs DynamoDB Local"]
            async fn $name() {
                let Some(env) = Env::start(stringify!($name)).await else { return };
                parity::$name(&env.store()).await;
            }
        )*
    };
}

parity_checks!(
    one_record_per_entry_key,
    stored_entry_reads_back_byte_identical,
    entry_trust_manifest_payload_survives_storage,
    catalogs_are_isolated,
    absent_condition_refuses_an_existing_record,
    matches_condition_refuses_a_stale_etag,
    delete_with_a_stale_etag_is_refused,
    delete_of_a_missing_record_is_not_found,
    writes_are_visible_to_the_next_read,
    list_returns_every_entry,
    same_state_and_now_assemble_to_the_same_bytes,
    expired_lease_keeps_its_version_kind,
    provenance_reads_back_exactly,
    provenance_that_would_not_read_back_is_refused,
    etag_is_the_digest_of_entry_and_provenance,
    lease_and_curated_puts_refuse_each_other,
    matches_condition_refuses_a_stale_lease_etag,
    recreated_identical_record_has_the_same_etag,
    unversioned_entry_beside_versioned_ones_is_refused,
    versioned_entry_beside_an_unversioned_one_is_refused,
    same_version_kind_puts_are_accepted,
    deleting_the_last_record_of_a_kind_frees_the_identifier,
    conditional_puts_keep_their_preconditions,
    purge_removes_every_item_of_the_catalog,
    purge_of_an_empty_catalog_removes_nothing,
    count_catalog_is_what_purge_removes,
    purge_leaves_other_catalogs_untouched,
    overwrite_replaces_a_record_of_either_mode,
    overwrite_requires_the_stored_version,
    overwrite_of_a_host_requires_its_version,
    scan_entries_matches_list_entries,
);

/// Checks that need a store they can share, such as with a `Registrar`.
macro_rules! shared_parity_checks {
    ($($name:ident),* $(,)?) => {
        $(
            #[tokio::test]
            #[ignore = "needs DynamoDB Local"]
            async fn $name() {
                let Some(env) = Env::start(stringify!($name)).await else { return };
                parity::$name(Arc::new(env.store())).await;
            }
        )*
    };
}

shared_parity_checks!(
    registered_identifier_reads_back_exactly,
    another_writer_cannot_take_over_an_entry,
    unregister_by_another_writer_is_refused,
    unregister_frees_the_identifier,
);

#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn purge_removes_many_entries() {
    let Some(env) = Env::start("purge_removes_many_entries").await else {
        return;
    };
    parity::purge_removes_many_entries(&env.store(), 300).await;
}

/// More than 1 MB of items, so the `Query` of a scan, a count and a purge returns several pages.
#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn purge_removes_items_across_several_query_pages() {
    let Some(env) = Env::start("purge_removes_items_across_several_query_pages").await else {
        return;
    };
    let store = env.store();
    let c = common::catalog("pages");
    for n in 0..60 {
        let mut entry = common::entry(&format!("urn:air:big{n}"), None);
        entry.description = Some("x".repeat(30 * 1024));
        store
            .put_entry(
                &c,
                &EntryRecord::new(entry).unwrap(),
                &common::curated(),
                Condition::Absent,
            )
            .await
            .unwrap();
    }
    assert_eq!(store.scan_entries(&c).await.unwrap().len(), 60);
    let counted = store.count_catalog(&c).await.unwrap();
    assert_eq!((counted.entries, counted.guards), (60, 60));
    assert_eq!(store.purge_catalog(&c).await.unwrap().removed, counted);
    assert!(items_of(&env, &c).await.is_empty());
}

#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn concurrent_absent_puts_have_one_winner() {
    let Some(env) = Env::start("concurrent_absent_puts_have_one_winner").await else {
        return;
    };
    let shared: Arc<dyn CatalogStore> = Arc::new(env.store());
    parity::concurrent_absent_puts_have_one_winner(shared).await;
}

#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn racing_version_kinds_leave_one_kind() {
    let Some(env) = Env::start("racing_version_kinds_leave_one_kind").await else {
        return;
    };
    let shared: Arc<dyn CatalogStore> = Arc::new(env.store());
    parity::racing_version_kinds_leave_one_kind(shared).await;
}

#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn racing_cross_mode_puts_leave_one_mode() {
    let Some(env) = Env::start("racing_cross_mode_puts_leave_one_mode").await else {
        return;
    };
    let shared: Arc<dyn CatalogStore> = Arc::new(env.store());
    parity::racing_cross_mode_puts_leave_one_mode(shared).await;
}

#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn concurrent_lease_refreshes_on_one_etag_have_one_winner() {
    let Some(env) = Env::start("concurrent_lease_refreshes_on_one_etag_have_one_winner").await
    else {
        return;
    };
    let shared: Arc<dyn CatalogStore> = Arc::new(env.store());
    parity::concurrent_lease_refreshes_on_one_etag_have_one_winner(shared).await;
}

#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn many_instances_refreshing_one_lease_all_succeed() {
    let Some(env) = Env::start("many_instances_refreshing_one_lease_all_succeed").await else {
        return;
    };
    let shared: Arc<dyn CatalogStore> = Arc::new(env.store());
    parity::many_instances_refreshing_one_lease_all_succeed(shared).await;
}

#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn backend_is_named() {
    let Some(env) = Env::start("backend_is_named").await else {
        return;
    };
    assert_eq!(env.store().backend_name(), "dynamodb");
}

/// More than 1 MB of entries, so `Query` returns several pages.
#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn list_follows_every_query_page() {
    let Some(env) = Env::start("list_follows_every_query_page").await else {
        return;
    };
    let store = env.store();
    let c = common::catalog("paged");
    let count = 150;
    for i in 0..count {
        let mut entry = common::entry(&format!("urn:air:p{i:04}"), None);
        entry.description = Some("x".repeat(10_000));
        let record = turul_ai_catalog_storage::EntryRecord::new(entry).unwrap();
        store
            .put_entry(&c, &record, &common::curated(), Condition::Any)
            .await
            .unwrap();
    }
    let listed = store.list_entries(&c).await.unwrap();
    assert!(listed.iter().map(|s| s.record.bytes().len()).sum::<usize>() > 1_048_576);
    assert_eq!(listed.len(), count);
}

#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn oversized_record_is_a_backend_error_naming_the_limit() {
    let Some(env) = Env::start("oversized_record_is_a_backend_error_naming_the_limit").await else {
        return;
    };
    let mut entry = common::entry("urn:air:big", None);
    entry.description = Some("x".repeat(401 * 1024));
    let record = turul_ai_catalog_storage::EntryRecord::new(entry).unwrap();
    let err = env
        .store()
        .put_entry(
            &common::catalog("big"),
            &record,
            &common::curated(),
            Condition::Any,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, turul_ai_catalog_storage::StoreError::Backend(m) if m.contains("400 KB")),
        "{err}"
    );
}

fn text(value: &str) -> AttributeValue {
    AttributeValue::S(value.to_string())
}

/// Writes `record` as the previous release did: no `provenance` or `mode`, the version the digest of
/// the record bytes, and its identifier's guard.
async fn put_previous_release_item(env: &Env, catalog: &CatalogId, record: &EntryRecord) {
    let key = record.record_key().unwrap();
    env.client
        .put_item()
        .table_name(&env.table)
        .item("pk", text(catalog.as_str()))
        .item("sk", text(key.as_str()))
        .item("record", AttributeValue::B(Blob::new(record.bytes())))
        .item("etag", text(Etag::of(record.bytes()).as_str()))
        .send()
        .await
        .unwrap();
    let guard = turul_ai_catalog_storage::RecordKey::guard(&record.entry().identifier).unwrap();
    env.client
        .put_item()
        .table_name(&env.table)
        .item("pk", text(catalog.as_str()))
        .item("sk", text(guard.as_str()))
        .item("versioned", AttributeValue::Bool(false))
        .item("entries", AttributeValue::N("1".to_string()))
        .send()
        .await
        .unwrap();
}

type Item = std::collections::HashMap<String, AttributeValue>;

/// Every item of `catalog`, in sort-key order.
async fn items_of(env: &Env, catalog: &CatalogId) -> Vec<Item> {
    let out = env
        .client
        .query()
        .table_name(&env.table)
        .key_condition_expression("pk = :c")
        .expression_attribute_values(":c", text(catalog.as_str()))
        .consistent_read(true)
        .send()
        .await
        .unwrap();
    out.items().to_vec()
}

#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn item_without_provenance_is_corrupt() {
    let Some(env) = Env::start("item_without_provenance_is_corrupt").await else {
        return;
    };
    let store = env.store();
    let c = common::catalog("old");
    let r = common::record("urn:air:a", None);
    put_previous_release_item(&env, &c, &r).await;
    let corrupt = |e: &StoreError| {
        matches!(
            e,
            StoreError::CorruptProvenance {
                error: ProvenanceError::Missing,
                ..
            }
        )
    };
    assert!(corrupt(&store.get_entry(&c, &r.key()).await.unwrap_err()));
    assert!(corrupt(&store.list_entries(&c).await.unwrap_err()));
    assert!(corrupt(
        &assemble(&store, &c, common::now())
            .await
            .map_err(|e| match e {
                AssembleError::Store(e) => e,
                other => panic!("{other:?}"),
            })
            .unwrap_err()
    ));
    // A write over it is refused too, not read as a curated replace.
    let err = store
        .put_entry(&c, &r, &common::curated(), Condition::Any)
        .await
        .unwrap_err();
    assert!(corrupt(&err), "{err:?}");
    assert!(corrupt(
        &store
            .put_entry(
                &c,
                &r,
                &common::curated(),
                Condition::Matches(Etag::of(r.bytes()))
            )
            .await
            .unwrap_err()
    ));
}

const DOCUMENT: &str = r#"{"specVersion":"1.0","host":{"displayName":"Op"},"entries":[{"identifier":"urn:air:a","type":"application/mcp-server-card+json","url":"https://example.com/a.json"},{"identifier":"urn:air:b","type":"application/mcp-server-card+json","url":"https://example.com/b.json"}]}"#;

const EXPECTED: &str = r#"{"entries":[{"identifier":"urn:air:a","type":"application/mcp-server-card+json","url":"https://example.com/a.json"},{"identifier":"urn:air:b","type":"application/mcp-server-card+json","url":"https://example.com/b.json"}],"host":{"displayName":"Op"},"specVersion":"1.0"}"#;

/// The upgrade path: a catalog id holding the previous release's items and a new one importing
/// beside it in the same table do not disturb each other.
#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn old_and_new_format_catalog_ids_share_a_table() {
    let Some(env) = Env::start("old_and_new_format_catalog_ids_share_a_table").await else {
        return;
    };
    let store = env.store();
    let (old, new) = (common::catalog("old"), common::catalog("new"));
    let document = parse(DOCUMENT.as_bytes()).unwrap();
    store
        .put_host(
            &old,
            &turul_ai_catalog_storage::HostRecord::new(document.host.clone().unwrap()).unwrap(),
            Condition::Any,
        )
        .await
        .unwrap();
    for entry in &document.entries {
        put_previous_release_item(&env, &old, &EntryRecord::new(entry.clone()).unwrap()).await;
    }
    let before = items_of(&env, &old).await;
    assert_eq!(before.len(), 5);

    import(
        &store,
        &new,
        &document,
        &common::writer("operator"),
        ImportMode::Merge,
    )
    .await
    .unwrap();
    let served = assemble(&store, &new, common::now()).await.unwrap();
    assert_eq!(std::str::from_utf8(&served.bytes).unwrap(), EXPECTED);
    assert_eq!(items_of(&env, &old).await, before);
    assert!(matches!(
        assemble(&store, &old, common::now()).await,
        Err(AssembleError::Store(StoreError::CorruptProvenance { .. }))
    ));
    assert_eq!(assemble(&store, &new, common::now()).await.unwrap(), served);

    // The purge reads none of the old items, which a read of this release refuses.
    let removed = store.purge_catalog(&old).await.unwrap().removed;
    assert_eq!(
        (removed.host, removed.entries, removed.guards, removed.other),
        (1, 2, 2, 0)
    );
    assert!(items_of(&env, &old).await.is_empty());
    assert_eq!(assemble(&store, &new, common::now()).await.unwrap(), served);
}

/// An entry item whose `mode` is neither `lease` nor `curated`.
#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn unknown_stored_mode_is_corrupt_provenance() {
    let Some(env) = Env::start("unknown_stored_mode_is_corrupt_provenance").await else {
        return;
    };
    let store = env.store();
    let c = common::catalog("unknown-mode");
    let r = common::record("urn:air:a", None);
    let etag = Etag::of_entry(&r, &common::curated()).unwrap();
    env.client
        .put_item()
        .table_name(&env.table)
        .item("pk", text(c.as_str()))
        .item("sk", text(r.record_key().unwrap().as_str()))
        .item("record", AttributeValue::B(Blob::new(r.bytes())))
        .item(
            "provenance",
            AttributeValue::B(Blob::new(common::curated().to_bytes().unwrap())),
        )
        .item("mode", text("other"))
        .item("etag", text(etag.as_str()))
        .send()
        .await
        .unwrap();
    parity::unknown_stored_mode_is_corrupt_provenance(&store, &c, &r, &etag).await;
}

/// Answers every request of its client with the DynamoDB error `body` and counts the answers.
#[derive(Debug)]
struct Conflicting {
    body: &'static str,
    answered: Arc<std::sync::atomic::AtomicUsize>,
}

impl aws_sdk_dynamodb::config::Intercept for Conflicting {
    fn name(&self) -> &'static str {
        "Conflicting"
    }

    fn modify_before_deserialization(
        &self,
        context: &mut aws_sdk_dynamodb::config::interceptors::BeforeDeserializationInterceptorContextMut<'_>,
        _: &aws_sdk_dynamodb::config::RuntimeComponents,
        _: &mut aws_sdk_dynamodb::config::ConfigBag,
    ) -> Result<(), aws_sdk_dynamodb::error::BoxError> {
        self.answered
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let response = context.response_mut();
        *response.status_mut() = 400u16.try_into().unwrap();
        response.headers_mut().remove("x-amz-crc32");
        *response.body_mut() = self.body.into();
        Ok(())
    }
}

/// A store over `env`'s table whose every request is answered with `body`, without SDK retries.
fn conflicting_store(
    env: &Env,
    body: &'static str,
) -> (DynamoDbCatalogStore, Arc<std::sync::atomic::AtomicUsize>) {
    let answered = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let config = env
        .client
        .config()
        .to_builder()
        .retry_config(aws_sdk_dynamodb::config::retry::RetryConfig::disabled())
        .interceptor(Conflicting {
            body,
            answered: Arc::clone(&answered),
        })
        .build();
    (
        DynamoDbCatalogStore::new(Client::from_conf(config), env.table.clone()),
        answered,
    )
}

/// A write that conflicts with other writers' transactions on every attempt stops after the
/// bound of 8 attempts with `Contended`, on the create, replace and overwrite paths.
#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn conflicts_on_every_attempt_are_contended() {
    let Some(env) = Env::start("conflicts_on_every_attempt_are_contended").await else {
        return;
    };
    let c = common::catalog("contended");
    let r = common::record("urn:air:a", None);

    let (store, answered) = conflicting_store(
        &env,
        r#"{"__type":"com.amazonaws.dynamodb.v20120810#TransactionCanceledException","message":"Transaction cancelled","CancellationReasons":[{"Code":"TransactionConflict"},{"Code":"None"}]}"#,
    );
    let created = store
        .put_entry(&c, &r, &common::curated(), Condition::Absent)
        .await;
    assert_eq!(created, Err(StoreError::Contended));
    assert_eq!(answered.load(std::sync::atomic::Ordering::SeqCst), 8);

    let (store, answered) = conflicting_store(
        &env,
        r#"{"__type":"com.amazonaws.dynamodb.v20120810#TransactionConflictException","message":"Transaction is ongoing for the item"}"#,
    );
    let replaced = store
        .put_entry(
            &c,
            &r,
            &common::curated(),
            Condition::Matches(Etag::of(b"any")),
        )
        .await;
    assert_eq!(replaced, Err(StoreError::Contended));
    assert_eq!(answered.load(std::sync::atomic::Ordering::SeqCst), 8);

    let (store, answered) = conflicting_store(
        &env,
        r#"{"__type":"com.amazonaws.dynamodb.v20120810#TransactionConflictException","message":"Transaction is ongoing for the item"}"#,
    );
    let overwritten = store
        .put_entry(
            &c,
            &r,
            &common::curated(),
            Condition::Overwrite(Etag::of(b"any")),
        )
        .await;
    assert_eq!(overwritten, Err(StoreError::Contended));
    assert_eq!(answered.load(std::sync::atomic::Ordering::SeqCst), 8);
}

/// Writes `raw` under `c` as an entry item whose provenance bytes do not read, and returns its
/// stored version.
async fn put_unreadable_provenance_item(env: &Env, c: &CatalogId, raw: &EntryRecord) -> Etag {
    let etag = Etag::of(raw.bytes());
    env.client
        .put_item()
        .table_name(&env.table)
        .item("pk", text(c.as_str()))
        .item("sk", text(raw.record_key().unwrap().as_str()))
        .item("record", AttributeValue::B(Blob::new(raw.bytes())))
        .item("provenance", AttributeValue::B(Blob::new(b"{}".as_slice())))
        .item("mode", text("curated"))
        .item("etag", text(etag.as_str()))
        .send()
        .await
        .unwrap();
    etag
}

#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn unreadable_items_are_scanned_and_overwritten() {
    let Some(env) = Env::start("unreadable_items_are_scanned_and_overwritten").await else {
        return;
    };
    let c = common::catalog("unreadable");
    let raw = common::record("urn:air:raw", None);
    let etag = put_unreadable_provenance_item(&env, &c, &raw).await;
    parity::unreadable_items_are_scanned_and_overwritten(&env.store(), &c, &raw, &etag).await;
}

#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn unreadable_items_are_counted_and_purged() {
    let Some(env) = Env::start("unreadable_items_are_counted_and_purged").await else {
        return;
    };
    let c = common::catalog("unreadable");
    let raw = common::record("urn:air:raw", None);
    put_unreadable_provenance_item(&env, &c, &raw).await;
    parity::unreadable_items_are_counted_and_purged(&env.store(), &c, &raw).await;
}

/// Items no release writes: an entry item without `provenance` or `mode`, and an item whose sort
/// key has no known prefix. A scan reports the first as unreadable provenance; a count and a purge
/// need only keys, so they count the second as `other` and remove both.
#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn items_without_provenance_or_a_known_key_are_counted_and_purged() {
    let Some(env) =
        Env::start("items_without_provenance_or_a_known_key_are_counted_and_purged").await
    else {
        return;
    };
    let store = env.store();
    let c = common::catalog("old");
    let r = common::record("urn:air:a", None);
    put_previous_release_item(&env, &c, &r).await;
    env.client
        .put_item()
        .table_name(&env.table)
        .item("pk", text(c.as_str()))
        .item("sk", text("x:unknown"))
        .send()
        .await
        .unwrap();
    store
        .put_host(&c, &common::host(), Condition::Any)
        .await
        .unwrap();
    let scanned = store.scan_entries(&c).await.unwrap();
    assert_eq!(scanned.len(), 1);
    assert_eq!(scanned[0].provenance, Err(ProvenanceError::Missing));
    assert_eq!(scanned[0].key, r.key());
    let counted = store.count_catalog(&c).await.unwrap();
    assert_eq!(
        (counted.host, counted.entries, counted.guards, counted.other),
        (1, 1, 1, 1)
    );
    assert_eq!(store.purge_catalog(&c).await.unwrap().removed, counted);
    assert!(items_of(&env, &c).await.is_empty());
}

/// Route A: `import` in replace mode upgrades, in place, a catalog id holding the previous
/// release's items, guards included.
#[tokio::test]
#[ignore = "needs DynamoDB Local"]
async fn replace_upgrades_an_old_format_catalog_in_place() {
    let Some(env) = Env::start("replace_upgrades_an_old_format_catalog_in_place").await else {
        return;
    };
    let store = env.store();
    let c = common::catalog("old");
    let document = parse(DOCUMENT.as_bytes()).unwrap();
    for entry in &document.entries {
        put_previous_release_item(&env, &c, &EntryRecord::new(entry.clone()).unwrap()).await;
    }
    // An entry the new document lacks.
    put_previous_release_item(&env, &c, &common::record("urn:air:gone", None)).await;
    import(
        &store,
        &c,
        &document,
        &common::writer("operator"),
        ImportMode::Replace { now: common::now() },
    )
    .await
    .unwrap();
    let served = assemble(&store, &c, common::now()).await.unwrap();
    assert_eq!(std::str::from_utf8(&served.bytes).unwrap(), EXPECTED);
    let items = items_of(&env, &c).await;
    assert!(
        items
            .iter()
            .filter(|i| matches!(i.get("sk"), Some(AttributeValue::S(k)) if k.starts_with("e:")))
            .all(|i| i.contains_key("provenance") && i.contains_key("mode"))
    );
    assert_eq!(store.count_catalog(&c).await.unwrap().entries, 2);
    // The guard of the deleted entry is gone with it, so its identifier is free.
    store
        .put_entry(
            &c,
            &common::record("urn:air:gone", Some("1")),
            &common::curated(),
            Condition::Absent,
        )
        .await
        .unwrap();
}
