//! The workspace's one dependency-direction test: the front ends and the lambda depend on exactly
//! the allowed crates, and their sources hold none of the named re-implementations. Needles are
//! built at run time so this file never contains the text it forbids.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use turul_ai_catalog::json::{Json, read};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

fn crate_dir(name: &str) -> PathBuf {
    workspace_root().join("crates").join(name)
}

fn metadata() -> Json {
    let out = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
        ])
        .current_dir(workspace_root())
        .output()
        .expect("cargo metadata runs");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    read(&out.stdout).expect("metadata is JSON")
}

fn get<'a>(json: &'a Json, name: &str) -> &'a Json {
    match json {
        Json::Object(m) => m.get(name).unwrap_or_else(|| panic!("no member {name}")),
        other => panic!("not an object: {other:?}"),
    }
}

fn array(json: &Json) -> &[Json] {
    match json {
        Json::Array(a) => a,
        other => panic!("not an array: {other:?}"),
    }
}

fn string(json: &Json) -> &str {
    match json {
        Json::String(s) => s,
        other => panic!("not a string: {other:?}"),
    }
}

/// (normal, dev) dependency names of a workspace package.
fn dependencies(package: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let meta = metadata();
    let pkg = array(get(&meta, "packages"))
        .iter()
        .find(|p| string(get(p, "name")) == package)
        .unwrap_or_else(|| panic!("package {package} is not a workspace member"));
    let (mut normal, mut dev) = (BTreeSet::new(), BTreeSet::new());
    for dep in array(get(pkg, "dependencies")) {
        let name = string(get(dep, "name")).to_string();
        match get(dep, "kind") {
            Json::Null => normal.insert(name),
            Json::String(k) if k == "dev" => dev.insert(name),
            other => panic!("unexpected dependency kind for {name}: {other:?}"),
        };
    }
    (normal, dev)
}

/// Names of a package's optional dependencies.
fn optional(package: &str) -> BTreeSet<String> {
    let meta = metadata();
    let pkg = array(get(&meta, "packages"))
        .iter()
        .find(|p| string(get(p, "name")) == package)
        .unwrap_or_else(|| panic!("package {package} is not a workspace member"));
    array(get(pkg, "dependencies"))
        .iter()
        .filter(|d| matches!(get(d, "optional"), Json::Bool(true)))
        .map(|d| string(get(d, "name")).to_string())
        .collect()
}

fn set(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

/// Every `.rs` file under `dir`, with its text.
fn sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(read_dir) = fs::read_dir(&d) else {
            continue;
        };
        for entry in read_dir {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = fs::read_to_string(&path).unwrap();
                out.push((path, text));
            }
        }
    }
    out.sort();
    out
}

fn src_of(crates: &[&str]) -> Vec<(PathBuf, String)> {
    crates
        .iter()
        .flat_map(|c| sources(&crate_dir(c).join("src")))
        .collect()
}

/// Needles found in code lines (comment lines are skipped).
fn hits(files: &[(PathBuf, String)], needles: &[String]) -> Vec<String> {
    let mut found = Vec::new();
    for (path, text) in files {
        for (n, line) in text.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            for needle in needles {
                if line.contains(needle.as_str()) {
                    found.push(format!("{}:{}: {needle}", path.display(), n + 1));
                }
            }
        }
    }
    found
}

fn needles(parts: &[&[&str]]) -> Vec<String> {
    parts.iter().map(|p| p.concat()).collect()
}

// the TUI depends on the format crate, the client, `ratatui` and `crossterm` only.
#[test]
fn tui_dependencies_are_exactly_the_allowed_set() {
    let (normal, dev) = dependencies("turul-ai-catalog-tui");
    assert_eq!(
        normal,
        set(&[
            "turul-ai-catalog",
            "turul-ai-catalog-client",
            "ratatui",
            "crossterm"
        ])
    );
    assert!(dev.is_empty(), "{dev:?}");
}

// the CLI depends on the format crate, the client, the TUI and `clap` only.
#[test]
fn cli_dependencies_are_exactly_the_allowed_set() {
    let (normal, dev) = dependencies("turul-ai-catalog-cli");
    assert_eq!(
        normal,
        set(&[
            "turul-ai-catalog",
            "turul-ai-catalog-client",
            "turul-ai-catalog-tui",
            "clap"
        ])
    );
    assert!(dev.is_empty(), "{dev:?}");
}

// the lambda serves a catalog assembled from a store; it never depends on the client or the trust
// crate. `axum` exists only for the local development server (feature `local`); `aws-config` and
// `aws-sdk-dynamodb` only for the DynamoDB backend (feature `dynamodb`).
#[test]
fn lambda_dependencies_are_exactly_the_allowed_set() {
    let (normal, dev) = dependencies("turul-ai-catalog-lambda");
    assert_eq!(
        normal,
        set(&[
            "turul-ai-catalog",
            "turul-ai-catalog-storage",
            "lambda_http",
            "tokio",
            "axum",
            "aws-config",
            "aws-sdk-dynamodb"
        ])
    );
    assert_eq!(
        optional("turul-ai-catalog-lambda"),
        set(&["axum", "aws-config", "aws-sdk-dynamodb"])
    );
    assert_eq!(
        dev,
        set(&[
            "turul-ai-catalog-client",
            "ai-catalog",
            "ai-catalog-validate",
            "futures",
            "async-trait"
        ])
    );
}

