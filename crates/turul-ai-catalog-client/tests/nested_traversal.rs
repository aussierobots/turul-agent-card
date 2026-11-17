//! Nested-catalog traversal contract for `load`: depth limit, cycles, request budget,
//! per-document reports, and what is never fetched.

mod support;

use std::fs;
use std::path::PathBuf;

use support::loopback::{Reply, Server};
use turul_ai_catalog::validate::{
    Code, Conformance, ConformanceLevel, MAX_NESTING_DEPTH, NestingDepth, validate, validate_at,
};
use turul_ai_catalog_client::{
    AcceptError, Child, FetchError, FetchPolicy, LoadError, LoadWarning, ResolvedCatalog,
    SkipReason, Source, fetch, load,
};

const CATALOG: &str = "application/ai-catalog+json";
const A2A: &str = "application/a2a-agent-card+json";

fn policy() -> FetchPolicy {
    let mut p = FetchPolicy::default();
    p.permit_loopback_http = true;
    p
}

fn budget(max_requests: u32) -> FetchPolicy {
    let mut p = policy();
    p.max_requests = max_requests;
    p
}

fn doc(entries: &[String]) -> Vec<u8> {
    format!(
        r#"{{"specVersion":"1.0","entries":[{}]}}"#,
        entries.join(",")
    )
    .into_bytes()
}

