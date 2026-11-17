//! The one definition of the schema-digest algorithm: lowercase-hex sha256 over, for every
//! regular file under `dir` in ascending byte order of its `/`-separated relative path, the
//! path, NUL, the file's byte length in decimal ASCII, NUL, and the file's bytes.

use std::fs;
use std::path::Path;

use sha2::{Digest, Sha256};

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let entry = entry.unwrap();
        let path = entry.path();
        let kind = entry.file_type().unwrap();
        if kind.is_dir() {
            collect(root, &path, out);
        } else if kind.is_file() {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .components()
                .map(|c| c.as_os_str().to_str().expect("UTF-8 path").to_string())
                .collect::<Vec<_>>()
                .join("/");
            out.push((rel, fs::read(&path).unwrap()));
        } else {
            panic!(
                "{} is neither a regular file nor a directory",
                path.display()
            );
        }
    }
}

pub fn schema_digest(dir: &Path) -> String {
    let mut files = Vec::new();
    collect(dir, dir, &mut files);
    files.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));
    let mut hasher = Sha256::new();
    for (path, bytes) in &files {
        hasher.update(path.as_bytes());
        hasher.update([0u8]);
        hasher.update(bytes.len().to_string().as_bytes());
        hasher.update([0u8]);
        hasher.update(bytes);
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}
