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
use aws_sdk_dynamodb::types::{
    AttributeDefinition, BillingMode, KeySchemaElement, KeyType, ScalarAttributeType,
};
use turul_ai_catalog_storage::{CatalogStore, Condition, DynamoDbCatalogStore};

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
    same_state_assembles_to_the_same_bytes,
    etag_is_the_digest_of_the_record_bytes,
    recreated_identical_record_has_the_same_etag,
);

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
        store.put_entry(&c, &record, Condition::Any).await.unwrap();
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
        .put_entry(&common::catalog("big"), &record, Condition::Any)
        .await
        .unwrap_err();
    assert!(
        matches!(&err, turul_ai_catalog_storage::StoreError::Backend(m) if m.contains("400 KB")),
        "{err}"
    );
}
