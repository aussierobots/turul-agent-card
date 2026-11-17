//! Safe Fetching contract for `fetch`, `FetchPolicy` and `resolve_reference`.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use support::loopback::{Length, Reply, Server};
use turul_ai_catalog::parse::ParseErrorKind;
use turul_ai_catalog_client::{
    AcceptError, FetchError, FetchPolicy, Fetched, LoadError, Source, fetch, load,
    resolve_reference,
};

const CATALOG: &str = "application/ai-catalog+json";

fn loopback_policy() -> FetchPolicy {
    let mut p = FetchPolicy::default();
    p.permit_loopback_http = true;
    p
}

// #safe-fetching: consumer-defined ceiling; defaults documented.
#[test]
fn default_policy_limits() {
    let p = FetchPolicy::default();
    assert_eq!(p.max_body_bytes, 4 * 1024 * 1024);
    assert_eq!(p.timeout, Duration::from_secs(10));
    assert_eq!(p.max_redirects, 5);
    assert!(!p.permit_loopback_http);
}

// #safe-fetching: reject private, loopback, link-local and cloud metadata addresses.
#[test]
fn disallowed_literal_addresses_are_refused_before_connecting() {
    let urls = [
        "https://127.0.0.1/",
        "https://10.0.0.1/",
        "https://[::1]/",
        "https://169.254.169.254/",
        "https://[fc00::1]/",
        "https://[::ffff:127.0.0.1]/",
        "https://2130706433/",
        "https://0x7f.0.0.1/",
        "https://0.0.0.0/",
        "https://[::]/",
        "https://100.100.100.200/",
        "https://192.0.0.192/",
        "https://[fd00:ec2::254]/",
    ];
    for u in urls {
        let r = fetch(u, None, &FetchPolicy::default());
        assert!(
            matches!(r, Err(FetchError::DisallowedAddress(_))),
            "{u} gave {r:?}"
        );
    }
    // WHATWG normalisation: the integer host is 127.0.0.1.
    match fetch("https://2130706433/", None, &FetchPolicy::default()) {
        Err(FetchError::DisallowedAddress(ip)) => assert_eq!(ip.to_string(), "127.0.0.1"),
        other => panic!("{other:?}"),
    }
}

// #safe-fetching: a name is refused by its resolved address.
#[test]
fn hostname_resolving_to_loopback_is_refused() {
    let r = fetch("https://localhost/", None, &FetchPolicy::default());
    assert!(matches!(r, Err(FetchError::DisallowedAddress(_))), "{r:?}");
}

// #safe-fetching: re-check the resolved address after any redirect.
#[test]
fn redirect_to_a_disallowed_address_is_refused() {
    let s = Server::start(vec![Reply::redirect(
        302,
        "https://169.254.169.254/latest/meta-data/",
    )]);
    let r = fetch(&s.url("/start"), None, &loopback_policy());
    assert!(matches!(r, Err(FetchError::DisallowedAddress(_))), "{r:?}");
    assert_eq!(s.log().len(), 1);
}

// #safe-fetching: restrict fetches to https.
#[test]
fn non_https_schemes_are_refused() {
    for u in [
        "http://93.184.216.34/",
        "data:application/json,{}",
        "file:///etc/passwd",
        "ftp://93.184.216.34/",
    ] {
        let r = fetch(u, None, &FetchPolicy::default());
        assert!(matches!(r, Err(FetchError::Scheme)), "{u} gave {r:?}");
    }
    let r = fetch("not a url", None, &FetchPolicy::default());
    assert!(matches!(r, Err(FetchError::InvalidUrl)), "{r:?}");
}

// #safe-fetching: redirects may not leave https, even with loopback permitted.
#[test]
fn redirect_to_http_is_refused() {
    let s = Server::start(vec![Reply::redirect(302, "http://93.184.216.34/")]);
    let r = fetch(&s.url("/start"), None, &loopback_policy());
    assert!(matches!(r, Err(FetchError::Scheme)), "{r:?}");
}

// loopback http needs the switch.
#[test]
fn loopback_http_is_refused_by_default() {
    let s = Server::start(vec![Reply::ok(CATALOG, b"{}")]);
    let r = fetch(&s.url("/"), None, &FetchPolicy::default());
    assert!(r.is_err(), "{r:?}");
    assert!(s.log().is_empty(), "no connection may be made");
}

