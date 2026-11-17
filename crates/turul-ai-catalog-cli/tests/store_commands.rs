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

const MAIN_COUNTS: &str = "host 0\nentries 2\nguards 2\nother 0\n";

fn purge_args<'a>(db: &'a Db, id: &'a str) -> Vec<&'a str> {
    vec!["store", "purge", "--sqlite", db.path(), "--catalog-id", id]
}

fn list_main(db: &Db) -> String {
    stdout(&run(
        &[
            "store",
            "list",
            "--sqlite",
            db.path(),
            "--catalog-id",
            "main",
        ],
        &[],
    ))
}

#[test]
fn purge_without_yes_previews_and_removes_nothing() {
    let db = Db::seeded("purge-preview");
    let out = run(&purge_args(&db, "main"), &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        format!("{MAIN_COUNTS}nothing removed; run again with --yes to remove\n")
    );
    assert_eq!(list_main(&db).lines().count(), 2);
}

#[test]
fn purge_with_yes_removes_every_item() {
    let db = Db::seeded("purge-yes");
    let out = run(&[&purge_args(&db, "main")[..], &["--yes"]].concat(), &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(stdout(&out), format!("{MAIN_COUNTS}removed 4 items\n"));
    assert_eq!(list_main(&db), "");
}

#[test]
fn purge_of_an_empty_catalog_reports_zeros_and_exits_0() {
    let db = Db::seeded("purge-empty");
    let out = run(&[&purge_args(&db, "none")[..], &["--yes"]].concat(), &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "host 0\nentries 0\nguards 0\nother 0\nremoved 0 items\n"
    );
}

#[test]
fn purge_refuses_the_live_catalog_id() {
    let db = Db::seeded("purge-live");
    // The flag equals the variable.
    let out = run(
        &[&purge_args(&db, "main")[..], &["--yes"]].concat(),
        &[("TURUL_AI_CATALOG_ID", "main")],
    );
    assert_eq!(code(&out), 2);
    assert_eq!(stdout(&out), "");
    assert!(
        stderr(&out).contains("TURUL_AI_CATALOG_ID"),
        "{}",
        stderr(&out)
    );
    // The flag is absent, so the id comes from the variable.
    let out = run(
        &["store", "purge", "--sqlite", db.path(), "--yes"],
        &[("TURUL_AI_CATALOG_ID", "main")],
    );
    assert_eq!(code(&out), 2);
    assert!(
        stderr(&out).contains("TURUL_AI_CATALOG_ID"),
        "{}",
        stderr(&out)
    );
    assert_eq!(list_main(&db).lines().count(), 2);
}

#[test]
fn purge_of_another_id_with_the_live_id_set_succeeds() {
    let db = Db::seeded("purge-other");
    let out = run(
        &[&purge_args(&db, "main")[..], &["--yes"]].concat(),
        &[("TURUL_AI_CATALOG_ID", "live")],
    );
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(list_main(&db), "");
}

fn catalog_file(test: &str, body: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("turul-store-{test}-{}.json", std::process::id()));
    std::fs::write(&path, body).unwrap();
    path
}

const ENTRY_A: &str = r#"{"identifier":"a","type":"application/mcp-server-card+json","url":"https://example.com/a.json"}"#;

fn catalog_of(entries: &[&str]) -> String {
    format!(
        r#"{{"specVersion":"1.0","entries":[{}]}}"#,
        entries.join(",")
    )
}

fn import_args<'a>(db: &'a Db, file: &'a str) -> Vec<&'a str> {
    vec![
        "store",
        "import",
        "--sqlite",
        db.path(),
        "--catalog-id",
        "fresh",
        "--file",
        file,
        "--writer",
        "op",
    ]
}

fn list_of(db: &Db, id: &str) -> String {
    stdout(&run(
        &["store", "list", "--sqlite", db.path(), "--catalog-id", id],
        &[],
    ))
}

#[test]
fn import_writes_curated_entries_and_exits_0() {
    let db = Db::seeded("import");
    let file = catalog_file("import", &catalog_of(&[ENTRY_A]));
    let out = run(&import_args(&db, file.to_str().unwrap()), &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(stdout(&out), "imported 1 entries\n");
    assert_eq!(list_of(&db, "fresh"), "a  -  curated  op  no expiry\n");
    std::fs::remove_file(file).unwrap();
}

#[test]
fn import_replace_removes_absent_entries() {
    let db = Db::seeded("import-replace");
    let file = catalog_file("import-replace", &catalog_of(&[ENTRY_A]));
    let mut args = import_args(&db, file.to_str().unwrap());
    args[5] = "main";
    // Without --replace the other writer's entry is refused.
    let out = run(&args, &[]);
    assert_eq!(code(&out), 2);
    args.push("--replace");
    let out = run(&args, &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(stdout(&out), "imported 1 entries\n");
    assert_eq!(list_main(&db), "a  -  curated  op  no expiry\n");
    std::fs::remove_file(file).unwrap();
}

#[test]
fn import_of_a_document_with_an_error_exits_1() {
    let db = Db::seeded("import-invalid");
    let file = catalog_file("import-invalid", &catalog_of(&[ENTRY_A, ENTRY_A]));
    let out = run(&import_args(&db, file.to_str().unwrap()), &[]);
    assert_eq!(code(&out), 1, "{}", stderr(&out));
    assert!(stderr(&out).contains("/entries/1"), "{}", stderr(&out));
    assert_eq!(list_of(&db, "fresh"), "");
    std::fs::remove_file(file).unwrap();
}

#[test]
fn import_replace_with_a_live_lease_exits_2() {
    let db = Db::seeded("import-live");
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let store = SqliteCatalogStore::connect(db.path()).await.unwrap();
            let entry = CatalogEntry::new(
                EntryIdentifier::new("live"),
                ArtifactType::MCP_SERVER_CARD,
                ArtifactSource::Url("https://example.com/card.json".to_string()),
            );
            store
                .put_entry(
                    &CatalogId::new("main").unwrap(),
                    &EntryRecord::new(entry).unwrap(),
                    &Provenance::Lease {
                        writer: WriterId::new("svc").unwrap(),
                        expires_at: Timestamp::parse("2999-01-01T00:00:00Z").unwrap(),
                    },
                    Condition::Absent,
                )
                .await
                .unwrap();
        });
    let file = catalog_file("import-live", &catalog_of(&[ENTRY_A]));
    let mut args = import_args(&db, file.to_str().unwrap());
    args[5] = "main";
    args.push("--replace");
    let out = run(&args, &[]);
    assert_eq!(code(&out), 2);
    let err = stderr(&out);
    assert!(
        err.contains("live") && err.contains("svc") && err.contains("2999-01-01T00:00:00Z"),
        "{err}"
    );
    assert_eq!(list_main(&db).lines().count(), 3);
    std::fs::remove_file(file).unwrap();
}

#[test]
fn import_follows_no_nested_catalog() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/child.json", listener.local_addr().unwrap());
    let nested =
        format!(r#"{{"identifier":"child","type":"application/ai-catalog+json","url":"{url}"}}"#);
    let db = Db::seeded("import-nested");
    let file = catalog_file("import-nested", &catalog_of(&[&nested]));
    let out = run(&import_args(&db, file.to_str().unwrap()), &[]);
    assert_eq!(code(&out), 0, "{}", stderr(&out));
    assert_eq!(list_of(&db, "fresh"), "child  -  curated  op  no expiry\n");
    assert!(listener.accept().is_err(), "no request must be sent");
    std::fs::remove_file(file).unwrap();
}
