//! `Filter` contract: a conjunction of exact criteria that prunes rows and nothing else.

mod support;

use std::fs;
use std::path::PathBuf;

use support::loopback::{Reply, Server};
use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
use turul_ai_catalog_client::{Child, FetchPolicy, Filter, ResolvedCatalog, Source, load};

const CATALOG: &str = "application/ai-catalog+json";

/// Rows: 0 `urn:a` 2.0 [billing] A2A; 1 `urn:a` 2.0.0 A2A; 2 `urn:b` (no version)
/// [billing, support] MCP; 3 `urn:c` "" A2A.
const ENTRIES: &str = r#"{"specVersion":"1.0","entries":[
  {"identifier":"urn:a","version":"2.0","tags":["billing"],
   "type":"application/a2a-agent-card+json","url":"https://h.example/a-2.0.json"},
  {"identifier":"urn:a","version":"2.0.0",
   "type":"application/a2a-agent-card+json","url":"https://h.example/a-2.0.0.json"},
  {"identifier":"urn:b","tags":["billing","support"],
   "type":"application/mcp-server-card+json","url":"https://h.example/b.json"},
  {"identifier":"urn:c","version":"",
   "type":"application/a2a-agent-card+json","url":"https://h.example/c.json"}
]}"#;

fn file_root() -> ResolvedCatalog {
    let dir =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("filter_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    // One file per test thread: tests run in parallel.
    let name = std::thread::current()
        .name()
        .unwrap_or("main")
        .replace("::", "_");
    let path = dir.join(format!("{name}.json"));
    fs::write(&path, ENTRIES).unwrap();
    load(&Source::File(path), &FetchPolicy::default()).expect("loads")
}

fn kept(tree: &ResolvedCatalog, f: &Filter) -> Vec<usize> {
    tree.filter(f).rows.iter().map(|r| r.index).collect()
}

fn by(
    identifier: Option<&str>,
    version: Option<&str>,
    artifact_type: Option<&str>,
    tag: Option<&str>,
) -> Filter {
    let mut f = Filter::default();
    f.identifier = identifier.map(EntryIdentifier::new);
    f.version = version.map(String::from);
    f.artifact_type = artifact_type.map(ArtifactType::new);
    f.tag = tag.map(String::from);
    f
}

// #multi-version-entries: a client needing one version matches `identifier` and `version`.
#[test]
fn identifier_and_version_match_exactly() {
    let t = file_root();
    assert_eq!(kept(&t, &by(Some("urn:a"), Some("2.0"), None, None)), [0]);
    assert_eq!(kept(&t, &by(Some("urn:a"), Some("2.0.0"), None, None)), [1]);
    assert_eq!(kept(&t, &by(None, Some("2.0"), None, None)), [0]);
    assert_eq!(kept(&t, &by(Some("urn:a"), None, None, None)), [0, 1]);
    assert!(kept(&t, &by(Some("urn:b"), Some("2.0"), None, None)).is_empty());
    assert_eq!(kept(&t, &by(None, Some(""), None, None)), [3]);
    assert!(kept(&t, &by(Some("URN:A"), None, None, None)).is_empty());
}

// #catalog-entry: `tags` for filtering; #design-goals: `type` to select entries.
#[test]
fn criteria_are_conjunctive() {
    let t = file_root();
    let a2a = Some("application/a2a-agent-card+json");
    assert_eq!(kept(&t, &by(None, None, None, Some("billing"))), [0, 2]);
    assert_eq!(kept(&t, &by(None, None, a2a, Some("billing"))), [0]);
    assert_eq!(
        kept(
            &t,
            &by(None, None, Some("application/mcp-server-card+json"), None)
        ),
        [2]
    );
    assert!(kept(&t, &by(Some("urn:a"), None, None, Some("support"))).is_empty());
    assert!(kept(&t, &by(None, None, None, Some("Billing"))).is_empty());
    assert!(
        kept(
            &t,
            &by(None, None, Some("Application/A2A-Agent-Card+JSON"), None)
        )
        .is_empty()
    );
}

fn assert_same_except_rows(a: &ResolvedCatalog, b: &ResolvedCatalog) {
    assert_eq!(a.document, b.document);
    assert_eq!(a.report, b.report);
    assert_eq!(a.base, b.base);
    assert_eq!(a.depth, b.depth);
    assert_eq!(a.warnings, b.warnings);
    assert_eq!(a.children.len(), b.children.len());
    for (x, y) in a.children.iter().zip(&b.children) {
        assert_eq!(x.entry, y.entry);
        match (&x.child, &y.child) {
            (Child::Catalog(p), Child::Catalog(q)) => assert_same_except_rows(p, q),
            (Child::Skipped { url: p, .. }, Child::Skipped { url: q, .. }) => assert_eq!(p, q),
            other => panic!("child kinds differ: {other:?}"),
        }
    }
}

fn rows(t: &ResolvedCatalog) -> Vec<(usize, String, Option<String>, Option<String>)> {
    t.rows
        .iter()
        .map(|r| {
            (
                r.index,
                r.name.clone(),
                r.description.clone(),
                r.version.clone(),
            )
        })
        .collect()
}

// Each node is filtered in its own scope; documents, reports, warnings and children stay.
#[test]
fn filter_prunes_rows_only() {
    let root = format!(
        r#"{{"specVersion":"1.0","entries":[
          {{"identifier":"urn:child","type":"{CATALOG}","url":"child.json"}},
          {{"identifier":"urn:x","type":"application/a2a-agent-card+json","url":"https://h.example/x.json"}},
          {{"identifier":"urn:gone","type":"{CATALOG}","url":"gone.json"}}]}}"#
    );
    let child = r#"{"specVersion":"1.0","entries":[
      {"identifier":"urn:x","type":"application/a2a-agent-card+json","url":"https://h.example/cx.json"},
      {"identifier":"urn:y","type":"application/a2a-agent-card+json","url":"https://h.example/cy.json"}]}"#;
    let s = Server::start(vec![
        Reply::ok("application/json", root.as_bytes()),
        Reply::ok(CATALOG, child.as_bytes()),
        Reply::new(404),
    ]);
    let mut p = FetchPolicy::default();
    p.permit_loopback_http = true;
    let tree = load(&Source::Url(s.url("/root.json")), &p).expect("loads");

    let f = by(Some("urn:x"), None, None, None);
    let out = tree.filter(&f);
    assert_same_except_rows(&out, &tree);
    assert_eq!(kept(&tree, &f), [1]);
    let Child::Catalog(c) = &out.children[0].child else {
        panic!("child was skipped");
    };
    assert_eq!(c.rows.iter().map(|r| r.index).collect::<Vec<_>>(), [0]);
    assert!(matches!(out.children[1].child, Child::Skipped { .. }));
    // Filtering does not change the tree it is called on.
    assert_eq!(tree.rows.len(), 3);
}

#[test]
fn default_filter_keeps_everything() {
    let t = file_root();
    let out = t.filter(&Filter::default());
    assert_same_except_rows(&out, &t);
    assert_eq!(rows(&out), rows(&t));
    assert_eq!(kept(&t, &Filter::default()), [0, 1, 2, 3]);
}
