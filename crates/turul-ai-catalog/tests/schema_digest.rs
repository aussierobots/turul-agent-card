//! The one schema-digest algorithm, shared by every crate's golden fixtures, and the
//! recorded digest of the canonical goldens.

use std::fs;
use std::path::{Path, PathBuf};

#[path = "support/schema_digest.rs"]
mod owner;
use owner::schema_digest;

/// Recorded digest of `tests/fixtures/canonical/`.
const RECORDED: &str = "a24024107f47ecddd78a22c935c52f24ab9a6f5a9f4d637b14d86a6fc860cd0f";

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("schema_digest_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

// An empty directory digests to sha256 of nothing
#[test]
fn empty_directory_digest_is_sha256_of_nothing() {
    let dir = scratch("empty");
    assert_eq!(
        schema_digest(&dir),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
}

// Each file is framed by relative path, NUL, byte length, NUL, bytes
#[test]
fn digest_frames_each_file_by_path_and_length() {
    let dir = scratch("frames");
    fs::write(dir.join("a"), "x").unwrap();
    fs::create_dir(dir.join("b")).unwrap();
    fs::write(dir.join("b/c"), "").unwrap();
    assert_eq!(
        schema_digest(&dir),
        "4320b59a30ab5a1ab4baec509a15cc2a8919fd31ec8099fc7807a6cffcf50ac6"
    );
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

// One owner. The needle is built at run time so this file does not match itself.
#[test]
fn schema_digest_has_one_definition() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let needle = format!("fn {}(", "schema_digest");
    let mut files = Vec::new();
    rs_files(&crates, &mut files);
    let hits: Vec<PathBuf> = files
        .into_iter()
        .filter(|f| fs::read_to_string(f).unwrap().contains(&needle))
        .collect();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(
        hits[0].ends_with("tests/support/schema_digest.rs"),
        "{hits:?}"
    );
}

// The goldens are pinned by digest
#[test]
fn canonical_goldens_match_recorded_digest() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/canonical");
    assert_eq!(schema_digest(&dir), RECORDED);
}