#[test]
fn loopback_http_is_permitted_only_by_the_switch() {
    let s = Server::start(vec![Reply::ok(CATALOG, b"{}")]);
    let f = fetch(&s.url("/"), None, &loopback_policy()).expect("served");
    assert_eq!(f.status, 200);
    assert_eq!(f.body, b"{}");
    // The switch does not open plain http to a non-loopback address.
    let r = fetch("http://93.184.216.34/", None, &loopback_policy());
    assert!(matches!(r, Err(FetchError::Scheme)), "{r:?}");
}

// #safe-fetching: maximum response size.
#[test]
fn body_at_the_ceiling_is_accepted() {
    let mut p = loopback_policy();
    p.max_body_bytes = 1024;
    for length in [Length::Exact, Length::Omit] {
        let s = Server::start(vec![Reply::ok(CATALOG, &[b'a'; 1024]).length(length)]);
        let f = fetch(&s.url("/"), None, &p).expect("at ceiling");
        assert_eq!(f.body.len(), 1024);
    }
}

#[test]
fn body_over_the_ceiling_is_refused() {
    let mut p = loopback_policy();
    p.max_body_bytes = 1024;
    for length in [Length::Exact, Length::Omit] {
        let s = Server::start(vec![Reply::ok(CATALOG, &[b'a'; 1025]).length(length)]);
        let r = fetch(&s.url("/"), None, &p);
        assert!(matches!(r, Err(FetchError::TooLarge)), "{r:?}");
    }
    // An announced length above the ceiling is refused whatever follows.
    let s = Server::start(vec![
        Reply::ok(CATALOG, b"x").length(Length::Announce(5000)),
    ]);
    let r = fetch(&s.url("/"), None, &p);
    assert!(matches!(r, Err(FetchError::TooLarge)), "{r:?}");
}

// #safe-fetching: request timeout.
#[test]
fn stalled_response_times_out() {
    let mut p = loopback_policy();
    p.timeout = Duration::from_millis(500);
    let s = Server::start(vec![
        Reply::ok(CATALOG, b"{}")
            .length(Length::Announce(2))
            .stall_after_head(Duration::from_secs(5)),
    ]);
    let t = Instant::now();
    let r = fetch(&s.url("/"), None, &p);
    assert!(matches!(r, Err(FetchError::Timeout)), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(3));
}

// #safe-fetching: the request timeout covers the body, however slowly it arrives.
#[test]
fn trickling_body_times_out() {
    let mut p = loopback_policy();
    p.timeout = Duration::from_secs(1);
    let s = Server::start(vec![
        Reply::ok(CATALOG, &[b' '; 20]).drip(Duration::from_millis(300)),
    ]);
    let t = Instant::now();
    let r = fetch(&s.url("/"), None, &p);
    assert!(matches!(r, Err(FetchError::Timeout)), "{r:?}");
    assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
}

// #safe-fetching: one deadline for the whole call.
#[test]
fn deadline_spans_redirect_hops() {
    let mut p = loopback_policy();
    p.timeout = Duration::from_secs(1);
    let d = Duration::from_millis(400);
    let s = Server::start(vec![
        Reply::redirect(302, "/b").delay(d),
        Reply::redirect(302, "/c").delay(d),
        Reply::ok(CATALOG, b"{}").delay(d),
    ]);
    let r = fetch(&s.url("/a"), None, &p);
    assert!(matches!(r, Err(FetchError::Timeout)), "{r:?}");
}

// #safe-fetching: bounded redirects.
#[test]
fn sixth_redirect_is_refused() {
    let p = loopback_policy();
    assert_eq!(p.max_redirects, 5);
    let s = Server::start((0..8).map(|_| Reply::redirect(302, "/n")).collect());
    let r = fetch(&s.url("/n"), None, &p);
    assert!(matches!(r, Err(FetchError::TooManyRedirects)), "{r:?}");
    assert_eq!(s.log().len(), 6, "the original request plus five redirects");

    let mut replies: Vec<Reply> = (0..5).map(|_| Reply::redirect(302, "/n")).collect();
    replies.push(Reply::ok(CATALOG, b"{}"));
    let s = Server::start(replies);
    assert_eq!(
        fetch(&s.url("/n"), None, &p)
            .expect("five redirects")
            .status,
        200
    );
}