fn nested(id: &str, url: &str) -> String {
    format!(r#"{{"identifier":"{id}","type":"{CATALOG}","url":"{url}"}}"#)
}

fn agent(id: &str, url: &str) -> String {
    format!(r#"{{"identifier":"{id}","type":"{A2A}","url":"{url}"}}"#)
}

/// A catalog whose entries are nested-catalog references to `urls`, in order.
fn nesting(urls: &[&str]) -> Vec<u8> {
    let entries: Vec<String> = urls
        .iter()
        .enumerate()
        .map(|(i, u)| nested(&format!("urn:example:catalog:{i}"), u))
        .collect();
    doc(&entries)
}

fn leaf() -> Vec<u8> {
    doc(&[agent(
        "urn:example:agent:leaf",
        "https://h.example/leaf.json",
    )])
}

fn ok(body: &[u8]) -> Reply {
    Reply::ok(CATALOG, body)
}

fn paths(s: &Server) -> Vec<String> {
    s.log().iter().map(|l| l.path().to_string()).collect()
}

fn catalog(tree: &ResolvedCatalog, i: usize) -> &ResolvedCatalog {
    let Child::Catalog(c) = &tree.children[i].child else {
        panic!("child {i} was skipped: {:?}", tree.children[i].child);
    };
    c
}

fn skipped(tree: &ResolvedCatalog, i: usize) -> (&str, &SkipReason) {
    let Child::Skipped { url, reason, .. } = &tree.children[i].child else {
        panic!("child {i} was loaded");
    };
    (url, reason)
}

fn load_url(s: &Server, path: &str, p: &FetchPolicy) -> ResolvedCatalog {
    load(&Source::Url(s.url(path)), p).expect("root loads")
}

fn file(name: &str, bytes: &[u8]) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("nested_traversal_{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

fn budget_warnings(tree: &ResolvedCatalog) -> Vec<&LoadWarning> {
    tree.warnings
        .iter()
        .filter(|w| matches!(w, LoadWarning::RequestBudget { .. }))
        .collect()
}

// #nested-catalog-depth-and-circular-references: reject a URL already fetched in the current path.
#[test]
fn catalog_on_the_current_path_is_skipped_as_cycle() {
    let s = Server::start(vec![ok(&nesting(&["b.json"])), ok(&nesting(&["a.json"]))]);
    let a = load_url(&s, "/a.json", &policy());
    let b = catalog(&a, 0);
    let (url, reason) = skipped(b, 0);
    assert!(matches!(reason, SkipReason::Cycle), "{reason:?}");
    assert_eq!(url, s.url("/a.json"));
    assert_eq!(paths(&s), ["/a.json", "/b.json"]);
}

// A redirect into an ancestor is the same catalog by another name.
#[test]
fn redirect_back_to_an_ancestor_is_a_cycle() {
    let a = nesting(&["c.json"]);
    let s = Server::start(vec![ok(&a), Reply::redirect(302, "/a.json"), ok(&a)]);
    let root = load_url(&s, "/a.json", &policy());
    let (url, reason) = skipped(&root, 0);
    assert!(matches!(reason, SkipReason::Cycle), "{reason:?}");
    assert_eq!(url, s.url("/c.json"));
    assert_eq!(paths(&s), ["/a.json", "/c.json", "/a.json"]);
}

// "current traversal path": two siblings naming one catalog both load.
#[test]
fn sibling_references_to_one_catalog_are_not_cycles() {
    let s = Server::start(vec![
        ok(&nesting(&["b.json", "b.json"])),
        ok(&leaf()),
        ok(&leaf()),
    ]);
    let root = load_url(&s, "/root.json", &policy());
    assert_eq!(root.children.len(), 2);
    catalog(&root, 0);
    catalog(&root, 1);
    assert_eq!(paths(&s), ["/root.json", "/b.json", "/b.json"]);
}

// #resolving-an-artifact-s-display-name and siblings: SHOULD NOT dereference an artifact to render.
#[test]
fn rows_are_resolved_without_fetching_artifacts() {
    let s = Server::silent();
    s.push(ok(&doc(&[agent(
        "urn:example:agent:billing",
        &s.url("/card.json"),
    )])));
    s.push(Reply::ok(
        A2A,
        br#"{"name":"Card Name","description":"Card description","version":"9.9.9"}"#,
    ));
    let root = load_url(&s, "/root.json", &policy());
    assert_eq!(root.rows.len(), 1);
    let row = &root.rows[0];
    assert_eq!(row.index, 0);
    assert_eq!(row.name, "billing");
    assert_eq!(row.description, None);
    assert_eq!(row.version, None);
    assert_eq!(paths(&s), ["/root.json"]);
}

// #safe-fetching: prefer inline evidence; `load` never requests host or evidence URLs.
#[test]
fn logo_and_evidence_urls_are_never_requested() {
    let s = Server::silent();
    let body = format!(
        r#"{{"specVersion":"1.0",
            "host":{{"displayName":"H","logoUrl":"{logo}","documentationUrl":"{docs}"}},
            "entries":[{{"identifier":"urn:example:agent:a","type":"{A2A}","url":"{card}",
              "digest":"sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
              "trustManifests":[{{"contributor":"did:web:h.example",
                "trustSchema":{{"identifier":"urn:example:schema","version":"1","governanceUri":"{gov}"}},
                "attestations":[{{"type":"SLSA-Provenance","uri":"{att}"}}],
                "provenance":[{{"relation":"derivedFrom","sourceId":"{src}",
                  "registryUri":"{reg}","statementUri":"{stmt}"}}]}}]}}]}}"#,
        logo = s.url("/logo.png"),
        docs = s.url("/docs"),
        card = s.url("/card.json"),
        gov = s.url("/gov"),
        att = s.url("/att"),
        src = s.url("/src"),
        reg = s.url("/reg"),
        stmt = s.url("/stmt"),
    );
    s.push(ok(body.as_bytes()));
    let root = load_url(&s, "/root.json", &policy());
    assert!(root.children.is_empty());
    assert_eq!(paths(&s), ["/root.json"]);
}

// #policy-urls-extension: policy documents are never requested by `load`.
#[test]
fn policy_urls_are_never_requested() {
    let s = Server::silent();
    let body = format!(
        r#"{{"specVersion":"1.0","entries":[{{"identifier":"urn:example:agent:a","type":"{A2A}",
            "url":"https://h.example/a.json",
            "extensions":{{"https://ai-catalog.org/extensions/policy-urls":
              {{"privacy":"{privacy}","terms":"{terms}"}}}}}}]}}"#,
        privacy = s.url("/privacy"),
        terms = s.url("/terms"),
    );
    s.push(ok(body.as_bytes()));
    let root = load_url(&s, "/root.json", &policy());
    assert!(root.report.diagnostics().is_empty(), "{:?}", root.report);
    assert_eq!(paths(&s), ["/root.json"]);
}

