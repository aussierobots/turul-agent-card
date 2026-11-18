//! The crypto boundary: which crates the trust crate may depend on, and what its sources may do.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const ALPHABET: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

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

fn sources(crate_dir: &Path) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    rs_files(&crate_dir.join("src"), &mut files);
    files
        .into_iter()
        .map(|f| {
            let text = fs::read_to_string(&f).unwrap();
            (f, text)
        })
        .collect()
}

fn trust_src() -> Vec<(PathBuf, String)> {
    sources(Path::new(env!("CARGO_MANIFEST_DIR")))
}

fn format_src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("turul-ai-catalog")
}

#[test]
fn direct_dependencies_are_only_the_named_crates() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = Command::new(cargo)
        .args([
            "tree",
            "-e",
            "normal",
            "-p",
            "turul-ai-catalog-trust",
            "--depth",
            "1",
            "--prefix",
            "none",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut names: Vec<String> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .skip(1)
        .filter_map(|l| l.split_whitespace().next().map(str::to_string))
        .collect();
    names.sort();
    names.dedup();
    assert_eq!(
        names,
        [
            "p256",
            "rand_core",
            "sha2",
            "turul-ai-catalog",
            "turul-ai-catalog-client",
            "turul-jwt-validator"
        ]
    );
}

#[test]
fn validator_features_exclude_jwks() {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = Command::new(cargo)
        .args([
            "tree",
            "-e",
            "normal",
            "-p",
            "turul-ai-catalog-trust",
            "--prefix",
            "none",
            "--format",
            "{p}|{f}",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8(out.stdout).unwrap();
    let features: Vec<&str> = text
        .lines()
        .filter(|l| l.starts_with("turul-jwt-validator "))
        .filter_map(|l| l.split_once('|').map(|(_, f)| f))
        .collect();
    assert!(!features.is_empty(), "validator not in the tree");
    for f in features {
        let names: Vec<&str> = f.split(',').collect();
        assert!(names.contains(&"detached_rust_crypto"), "{f}");
        for banned in ["jwks", "aws_lc_rs", "rust_crypto"] {
            assert!(!names.contains(&banned), "{banned} in {f}");
        }
    }
}

#[test]
fn trust_src_has_no_network_access_but_client_fetch() {
    for (file, text) in trust_src() {
        for banned in ["std::net", "reqwest", "TcpStream", "UdpSocket"] {
            assert!(!text.contains(banned), "{banned} in {}", file.display());
        }
    }
}

#[test]
fn no_base64_or_jws_splitting_in_trust_src() {
    let splits = [format!("split({})", "'.'"), format!("split({})", "\".\"")];
    for (file, text) in trust_src() {
        assert!(!text.contains(ALPHABET), "alphabet in {}", file.display());
        for s in &splits {
            assert!(!text.contains(s.as_str()), "{s} in {}", file.display());
        }
    }
}

#[test]
fn format_crate_has_no_jws_or_base64url() {
    let dir = format_src_dir();
    for (file, text) in sources(&dir) {
        let stem = file.file_stem().unwrap().to_str().unwrap().to_string();
        assert!(stem != "jws" && stem != "base64url", "{}", file.display());
        assert!(!text.contains(ALPHABET), "alphabet in {}", file.display());
    }
}