// #safe-fetching: every fetched document is untrusted input.
#[test]
fn fetched_documents_are_untrusted_input() {
    let deep = format!("{}{}", "[".repeat(129), "]".repeat(129));
    let cases: Vec<(Vec<u8>, ParseErrorKind)> = vec![
        (deep.into_bytes(), ParseErrorKind::DepthLimit),
        (
            vec![b'{', b'"', 0xff, b'"', b':', b'1', b'}'],
            ParseErrorKind::InvalidUtf8,
        ),
        (
            br#"{"specVersion":"1.0","specVersion":"1.0"}"#.to_vec(),
            ParseErrorKind::DuplicateMember,
        ),
    ];
    for (body, want) in cases {
        let s = Server::start(vec![Reply::ok(CATALOG, &body)]);
        let r = load(&Source::Url(s.url("/c")), &loopback_policy());
        match r {
            Err(LoadError::Accept(AcceptError::Parse(e))) => assert_eq!(e.kind, want),
            other => panic!("{other:?}"),
        }
    }
}

// RFC 9110 / #trust-layers: bytes as delivered; identity encoding requested.
#[test]
fn identity_encoding_is_requested_and_other_encodings_refused() {
    let s = Server::start(vec![
        Reply::ok(CATALOG, b"{}"),
        Reply::ok(CATALOG, b"{}").header("Content-Encoding", "gzip"),
    ]);
    fetch(&s.url("/a"), None, &loopback_policy()).expect("identity");
    let log = s.log();
    assert_eq!(log[0].header("accept-encoding"), Some("identity"));
    assert!(matches!(log[0].header("accept"), None | Some("*/*")));
    assert!(log[0].header("cookie").is_none());
    assert!(log[0].header("authorization").is_none());
    let r = fetch(&s.url("/b"), None, &loopback_policy());
    assert!(matches!(r, Err(FetchError::ContentEncoding)), "{r:?}");
}

// #safe-fetching: a proxy would resolve and connect, bypassing the address rule.
#[test]
fn proxy_environment_is_ignored() {
    if std::env::var("FETCH_PROXY_CHILD").is_ok() {
        let s = Server::start(vec![Reply::ok(CATALOG, b"{}")]);
        let f = fetch(&s.url("/"), None, &loopback_policy()).expect("direct fetch");
        assert_eq!(f.status, 200);
        return;
    }
    let proxy = Server::silent();
    let addr = format!("http://127.0.0.1:{}", proxy.port());
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["proxy_environment_is_ignored", "--exact", "--nocapture"])
        .env("FETCH_PROXY_CHILD", "1")
        .env("ALL_PROXY", &addr)
        .env("HTTP_PROXY", &addr)
        .env("HTTPS_PROXY", &addr)
        .env("all_proxy", &addr)
        .env("http_proxy", &addr)
        .env("https_proxy", &addr)
        .env_remove("NO_PROXY")
        .env_remove("no_proxy")
        .output()
        .expect("spawn child");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(proxy.log().is_empty(), "the proxy must see nothing");
}

// RFC 3986 section 5.1.3: the base is the URI the representation was retrieved from.
#[test]
fn fetched_url_is_the_final_url() {
    let s = Server::start(vec![
        Reply::redirect(301, "/final"),
        Reply::ok(CATALOG, b"{}"),
    ]);
    let f = fetch(&s.url("/start"), None, &loopback_policy()).unwrap();
    assert_eq!(f.url(), Some(s.url("/final").as_str()));
    assert_eq!(Fetched::new(200, None, Vec::new()).url(), None);
}

// RFC 9110 section 15.4: a redirect status without Location is a final response.
#[test]
fn redirect_without_location_is_a_final_response() {
    let s = Server::start(vec![Reply::new(302)]);
    let f = fetch(&s.url("/"), None, &loopback_policy()).unwrap();
    assert_eq!(f.status, 302);
}

#[test]
fn relative_redirect_location_resolves_against_the_hop_url() {
    let s = Server::start(vec![
        Reply::redirect(307, "../next?x=1"),
        Reply::ok(CATALOG, b"{}"),
    ]);
    let f = fetch(&s.url("/a/b/c"), None, &loopback_policy()).unwrap();
    assert_eq!(s.log()[1].path(), "/a/next?x=1");
    assert_eq!(f.url(), Some(s.url("/a/next?x=1").as_str()));
}