// #nested-catalog-entries: only `application/ai-catalog+json` entries with `url` are followed.
#[test]
fn only_nested_catalog_entries_are_followed() {
    let s = Server::silent();
    let inline = format!(
        r#"{{"identifier":"urn:example:catalog:inline","type":"{CATALOG}",
            "data":{{"specVersion":"1.0","entries":[{}]}}}}"#,
        agent("urn:example:agent:inner", &s.url("/inner-card.json"))
    );
    let upper = format!(
        r#"{{"identifier":"urn:example:catalog:upper","type":"Application/AI-Catalog+JSON","url":"{}"}}"#,
        s.url("/upper.json")
    );
    s.push(ok(&doc(&[
        agent("urn:example:agent:a", &s.url("/card.json")),
        inline,
        upper,
    ])));
    let root = load_url(&s, "/root.json", &policy());
    assert_eq!(root.rows.len(), 3);
    assert!(root.children.is_empty());
    assert_eq!(paths(&s), ["/root.json"]);
}

// #nested-catalog-entries: each sub-catalog is an independent document with its own report.
#[test]
fn each_fetched_catalog_has_its_own_report() {
    let child = format!(
        r#"{{"specVersion":"1.0","entries":[{},
            {{"identifier":"urn:example:agent:other","type":"{A2A}","url":"https://h.example/b.json",
              "updatedAt":"yesterday"}}]}}"#,
        agent("urn:example:shared", "https://h.example/a.json")
    );
    let s = Server::start(vec![
        ok(&doc(&[nested("urn:example:shared", "child.json")])),
        ok(child.as_bytes()),
    ]);
    let root = load_url(&s, "/root.json", &policy());
    assert_eq!(root.report, validate(&root.document));
    assert!(root.report.diagnostics().is_empty(), "{:?}", root.report);
    assert_eq!(
        root.report.conformance(),
        Conformance::Met(ConformanceLevel::Minimal)
    );

    let c = catalog(&root, 0);
    assert_eq!(c.depth, NestingDepth::ROOT.child().unwrap());
    assert_eq!(c.report, validate_at(&c.document, c.depth));
    let codes: Vec<Code> = c.report.diagnostics().iter().map(|d| d.code).collect();
    assert_eq!(codes, [Code::TimestampInvalid]);
    assert_eq!(c.report.conformance(), Conformance::NotMet);
}

// #nested-catalog-depth-and-circular-references: limit 4, the root is level 1.
#[test]
fn five_level_url_chain_stops_at_level_four() {
    assert_eq!(MAX_NESTING_DEPTH, 4);
    let s = Server::start(vec![
        ok(&nesting(&["l2.json"])),
        ok(&nesting(&["l3.json"])),
        ok(&nesting(&["l4.json"])),
        ok(&nesting(&["l5.json"])),
        ok(&leaf()),
    ]);
    let l1 = load_url(&s, "/l1.json", &policy());
    let l2 = catalog(&l1, 0);
    let l3 = catalog(l2, 0);
    let l4 = catalog(l3, 0);
    let depths: Vec<u8> = [&l1, l2, l3, l4].iter().map(|n| n.depth.get()).collect();
    assert_eq!(depths, [1, 2, 3, 4]);
    assert_eq!(l1.depth, NestingDepth::ROOT);
    let (url, reason) = skipped(l4, 0);
    assert!(matches!(reason, SkipReason::Depth), "{reason:?}");
    assert_eq!(url, s.url("/l5.json"));
    assert_eq!(paths(&s), ["/l1.json", "/l2.json", "/l3.json", "/l4.json"]);
}

// A relative `url` resolves against the final URL of the document that holds it.
#[test]
fn relative_url_resolves_against_the_fetching_document() {
    let s = Server::start(vec![
        ok(&nesting(&["sub/child.json"])),
        ok(&nesting(&["../leaf.json"])),
        ok(&leaf()),
    ]);
    let root = load_url(&s, "/dir/root.json", &policy());
    let child = catalog(&root, 0);
    assert_eq!(
        child.base.as_deref(),
        Some(s.url("/dir/sub/child.json").as_str())
    );
    let leaf = catalog(child, 0);
    assert_eq!(leaf.base.as_deref(), Some(s.url("/dir/leaf.json").as_str()));
    assert_eq!(
        paths(&s),
        ["/dir/root.json", "/dir/sub/child.json", "/dir/leaf.json"]
    );
}

