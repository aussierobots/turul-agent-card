#![cfg(feature = "postgres")]

mod common;
mod parity;

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use turul_ai_catalog_storage::{CatalogStore, PostgresCatalogStore};

const URL: &str = "TURUL_AI_CATALOG_POSTGRES_URL";
const BIN: &str = "POSTGRES_BIN";

/// One PostgreSQL server shared by the tests of this file; the last guard to drop stops it.
struct Server {
    users: usize,
    bin: PathBuf,
    port: u16,
    started: bool,
}

static SERVER: Mutex<Server> = Mutex::new(Server {
    users: 0,
    bin: PathBuf::new(),
    port: 0,
    started: false,
});

/// The directory holding `initdb`, `pg_ctl` and `createdb`.
fn bin_dir() -> Option<PathBuf> {
    let from_env = std::env::var_os(BIN).map(PathBuf::from);
    let from_path = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default();
    from_env
        .into_iter()
        .chain(from_path)
        .chain([PathBuf::from("/opt/homebrew/opt/postgresql@17/bin")])
        .find(|d| {
            ["initdb", "pg_ctl", "createdb"]
                .iter()
                .all(|b| d.join(b).is_file())
        })
}

/// The data directory and socket directory. Kept short: a unix socket path is limited to about
/// 100 bytes.
fn datadir() -> PathBuf {
    std::env::temp_dir().join(format!("tac-pg-{}", std::process::id()))
}

fn run(program: PathBuf, args: &[&str]) -> Result<(), String> {
    let out = Command::new(&program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("cannot run {}: {e}", program.display()))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} failed: {}",
            program.display(),
            String::from_utf8_lossy(&out.stderr)
        ))
    }
}

/// Starts (or joins) the shared server and returns its bin directory and port, or why it cannot
/// run.
fn acquire_server() -> Result<(PathBuf, u16), String> {
    let mut server = SERVER.lock().unwrap_or_else(|e| e.into_inner());
    if !server.started {
        let bin = bin_dir().ok_or_else(|| format!("no initdb and pg_ctl (set {BIN})"))?;
        let port = TcpListener::bind("127.0.0.1:0")
            .and_then(|l| l.local_addr())
            .map_err(|e| e.to_string())?
            .port();
        let dir = datadir();
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let data = dir.join("data");
        let log = dir.join("server.log");
        let started = run(
            bin.join("initdb"),
            &[
                "-D",
                &data.to_string_lossy(),
                "-A",
                "trust",
                "-U",
                "postgres",
                "--no-locale",
                "-E",
                "UTF8",
            ],
        )
        .and_then(|()| {
            run(
                bin.join("pg_ctl"),
                &[
                    "-w",
                    "-D",
                    &data.to_string_lossy(),
                    "-l",
                    &log.to_string_lossy(),
                    "-o",
                    &format!(
                        "-p {port} -k {} -c listen_addresses=127.0.0.1",
                        dir.display()
                    ),
                    "start",
                ],
            )
        });
        if let Err(e) = started {
            let _ = run(
                bin.join("pg_ctl"),
                &["-D", &data.to_string_lossy(), "-m", "immediate", "stop"],
            );
            let _ = std::fs::remove_dir_all(&dir);
            panic!("PostgreSQL from {} did not start: {e}", bin.display());
        }
        server.bin = bin;
        server.port = port;
        server.started = true;
    }
    server.users += 1;
    Ok((server.bin.clone(), server.port))
}

fn release_server() {
    let mut server = SERVER.lock().unwrap_or_else(|e| e.into_inner());
    server.users -= 1;
    if server.users == 0 && server.started {
        let dir = datadir();
        let _ = run(
            server.bin.join("pg_ctl"),
            &[
                "-w",
                "-D",
                &dir.join("data").to_string_lossy(),
                "-m",
                "fast",
                "stop",
            ],
        );
        let _ = std::fs::remove_dir_all(&dir);
        server.started = false;
    }
}

