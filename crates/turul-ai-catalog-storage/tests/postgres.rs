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
        let database = format!("catalog_{}_{nanos}", std::process::id());
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
    same_state_assembles_to_the_same_bytes,
    etag_is_the_digest_of_the_record_bytes,
    recreated_identical_record_has_the_same_etag,
    unversioned_entry_beside_versioned_ones_is_refused,
    versioned_entry_beside_an_unversioned_one_is_refused,
    same_version_kind_puts_are_accepted,
    deleting_the_last_record_of_a_kind_frees_the_identifier,
    conditional_puts_keep_their_preconditions,
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
async fn backend_is_named() {
    let Some(env) = Env::start("backend_is_named").await else {
        return;
    };
    assert_eq!(env.store().await.backend_name(), "postgres");
}
