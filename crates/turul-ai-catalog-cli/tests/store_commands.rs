//! CLI `store list` and `store remove`: the real binary against a SQLite file seeded through the
//! storage crate.

use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
use turul_ai_catalog::model::{ArtifactSource, CatalogEntry};
use turul_ai_catalog::timestamp::Timestamp;
use turul_ai_catalog_storage::{
    CatalogId, CatalogStore, Condition, EntryRecord, Provenance, SqliteCatalogStore, WriterId,
};

struct Db(PathBuf);

impl Db {
    /// A fresh database holding two entries of catalog `main`: a curated `a` and a lease `b@1`
    /// that expired long ago.
    fn seeded(test: &str) -> Db {
        let path =
            std::env::temp_dir().join(format!("turul-store-{test}-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let db = Db(path);
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let store = SqliteCatalogStore::connect(db.path()).await.unwrap();
                let main = CatalogId::new("main").unwrap();
                let writer = WriterId::new("svc").unwrap();
                for (id, version, provenance) in [
                    (
                        "a",
                        None,
                        Provenance::Curated {
                            writer: writer.clone(),
                        },
                    ),
                    (
                        "b",
                        Some("1"),
                        Provenance::Lease {
                            writer,
                            expires_at: Timestamp::parse("2020-01-01T00:00:00Z").unwrap(),
                        },
                    ),
                ] {
                    let mut entry = CatalogEntry::new(
                        EntryIdentifier::new(id),
                        ArtifactType::MCP_SERVER_CARD,
                        ArtifactSource::Url("https://example.com/card.json".to_string()),
                    );
                    entry.version = version.map(str::to_string);
                    store
                        .put_entry(
                            &main,
                            &EntryRecord::new(entry).unwrap(),
                            &provenance,
                            Condition::Absent,
                        )
                        .await
                        .unwrap();
                }
            });
        db
    }

    fn path(&self) -> &str {
        self.0.to_str().unwrap()
    }
}

impl Drop for Db {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn run(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_turul-ai-catalog"));
    for name in [
        "TURUL_AI_CATALOG_ID",
        "TURUL_AI_CATALOG_SQLITE",
        "TURUL_AI_CATALOG_TABLE",
        "TURUL_AI_CATALOG_POSTGRES_URL",
    ] {
        command.env_remove(name);
    }
    command
        .args(args)
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        .output()
        .expect("binary runs")
}

fn code(out: &Output) -> i32 {
    out.status.code().expect("exit code")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn list_prints_provenance_and_expiry_state() {
    let db = Db::seeded("list");
    let out = run(
        &[
            "store",
            "list",
            "--sqlite",
            db.path(),
            "--catalog-id",
            "main",
        ],
        &[],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "a  -  curated  svc  no expiry\nb  1  lease  svc  expired at 2020-01-01T00:00:00Z\n"
    );
    let empty = run(
        &[
            "store",
            "list",
            "--sqlite",
            db.path(),
            "--catalog-id",
            "none",
        ],
        &[],
    );
    assert_eq!((code(&empty), stdout(&empty)), (0, String::new()));
}

#[test]
fn remove_deletes_the_entry_and_exits_0() {
    let db = Db::seeded("remove");
    let args = [
        "store",
        "remove",
        "--sqlite",
        db.path(),
        "--catalog-id",
        "main",
    ];
    let out = run(
        &[&args[..], &["--identifier", "b", "--version", "1"]].concat(),
        &[],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(stdout(&out), "removed b@1\n");
    let left = run(
        &[
            "store",
            "list",
            "--sqlite",
            db.path(),
            "--catalog-id",
            "main",
        ],
        &[],
    );
    assert_eq!(stdout(&left), "a  -  curated  svc  no expiry\n");
}

#[test]
fn remove_without_a_match_exits_2() {
    let db = Db::seeded("nomatch");
    let args = [
        "store",
        "remove",
        "--sqlite",
        db.path(),
        "--catalog-id",
        "main",
    ];
    // no `--version` names the unversioned key, which `b` does not have
    let out = run(&[&args[..], &["--identifier", "b"]].concat(), &[]);
    assert_eq!(code(&out), 2);
    assert_eq!(stdout(&out), "");
    assert!(stderr(&out).contains("no entry"), "{}", stderr(&out));
    let left = run(
        &[
            "store",
            "list",
            "--sqlite",
            db.path(),
            "--catalog-id",
            "main",
        ],
        &[],
    );
    assert_eq!(stdout(&left).lines().count(), 2);
}

#[test]
fn no_backend_is_a_usage_error() {
    let out = run(&["store", "list", "--catalog-id", "main"], &[]);
    assert_eq!(code(&out), 2);
}

// A second backend can only exist where its feature is built.
#[cfg(feature = "dynamodb")]
#[test]
fn two_backends_are_a_usage_error() {
    let db = Db::seeded("two");
    let out = run(
        &[
            "store",
            "list",
            "--sqlite",
            db.path(),
            "--table",
            "t",
            "--catalog-id",
            "main",
        ],
        &[],
    );
    assert_eq!(code(&out), 2);
}

#[test]
fn backend_and_catalog_id_come_from_the_environment() {
    let db = Db::seeded("env");
    let out = run(
        &["store", "list"],
        &[
            ("TURUL_AI_CATALOG_SQLITE", db.path()),
            ("TURUL_AI_CATALOG_ID", "main"),
        ],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(stdout(&out).lines().count(), 2);
}

#[test]
fn an_invalid_catalog_id_exits_2() {
    let db = Db::seeded("badid");
    let out = run(
        &["store", "list", "--sqlite", db.path(), "--catalog-id", ""],
        &[],
    );
    assert_eq!(code(&out), 2);
    assert!(!stderr(&out).is_empty());
}
