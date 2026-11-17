//! The recorded schema digest of the signed-payload goldens, computed by the one owner of the
//! schema-digest algorithm in the format crate.

use std::fs;
use std::path::{Path, PathBuf};

#[path = "../../turul-ai-catalog/tests/support/schema_digest.rs"]
mod owner;
use owner::schema_digest;

/// Recorded digest of `tests/fixtures/signed_payload/`.
const RECORDED: &str = "eebddd1ef08d76185c6de695dbf491e024a33509469b16c59bcc0c3497822e35";

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
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let needle = format!("fn {}(", "schema_digest");
    let mut files = Vec::new();
    rs_files(crates, &mut files);
    let hits: Vec<PathBuf> = files
        .into_iter()
        .filter(|f| fs::read_to_string(f).unwrap().contains(&needle))
        .collect();
    assert_eq!(hits.len(), 1, "{hits:?}");
    assert!(
        hits[0].ends_with("turul-ai-catalog/tests/support/schema_digest.rs"),
        "{hits:?}"
    );
}

#[test]
fn signed_payload_goldens_match_recorded_digest() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/signed_payload");
    assert_eq!(schema_digest(&dir), RECORDED);
}