// the storage crate's default features add no runtime or network dependency, never the client or
// trust crate, and the format crate keeps zero dependencies.
#[test]
fn storage_default_features_add_no_runtime_or_network_dependency() {
    let (normal, _) = dependencies("turul-ai-catalog-storage");
    let optional = optional("turul-ai-catalog-storage");
    assert_eq!(optional, set(&["sqlx", "aws-sdk-dynamodb"]));
    let always: std::collections::BTreeSet<_> = normal.difference(&optional).cloned().collect();
    assert_eq!(always, set(&["turul-ai-catalog", "async-trait", "sha2"]));
    let (core, _) = dependencies("turul-ai-catalog");
    assert!(core.is_empty(), "{core:?}");
}

// #embedded-content-safety: nothing in a front end or the lambda touches the network, the file
// system or a child process.
#[test]
fn sources_have_no_network_filesystem_or_process_access() {
    // the local development server (`src/bin/`, feature `local`) reads a file and binds a
    // port by design; the deployed lambda (`src/lib.rs`, `src/main.rs`) does neither.
    let files: Vec<_> = src_of(&[
        "turul-ai-catalog-tui",
        "turul-ai-catalog-cli",
        "turul-ai-catalog-lambda",
    ])
    .into_iter()
    .filter(|(path, _)| !path.ends_with("turul-ai-catalog-lambda/src/bin/serve.rs"))
    .collect();
    assert!(!files.is_empty(), "no sources found");
    let found = hits(
        &files,
        &needles(&[
            &["std", "::net"],
            &["std", "::fs"],
            &["std::process::", "Command"],
            &["Command", "::new"],
            &["Tcp", "Stream"],
        ]),
    );
    assert!(found.is_empty(), "{found:#?}");
}

// parse, validate, resolve, nesting and identifier splitting have one owner.
#[test]
fn front_ends_do_not_reimplement_consumer_logic() {
    let files = src_of(&["turul-ai-catalog-tui", "turul-ai-catalog-cli"]);
    assert!(!files.is_empty(), "no sources found");
    let found = hits(
        &files,
        &needles(&[
            &["parse::", "parse"],
            &["validate::", "validate"],
            &["validate", "_at"],
            &["resolve", "::"],
            &["Nesting", "Depth"],
            &["split(", "':')"],
            &["rsplit(", "':')"],
            &["urn", ":air"],
            &["req", "west"],
            &["serde", "_json"],
        ]),
    );
    assert!(found.is_empty(), "{found:#?}");
}

// the front ends never build or edit a catalog.
#[test]
fn front_ends_do_not_build_or_edit_catalogs() {
    let files = src_of(&["turul-ai-catalog-tui", "turul-ai-catalog-cli"]);
    assert!(!files.is_empty(), "no sources found");
    let found = hits(
        &files,
        &needles(&[
            &["AiCatalog", "::new"],
            &["CatalogEntry", "::new"],
            &["HostInfo", "::new"],
            &["canonical::", "to_canonical_vec"],
            &["entries", ".push"],
            &["entries", ".retain"],
            &[".", "key()"],
        ]),
    );
    assert!(found.is_empty(), "{found:#?}");
}

// `produce` is the one writer of catalog files.
#[test]
fn only_produce_writes_catalog_files() {
    let crates_dir = workspace_root().join("crates");
    let mut files = Vec::new();
    for entry in fs::read_dir(&crates_dir).unwrap() {
        files.extend(sources(&entry.unwrap().path().join("src")));
    }
    assert!(!files.is_empty(), "no sources found");
    let allowed = crate_dir("turul-ai-catalog-client")
        .join("src")
        .join("produce.rs");
    let others: Vec<_> = files.into_iter().filter(|(p, _)| *p != allowed).collect();
    let found = hits(
        &others,
        &needles(&[
            &["fs::", "rename"],
            &["fs::", "write"],
            &["File::", "create"],
        ]),
    );
    assert!(found.is_empty(), "{found:#?}");
}

// the well-known path and the media type are spelled only in the format crate.
#[test]
fn only_the_format_crate_spells_the_well_known_path_or_media_type() {
    let crates_dir = workspace_root().join("crates");
    let mut files = Vec::new();
    for entry in fs::read_dir(&crates_dir).unwrap() {
        let path = entry.unwrap().path();
        if path.file_name().is_some_and(|n| n == "turul-ai-catalog") {
            continue;
        }
        files.extend(sources(&path.join("src")));
    }
    assert!(!files.is_empty(), "no sources found");
    let found = hits(
        &files,
        &needles(&[
            &["/.well-known/", "ai-catalog.json"],
            &["application/", "ai-catalog+json"],
        ]),
    );
    assert!(found.is_empty(), "{found:#?}");
}