// Inline nesting inside a fetched child is validated from the child's depth.
#[test]
fn inline_nesting_inside_a_fetched_child_counts_from_its_depth() {
    let inline = |data: &str| {
        format!(
            r#"{{"specVersion":"1.0","entries":[{{"identifier":"urn:example:catalog:inline","type":"{CATALOG}","data":{data}}}]}}"#
        )
    };
    let innermost = r#"{"specVersion":"1.0","entries":[]}"#;
    let child = inline(&inline(&inline(innermost)));
    let s = Server::start(vec![ok(&nesting(&["child.json"])), ok(child.as_bytes())]);
    let root = load_url(&s, "/root.json", &policy());
    let c = catalog(&root, 0);
    assert_eq!(c.depth.get(), 2);
    assert_eq!(c.report, validate_at(&c.document, c.depth));
    assert_ne!(c.report, validate(&c.document));
    let limit: Vec<String> = c
        .report
        .diagnostics()
        .iter()
        .filter(|d| d.code == Code::NestedDepthLimit)
        .map(|d| d.path.to_string())
        .collect();
    assert_eq!(limit, ["/entries/0/data/entries/0/data/entries/0/data"]);
    assert_eq!(c.report.conformance(), Conformance::Undetermined);
    assert!(root.children.len() == 1 && c.children.is_empty());
}

// A file root has no base URL, so a relative nested `url` cannot be resolved.
#[test]
fn relative_url_from_a_file_root_is_unresolvable() {
    let path = file("relative.json", &nesting(&["teams/catalog.json"]));
    let root = load(&Source::File(path), &FetchPolicy::default()).expect("loads");
    assert_eq!(root.base, None);
    assert_eq!(root.depth, NestingDepth::ROOT);
    let (url, reason) = skipped(&root, 0);
    assert!(matches!(reason, SkipReason::Unresolvable), "{reason:?}");
    assert_eq!(url, "teams/catalog.json");
}

// `base` is the final URL after redirects, and children resolve against it.
#[test]
fn base_is_the_final_fetched_url() {
    let s = Server::start(vec![
        Reply::redirect(302, "/final/root.json"),
        ok(&nesting(&["child.json"])),
        ok(&leaf()),
    ]);
    let root = load_url(&s, "/start", &policy());
    assert_eq!(
        root.base.as_deref(),
        Some(s.url("/final/root.json").as_str())
    );
    let child = catalog(&root, 0);
    assert_eq!(
        child.base.as_deref(),
        Some(s.url("/final/child.json").as_str())
    );
    assert_eq!(
        paths(&s),
        ["/start", "/final/root.json", "/final/child.json"]
    );
}

// A child failure is a skip: no error, no parent diagnostic, siblings continue.
#[test]
fn failed_child_is_skipped_and_siblings_continue() {
    let s = Server::start(vec![
        ok(&nesting(&["missing.json", "ok.json"])),
        Reply::new(404)
            .header("Content-Type", "text/plain")
            .body(b"nope"),
        ok(&leaf()),
    ]);
    let root = load_url(&s, "/root.json", &policy());
    let (url, reason) = skipped(&root, 0);
    assert_eq!(url, s.url("/missing.json"));
    assert!(
        matches!(
            reason,
            SkipReason::Fetch(LoadError::Accept(AcceptError::Status(404)))
        ),
        "{reason:?}"
    );
    catalog(&root, 1);
    assert_eq!(root.report, validate(&root.document));
    assert!(root.report.diagnostics().is_empty());
}

// `url` entries inside an inline nested catalog are validated, not followed.
#[test]
fn url_entries_inside_inline_data_are_not_followed() {
    let s = Server::silent();
    let inline = format!(
        r#"{{"identifier":"urn:example:catalog:inline","type":"{CATALOG}",
            "data":{{"specVersion":"1.0","entries":[{}]}}}}"#,
        nested("urn:example:catalog:inner", &s.url("/inner.json"))
    );
    s.push(ok(&doc(&[inline])));
    s.push(ok(&leaf()));
    let root = load_url(&s, "/root.json", &policy());
    assert!(root.children.is_empty());
    assert_eq!(paths(&s), ["/root.json"]);
}

