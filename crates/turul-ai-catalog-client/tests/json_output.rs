//! `ResolvedCatalog::to_json` contract: columnar canonical output with legends, pinned by
//! hand-authored goldens, carrying no trust verdict.

mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use support::loopback::{Reply, Server};
use turul_ai_catalog::json::{Json, read};
use turul_ai_catalog::model::AiCatalog;
use turul_ai_catalog::parse::parse;
use turul_ai_catalog_client::{FetchPolicy, Source, load};

const CATALOG: &str = "application/ai-catalog+json";

const MEMBER_NAMES: [&str; 20] = [
    "legend",
    "code",
    "severity",
    "type",
    "root",
    "source",
    "conformance",
    "entries",
    "index",
    "identifier",
    "version",
    "name",
    "description",
    "url",
    "diagnostics",
    "path",
    "children",
    "entry",
    "catalog",
    "skipped",
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/json_output")
}

fn fixture(name: &str) -> Vec<u8> {
    fs::read(fixtures().join(name)).unwrap()
}

fn output(source: &Source, policy: &FetchPolicy) -> Vec<u8> {
    load(source, policy)
        .expect("loads")
        .to_json()
        .to_canonical_vec()
        .expect("canonical")
}

fn file_root_output() -> Vec<u8> {
    output(
        &Source::File(fixtures().join("file-root.catalog.json")),
        &FetchPolicy::default(),
    )
}

fn names(v: &Json, out: &mut BTreeSet<String>) {
    match v {
        Json::Object(m) => {
            for (k, x) in m.iter() {
                out.insert(k.clone());
                names(x, out);
            }
        }
        Json::Array(items) => items.iter().for_each(|x| names(x, out)),
        _ => {}
    }
}

fn member<'a>(v: &'a Json, name: &str) -> &'a Json {
    match v {
        Json::Object(m) => m.get(name).unwrap_or_else(|| panic!("no {name}")),
        other => panic!("{other:?} is not an object"),
    }
}

#[test]
fn file_root_output_equals_golden() {
    assert_eq!(
        String::from_utf8(file_root_output()).unwrap(),
        String::from_utf8(fixture("file-root.jcs")).unwrap()
    );
}

#[test]
fn loopback_tree_output_equals_golden() {
    let s = Server::start(vec![
        Reply::ok("application/json", &fixture("loopback-tree/root.json")),
        Reply::ok(CATALOG, &fixture("loopback-tree/child.json")),
        Reply::new(404)
            .header("Content-Type", "text/plain")
            .body(&fixture("loopback-tree/missing")),
    ]);
    let mut p = FetchPolicy::default();
    p.permit_loopback_http = true;
    p.max_requests = 3;
    let got = output(&Source::Url(s.url("/root.json")), &p);
    let want = String::from_utf8(fixture("loopback-tree.jcs"))
        .unwrap()
        .replace("http://loopback.test", &s.url(""));
    assert_eq!(String::from_utf8(got).unwrap(), want);
    let paths: Vec<String> = s.log().iter().map(|l| l.path().to_string()).collect();
    assert_eq!(paths, ["/root.json", "/child.json", "/missing"]);
}

// Two loads give equal bytes, and the bytes are already canonical.
#[test]
fn output_is_deterministic() {
    let a = file_root_output();
    assert_eq!(a, file_root_output());
    assert_eq!(read(&a).unwrap().to_canonical_vec().unwrap(), a);
}

// #trust-layers: without Layer 2, Trust Manifest content is advisory; the output holds none.
#[test]
fn output_carries_no_trust_verdict() {
    let allowed: BTreeSet<String> = MEMBER_NAMES.iter().map(|s| s.to_string()).collect();
    let mut union = BTreeSet::new();
    for golden in ["file-root.jcs", "loopback-tree.jcs"] {
        let mut found = BTreeSet::new();
        names(&read(&fixture(golden)).unwrap(), &mut found);
        let extra: Vec<&String> = found.difference(&allowed).collect();
        assert!(extra.is_empty(), "{golden}: {extra:?}");
        union.extend(found);
    }
    assert_eq!(union, allowed);

    let document: AiCatalog = parse(&fixture("file-root.catalog.json")).unwrap();
    let manifests: Vec<_> = document
        .entries
        .iter()
        .flat_map(|e| e.trust_manifests.iter().flatten())
        .collect();
    assert!(!manifests.is_empty());
    let bytes = String::from_utf8(file_root_output()).unwrap();
    for m in manifests {
        let sig = m.signature.as_ref().expect("signed");
        for value in [&m.contributor, &sig.signer, &sig.jws] {
            assert!(!bytes.contains(value.as_str()), "{value}");
        }
    }
}

// JSON output carries the original strings; JCS escapes every control character.
#[test]
fn control_characters_are_escaped_not_replaced() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("json_output_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("controls.json");
    fs::write(
        &path,
        r#"{"specVersion":"1.0","entries":[{"identifier":"urn:example:agent:esc",
            "displayName":"\u001b[2J","description":"bell\u0007",
            "type":"application/a2a-agent-card+json","url":"https://h.example/a.json"}]}"#,
    )
    .unwrap();
    let bytes = output(&Source::File(path), &FetchPolicy::default());
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains(r#""\u001b[2J""#), "{text}");
    assert!(text.contains(r#""bell\u0007""#), "{text}");
    assert!(!bytes.iter().any(|&b| b < 0x20 || b == 0x7f));
    assert!(!text.contains('\u{FFFD}'));

    let value = read(&bytes).unwrap();
    let entries = member(member(&value, "root"), "entries");
    let Json::Array(name) = member(entries, "name") else {
        panic!("name is not an array");
    };
    assert_eq!(name, &[Json::String("\u{1b}[2J".into())]);
}