// RFC 3986 section 5.4.1 normal examples (WHATWG serialisation; "//g" gains the root "/").
#[test]
fn resolve_reference_matches_rfc_3986_normal_examples() {
    let base = Some("http://a/b/c/d;p?q");
    let cases = [
        ("g:h", "g:h"),
        ("g", "http://a/b/c/g"),
        ("./g", "http://a/b/c/g"),
        ("g/", "http://a/b/c/g/"),
        ("/g", "http://a/g"),
        ("//g", "http://g/"),
        ("?y", "http://a/b/c/d;p?y"),
        ("g?y", "http://a/b/c/g?y"),
        ("#s", "http://a/b/c/d;p?q#s"),
        ("g#s", "http://a/b/c/g#s"),
        ("g?y#s", "http://a/b/c/g?y#s"),
        (";x", "http://a/b/c/;x"),
        ("g;x", "http://a/b/c/g;x"),
        ("g;x?y#s", "http://a/b/c/g;x?y#s"),
        ("", "http://a/b/c/d;p?q"),
        (".", "http://a/b/c/"),
        ("./", "http://a/b/c/"),
        ("..", "http://a/b/"),
        ("../", "http://a/b/"),
        ("../g", "http://a/b/g"),
        ("../..", "http://a/"),
        ("../../", "http://a/"),
        ("../../g", "http://a/g"),
    ];
    for (reference, want) in cases {
        assert_eq!(
            resolve_reference(base, reference).as_deref().ok(),
            Some(want),
            "reference {reference:?}"
        );
    }
}

#[test]
fn reference_without_base_is_unresolvable_unless_absolute() {
    assert!(resolve_reference(None, "/relative").is_err());
    assert!(resolve_reference(None, "relative.json").is_err());
    assert!(resolve_reference(Some("not a base"), "relative.json").is_err());
    assert_eq!(
        resolve_reference(None, "https://example.com/a.json").unwrap(),
        "https://example.com/a.json"
    );
    // An absolute reference wins over the base.
    assert_eq!(
        resolve_reference(Some("https://a.example/x"), "https://b.example/y").unwrap(),
        "https://b.example/y"
    );
}

// Single request path: `reqwest` only in fetch.rs and pinned.rs; the trust crate opens no connection.
#[test]
fn fetch_is_the_only_request_path() {
    fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                rust_files(&p, out);
            } else if p.extension().is_some_and(|x| x == "rs") {
                out.push(p);
            }
        }
    }
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let client_src = crates.join("turul-ai-catalog-client").join("src");
    let mut named_fetch = false;
    for entry in std::fs::read_dir(&crates).unwrap().flatten() {
        let src = entry.path().join("src");
        let mut files = Vec::new();
        rust_files(&src, &mut files);
        let is_trust = entry.file_name() == "turul-ai-catalog-trust";
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            let name = f.file_name().unwrap().to_string_lossy().to_string();
            let allowed = f.parent() == Some(client_src.as_path())
                && (name == "fetch.rs" || name == "pinned.rs");
            if text.contains("reqwest") {
                assert!(allowed, "{} names reqwest", f.display());
                named_fetch = true;
            }
            if is_trust {
                assert!(!text.contains("std::net"), "{} names std::net", f.display());
            }
        }
    }
    assert!(named_fetch, "fetch.rs must be the code that names reqwest");
}

// RFC 9110 §12.5.1: a caller-named Accept is sent on every hop, redirects included, and Safe
// Fetching is unchanged.
#[test]
fn caller_named_accept_is_sent_on_every_hop() {
    let a2a = "application/a2a-agent-card+json";
    let s = Server::start(vec![Reply::redirect(302, "/card"), Reply::ok(a2a, b"{}")]);
    let f = fetch(&s.url("/start"), Some(a2a), &loopback_policy()).expect("fetched");
    assert_eq!(f.status, 200);
    let log = s.log();
    assert_eq!(log.len(), 2);
    for hop in &log {
        assert_eq!(hop.header("accept"), Some(a2a));
        assert_eq!(hop.header("accept-encoding"), Some("identity"));
    }
}

// A value that cannot be an HTTP header is refused before any request is sent.
#[test]
fn invalid_accept_is_refused_before_sending() {
    let s = Server::start(vec![Reply::ok(CATALOG, b"{}")]);
    let r = fetch(&s.url("/"), Some("bad\nvalue"), &loopback_policy());
    assert!(matches!(r, Err(FetchError::InvalidAccept)), "{r:?}");
    assert!(s.log().is_empty());
}