// Depth-first, entry order, one request at a time.
#[test]
fn traversal_order_is_depth_first_in_entry_order() {
    let s = Server::start(vec![
        ok(&nesting(&["a.json", "b.json"])),
        ok(&nesting(&["a1.json"])),
        ok(&leaf()),
        ok(&leaf()),
    ]);
    let root = load_url(&s, "/root.json", &policy());
    let entries: Vec<usize> = root.children.iter().map(|n| n.entry).collect();
    assert_eq!(entries, [0, 1]);
    assert_eq!(paths(&s), ["/root.json", "/a.json", "/a1.json", "/b.json"]);
}

#[test]
fn default_request_budget_is_256() {
    assert_eq!(FetchPolicy::default().max_requests, 256);
}

/// Root with five nested entries: `c1` plain, `c2` redirecting once to `c2b`, then `c3`..`c5`.
fn budget_replies(root_type: &str) -> Vec<Reply> {
    vec![
        Reply::ok(
            root_type,
            &nesting(&["c1.json", "c2.json", "c3.json", "c4.json", "c5.json"]),
        ),
        ok(&leaf()),
        Reply::redirect(302, "/c2b.json"),
        ok(&leaf()),
        ok(&leaf()),
        ok(&leaf()),
        ok(&leaf()),
    ]
}

// Every request sent counts, redirect hops included; then every further child is skipped.
#[test]
fn request_budget_counts_every_request_redirects_included() {
    let s = Server::start(budget_replies(CATALOG));
    let root = load_url(&s, "/root.json", &budget(4));
    catalog(&root, 0);
    assert_eq!(
        catalog(&root, 1).base.as_deref(),
        Some(s.url("/c2b.json").as_str())
    );
    for (i, name) in [(2, "/c3.json"), (3, "/c4.json"), (4, "/c5.json")] {
        let (url, reason) = skipped(&root, i);
        assert!(matches!(reason, SkipReason::Budget), "{i}: {reason:?}");
        assert_eq!(url, s.url(name));
    }
    assert_eq!(
        paths(&s),
        ["/root.json", "/c1.json", "/c2.json", "/c2b.json"]
    );
}

// Once spent, the budget skips a child before the depth and cycle checks.
#[test]
fn spent_budget_skips_every_further_child() {
    let s = Server::start(vec![
        ok(&nesting(&["l2.json", "x.json"])),
        ok(&nesting(&["l3.json"])),
        ok(&nesting(&["l4.json"])),
        ok(&nesting(&["l5.json", "l1.json"])),
    ]);
    let l1 = load_url(&s, "/l1.json", &budget(4));
    let l4 = catalog(catalog(catalog(&l1, 0), 0), 0);
    for i in [0, 1] {
        let (_, reason) = skipped(l4, i);
        assert!(matches!(reason, SkipReason::Budget), "{i}: {reason:?}");
    }
    let (_, reason) = skipped(&l1, 1);
    assert!(matches!(reason, SkipReason::Budget), "{reason:?}");
    assert_eq!(s.log().len(), 4);
    assert!(matches!(
        budget_warnings(&l1)[..],
        [LoadWarning::RequestBudget {
            limit: 4,
            skipped: 3,
            ..
        }]
    ));
}

// One warning on the root, last, naming the limit and the skipped count; reports unchanged.
#[test]
fn one_budget_warning_names_limit_and_skipped_count() {
    let s = Server::start(budget_replies("application/json"));
    let root = load_url(&s, "/root.json", &budget(4));
    let w = root.warnings.last().expect("a warning");
    assert!(
        matches!(
            w,
            LoadWarning::RequestBudget {
                limit: 4,
                skipped: 3,
                ..
            }
        ),
        "{w:?}"
    );
    assert_eq!(budget_warnings(&root).len(), 1);
    assert_eq!(root.warnings.first(), Some(&LoadWarning::MediaTypeGeneric));
    assert_eq!(w.code(), "request-budget");
    let text = w.to_string();
    assert!(text.contains('4') && text.contains('3'), "{text}");
    for i in [0, 1] {
        assert!(budget_warnings(catalog(&root, i)).is_empty());
    }

    let u = Server::start(budget_replies("application/json"));
    let full = load_url(&u, "/root.json", &policy());
    assert_eq!(full.warnings, [LoadWarning::MediaTypeGeneric]);
    assert_eq!(root.report, full.report);
    for i in [0, 1] {
        assert_eq!(catalog(&root, i).report, catalog(&full, i).report);
    }
}

