//! DID document retrieval acceptance and the network boundary of `resolve`.

use std::fs;
use std::path::Path;

use turul_ai_catalog_client::Fetched;
use turul_ai_catalog_trust::{DidWebRoot, ResolveError, accept_document, document_url};

fn root() -> DidWebRoot {
    DidWebRoot::parse("did:web:example.com").unwrap()
}

fn ok(body: &str) -> Fetched {
    Fetched::new(
        200,
        Some("application/did+json".into()),
        body.as_bytes().to_vec(),
    )
}

fn with_type(ct: Option<&str>) -> Fetched {
    Fetched::new(
        200,
        ct.map(String::from),
        br#"{"id":"did:web:example.com"}"#.to_vec(),
    )
}

#[test]
fn document_url_is_the_well_known_did_json() {
    assert_eq!(
        document_url(&root()),
        "https://example.com/.well-known/did.json"
    );
}

#[test]
fn document_id_other_than_signer_is_refused() {
    for id in ["did:web:other.example", "did:web:Example.com", ""] {
        let r = accept_document(&ok(&format!(r#"{{"id":"{id}"}}"#)), &root());
        assert!(matches!(r, Err(ResolveError::IdMismatch)), "{id}: {r:?}");
    }
    let r = accept_document(&ok(r#"{"id":"did:web:example.com"}"#), &root()).unwrap();
    assert_eq!(r.id(), &root());
}

#[test]
fn malformed_verification_relationships_are_refused() {
    let id = r#""id":"did:web:example.com""#;
    for (body, pointer) in [
        ("[]".to_string(), ""),
        ("{}".to_string(), "/id"),
        (r#"{"id":1}"#.to_string(), "/id"),
        (
            format!(r#"{{{id},"verificationMethod":[42]}}"#),
            "/verificationMethod/0",
        ),
        (
            format!(r#"{{{id},"verificationMethod":{{}}}}"#),
            "/verificationMethod",
        ),
        (
            format!(r#"{{{id},"verificationMethod":[{{"id":"a","type":"t"}}]}}"#),
            "/verificationMethod/0/controller",
        ),
        (
            format!(r#"{{{id},"assertionMethod":{{}}}}"#),
            "/assertionMethod",
        ),
        (
            format!(r##"{{{id},"assertionMethod":["#k",7]}}"##),
            "/assertionMethod/1",
        ),
    ] {
        match accept_document(&ok(&body), &root()) {
            Err(ResolveError::Invalid { pointer: p }) => {
                assert_eq!(p.to_string(), pointer, "{body}")
            }
            other => panic!("{body}: {other:?}"),
        }
    }
}

#[test]
fn duplicate_member_is_refused() {
    let r = accept_document(
        &ok(r#"{"id":"did:web:example.com","id":"did:web:example.com"}"#),
        &root(),
    );
    assert!(matches!(r, Err(ResolveError::Json(_))), "{r:?}");
}

#[test]
fn status_other_than_200_is_refused() {
    let f = Fetched::new(
        404,
        Some("application/did+json".into()),
        br#"{"id":"did:web:example.com"}"#.to_vec(),
    );
    assert!(matches!(
        accept_document(&f, &root()),
        Err(ResolveError::Status(404))
    ));
}

#[test]
fn non_did_media_type_is_refused() {
    for ct in [Some("text/html"), None] {
        let r = accept_document(&with_type(ct), &root());
        assert!(
            matches!(r, Err(ResolveError::MediaType(_))),
            "{ct:?}: {r:?}"
        );
    }
}

#[test]
fn did_and_json_media_types_are_accepted() {
    for ct in [
        "application/did+json",
        "application/did+ld+json",
        "application/json",
        "application/did+json; charset=utf-8",
        "Application/JSON",
    ] {
        assert!(
            accept_document(&with_type(Some(ct)), &root()).is_ok(),
            "{ct}"
        );
    }
}

#[test]
fn unread_members_do_not_affect_acceptance() {
    for extra in [
        r#""service":42"#,
        r#""service":[{"x":null}]"#,
        r#""authentication":"nope""#,
        r#""controller":7"#,
    ] {
        let body = format!(r#"{{"id":"did:web:example.com",{extra}}}"#);
        assert!(accept_document(&ok(&body), &root()).is_ok(), "{extra}");
    }
}

fn src_files(dir: &Path, out: &mut Vec<(String, String)>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            src_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push((p.display().to_string(), fs::read_to_string(&p).unwrap()));
        }
    }
}

#[test]
fn resolve_is_fetch_then_accept_document() {
    let src = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/did_web.rs")).unwrap();
    let start = src.find("pub fn resolve(").expect("resolve");
    let body = &src[start..];
    let body = &body[..body.find("\n}\n").unwrap()];
    let lines: Vec<&str> = body.lines().skip(1).map(str::trim).collect();
    assert_eq!(
        lines,
        [
            "let fetched = fetch(&document_url(root), policy).map_err(ResolveError::Fetch)?;",
            "accept_document(&fetched, root)",
        ]
    );
}

#[test]
fn trust_src_reaches_the_network_only_through_client_fetch() {
    let mut files = Vec::new();
    src_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    for (path, text) in files {
        for banned in ["std::net", "reqwest", "TcpStream", "hyper::"] {
            assert!(!text.contains(banned), "{path} uses {banned}");
        }
    }
    let did = fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/did_web.rs")).unwrap();
    assert_eq!(
        did.matches("fetch(").count(),
        1,
        "one fetch call in did_web"
    );
}