/// A uniquely named database and, unless a URL was supplied, its share of the server. Dropping it
/// releases the server, which is stopped and deleted by the last guard, also when a test panics.
struct Env {
    url: String,
    owns_server: bool,
}

impl Env {
    async fn start(test: &str) -> Option<Env> {
        if let Ok(url) = std::env::var(URL) {
            return Some(Env {
                url,
                owns_server: false,
            });
        }
        let (bin, port) = match acquire_server() {
            Ok(s) => s,
            Err(reason) => {
                println!("skipping {test}: {reason}; set {URL} or install PostgreSQL");
                return None;
            }
        };
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let database = format!("catalog_{}_{nanos}_{n}", std::process::id());
        // The guard exists before the database does, so a failure still releases the server.
        let env = Env {
            url: format!("postgres://postgres@127.0.0.1:{port}/{database}"),
            owns_server: true,
        };
        run(
            bin.join("createdb"),
            &[
                "-h",
                "127.0.0.1",
                "-p",
                &port.to_string(),
                "-U",
                "postgres",
                &database,
            ],
        )
        .expect("create the test database");
        Some(env)
    }

    async fn store(&self) -> PostgresCatalogStore {
        PostgresCatalogStore::connect(&self.url).await.unwrap()
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        if self.owns_server {
            release_server();
        }
    }
}