// A budget spent inside a child's redirect chain skips that child.
#[test]
fn budget_spent_inside_a_redirect_chain_skips_that_child() {
    let s = Server::start(vec![
        ok(&nesting(&["r.json"])),
        Reply::redirect(302, "/target.json"),
        ok(&leaf()),
    ]);
    let root = load_url(&s, "/root.json", &budget(2));
    let (url, reason) = skipped(&root, 0);
    assert!(matches!(reason, SkipReason::Budget), "{reason:?}");
    assert_eq!(url, s.url("/r.json"));
    assert_eq!(s.log().len(), 2);
}

// The root counts against the budget; a root that cannot be fetched within it is an error.
#[test]
fn root_beyond_the_budget_is_a_load_error() {
    let s = Server::start(vec![ok(&leaf())]);
    let r = load(&Source::Url(s.url("/root.json")), &budget(0));
    assert!(
        matches!(r, Err(LoadError::Budget { limit: 0, .. })),
        "{r:?}"
    );
    assert!(s.log().is_empty());

    let s = Server::start(vec![Reply::redirect(302, "/final.json"), ok(&leaf())]);
    let r = load(&Source::Url(s.url("/root.json")), &budget(1));
    assert!(
        matches!(r, Err(LoadError::Budget { limit: 1, .. })),
        "{r:?}"
    );
    assert_eq!(s.log().len(), 1);
}

// A hop refused by scheme or address vetting sends nothing and costs nothing.
#[test]
fn refused_hops_cost_no_budget() {
    let s = Server::start(vec![
        ok(&nesting(&["http://169.254.169.254/", "ok.json"])),
        ok(&leaf()),
    ]);
    let root = load_url(&s, "/root.json", &budget(2));
    let (url, reason) = skipped(&root, 0);
    assert_eq!(url, "http://169.254.169.254/");
    assert!(
        matches!(
            reason,
            SkipReason::Fetch(LoadError::Fetch(FetchError::DisallowedAddress(_)))
        ),
        "{reason:?}"
    );
    catalog(&root, 1);
    assert!(budget_warnings(&root).is_empty());
    assert_eq!(paths(&s), ["/root.json", "/ok.json"]);
}

// A file root sends no request, so the whole budget goes to its children.
#[test]
fn file_root_costs_no_request() {
    let s = Server::start(vec![ok(&leaf())]);
    let path = file("absolute.json", &nesting(&[s.url("/child.json").as_str()]));
    let root = load(&Source::File(path), &budget(1)).expect("loads");
    catalog(&root, 0);
    assert!(budget_warnings(&root).is_empty());
    assert_eq!(paths(&s), ["/child.json"]);
}

// No warning unless some child is skipped for the budget, even when the budget is spent exactly.
#[test]
fn no_budget_warning_without_a_budget_skip() {
    let s = Server::start(vec![ok(&nesting(&["leaf.json"])), ok(&leaf())]);
    let root = load_url(&s, "/root.json", &budget(2));
    catalog(&root, 0);
    assert!(root.warnings.is_empty(), "{:?}", root.warnings);

    let s = Server::start(vec![ok(&nesting(&["b.json"])), ok(&nesting(&["a.json"]))]);
    let a = load_url(&s, "/a.json", &policy());
    assert!(matches!(skipped(catalog(&a, 0), 0).1, SkipReason::Cycle));
    assert!(a.warnings.is_empty(), "{:?}", a.warnings);
}

// `max_requests` is read only by `load`; a single `fetch` is bounded by its redirect limit.
#[test]
fn fetch_alone_ignores_the_request_budget() {
    let s = Server::start(vec![ok(&leaf())]);
    let f = fetch(&s.url("/x.json"), None, &budget(0)).expect("fetches");
    assert_eq!(f.status, 200);
    assert_eq!(s.log().len(), 1);
}