macro_rules! parity_checks {
    ($($name:ident),* $(,)?) => {
        $(
            #[tokio::test]
            #[ignore = "needs PostgreSQL"]
            async fn $name() {
                let Some(env) = Env::start(stringify!($name)).await else { return };
                parity::$name(&env.store().await).await;
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
            #[ignore = "needs PostgreSQL"]
            async fn $name() {
                let Some(env) = Env::start(stringify!($name)).await else { return };
                parity::$name(Arc::new(env.store().await)).await;
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
#[ignore = "needs PostgreSQL"]
async fn concurrent_absent_puts_have_one_winner() {
    let Some(env) = Env::start("concurrent_absent_puts_have_one_winner").await else {
        return;
    };
    let shared: Arc<dyn CatalogStore> = Arc::new(env.store().await);
    parity::concurrent_absent_puts_have_one_winner(shared).await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn racing_version_kinds_leave_one_kind() {
    let Some(env) = Env::start("racing_version_kinds_leave_one_kind").await else {
        return;
    };
    let shared: Arc<dyn CatalogStore> = Arc::new(env.store().await);
    parity::racing_version_kinds_leave_one_kind(shared).await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn racing_cross_mode_puts_leave_one_mode() {
    let Some(env) = Env::start("racing_cross_mode_puts_leave_one_mode").await else {
        return;
    };
    let shared: Arc<dyn CatalogStore> = Arc::new(env.store().await);
    parity::racing_cross_mode_puts_leave_one_mode(shared).await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn concurrent_lease_refreshes_on_one_etag_have_one_winner() {
    let Some(env) = Env::start("concurrent_lease_refreshes_on_one_etag_have_one_winner").await
    else {
        return;
    };
    let shared: Arc<dyn CatalogStore> = Arc::new(env.store().await);
    parity::concurrent_lease_refreshes_on_one_etag_have_one_winner(shared).await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn many_instances_refreshing_one_lease_all_succeed() {
    let Some(env) = Env::start("many_instances_refreshing_one_lease_all_succeed").await else {
        return;
    };
    let shared: Arc<dyn CatalogStore> = Arc::new(env.store().await);
    parity::many_instances_refreshing_one_lease_all_succeed(shared).await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn purge_removes_many_entries() {
    let Some(env) = Env::start("purge_removes_many_entries").await else {
        return;
    };
    parity::purge_removes_many_entries(&env.store().await, 300).await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn backend_is_named() {
    let Some(env) = Env::start("backend_is_named").await else {
        return;
    };
    assert_eq!(env.store().await.backend_name(), "postgres");
}

/// A table created by an earlier release has no provenance columns, so connecting fails and names
/// the column instead of reading its records as curated.
#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn table_from_an_earlier_release_fails_connect() {
    let Some(env) = Env::start("table_from_an_earlier_release_fails_connect").await else {
        return;
    };
    let pool = sqlx::PgPool::connect(&env.url).await.unwrap();
    sqlx::query(
        "CREATE TABLE catalog_records (catalog_id TEXT NOT NULL, record_key TEXT NOT NULL, \
         record BYTEA NOT NULL, etag TEXT NOT NULL, PRIMARY KEY (catalog_id, record_key))",
    )
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    let Err(turul_ai_catalog_storage::StoreError::Backend(message)) =
        PostgresCatalogStore::connect(&env.url).await
    else {
        panic!("connect must refuse a table without provenance columns");
    };
    assert!(message.contains("provenance"), "{message}");
}

/// A row whose mode is neither `lease` nor `curated`, written with the `CHECK` dropped.
#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn unknown_stored_mode_is_corrupt_provenance() {
    let Some(env) = Env::start("unknown_stored_mode_is_corrupt_provenance").await else {
        return;
    };
    let store = env.store().await;
    let c = common::catalog("unknown-mode");
    let r = common::record("urn:air:a", None);
    let etag = turul_ai_catalog_storage::Etag::of_entry(&r, &common::curated()).unwrap();
    let pool = sqlx::PgPool::connect(&env.url).await.unwrap();
    sqlx::query("ALTER TABLE catalog_records DROP CONSTRAINT catalog_records_check")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO catalog_records (catalog_id, record_key, record, etag, provenance, mode) \
         VALUES ($1, $2, $3, $4, $5, 'other')",
    )
    .bind(c.as_str())
    .bind(r.record_key().unwrap().as_str())
    .bind(r.bytes())
    .bind(etag.as_str())
    .bind(common::curated().to_bytes().unwrap())
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    parity::unknown_stored_mode_is_corrupt_provenance(&store, &c, &r, &etag).await;
}

/// Writes `raw` as an entry row under `c` whose provenance bytes do not read, and returns its
/// stored version.
async fn inject_unreadable(
    env: &Env,
    c: &turul_ai_catalog_storage::CatalogId,
    raw: &turul_ai_catalog_storage::EntryRecord,
) -> turul_ai_catalog_storage::Etag {
    let etag = turul_ai_catalog_storage::Etag::of(raw.bytes());
    let pool = sqlx::PgPool::connect(&env.url).await.unwrap();
    sqlx::query(
        "INSERT INTO catalog_records (catalog_id, record_key, record, etag, provenance, mode) \
         VALUES ($1, $2, $3, $4, $5, 'curated')",
    )
    .bind(c.as_str())
    .bind(raw.record_key().unwrap().as_str())
    .bind(raw.bytes())
    .bind(etag.as_str())
    .bind(b"{}".as_slice())
    .execute(&pool)
    .await
    .unwrap();
    pool.close().await;
    etag
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn unreadable_items_are_scanned_and_overwritten() {
    let Some(env) = Env::start("unreadable_items_are_scanned_and_overwritten").await else {
        return;
    };
    let c = common::catalog("unreadable");
    let raw = common::record("urn:air:raw", None);
    let store = env.store().await;
    let etag = inject_unreadable(&env, &c, &raw).await;
    parity::unreadable_items_are_scanned_and_overwritten(&store, &c, &raw, &etag).await;
}

#[tokio::test]
#[ignore = "needs PostgreSQL"]
async fn unreadable_items_are_counted_and_purged() {
    let Some(env) = Env::start("unreadable_items_are_counted_and_purged").await else {
        return;
    };
    let c = common::catalog("unreadable");
    let raw = common::record("urn:air:raw", None);
    let store = env.store().await;
    inject_unreadable(&env, &c, &raw).await;
    parity::unreadable_items_are_counted_and_purged(&store, &c, &raw).await;
}
