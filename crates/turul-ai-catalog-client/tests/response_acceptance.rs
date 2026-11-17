//! Catalog response gate contract.

mod support;

use support::loopback::{Reply, Server};
use turul_ai_catalog::parse::ParseErrorKind;
use turul_ai_catalog_client::{
    AcceptError, FetchError, FetchPolicy, Fetched, LoadWarning, Source, accept_response, fetch,
    load,
};

const FIXTURE: &[u8] = include_bytes!("fixtures/ai-catalog.json");

fn response(content_type: Option<&str>, body: &[u8]) -> Fetched {
    Fetched::new(200, content_type.map(String::from), body.to_vec())
}

// #media-type-registration-application-ai-catalog-json
#[test]
fn ai_catalog_media_type_is_accepted() {
    let a =
        accept_response(&response(Some("application/ai-catalog+json"), FIXTURE)).expect("accepted");
    assert!(a.warnings.is_empty());
}

// #link-relation-discovery step 4: a generic type such as application/json is acceptable.
#[test]
fn generic_json_gives_media_type_generic() {
    let a = accept_response(&response(Some("application/json"), FIXTURE)).expect("accepted");
    assert!(matches!(
        a.warnings.as_slice(),
        [LoadWarning::MediaTypeGeneric]
    ));
}

#[test]
fn other_media_types_are_refused() {
    for ct in [
        "text/html",
        "text/json",
        "application/ld+json",
        "application/octet-stream",
    ] {
        match accept_response(&response(Some(ct), FIXTURE)) {
            Err(AcceptError::MediaType(Some(got))) => assert_eq!(got, ct),
            other => panic!("{ct}: {other:?}"),
        }
    }
    assert!(matches!(
        accept_response(&response(None, FIXTURE)),
        Err(AcceptError::MediaType(None))
    ));
}

// #consumer-behavior: an unsupported major version is refused.
#[test]
fn unsupported_major_is_refused() {
    let r = accept_response(&response(
        Some("application/ai-catalog+json"),
        br#"{"specVersion":"2.0"}"#,
    ));
    match r {
        Err(AcceptError::Parse(e)) => {
            assert!(matches!(e.kind, ParseErrorKind::UnsupportedMajor { .. }))
        }
        other => panic!("{other:?}"),
    }
}

// #location-independence: a catalog in a file has no media type.
#[test]
fn file_source_has_no_media_type_rule() {
    let dir = std::env::temp_dir().join(format!("catalog-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("catalog.txt");
    std::fs::write(&path, FIXTURE).unwrap();
    let loaded = load(&Source::File(path), &FetchPolicy::default()).expect("file loads");
    assert_eq!(loaded.base, None);
    assert!(loaded.warnings.is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

// RFC 9110 section 15: only 200 carries the complete requested representation.
#[test]
fn status_other_than_200_is_refused() {
    for status in [203u16, 204, 206, 302, 404] {
        let f = Fetched::new(
            status,
            Some("application/ai-catalog+json".into()),
            FIXTURE.to_vec(),
        );
        assert!(
            matches!(accept_response(&f), Err(AcceptError::Status(s)) if s == status),
            "status {status}"
        );
    }
}

// RFC 9110 section 8.3.1: media type is case-insensitive; parameters are ignored.
#[test]
fn media_type_is_case_insensitive_and_parameters_are_ignored() {
    for ct in [
        "Application/AI-Catalog+JSON; charset=utf-8",
        " application/ai-catalog+json ;x=y",
    ] {
        let a =
            accept_response(&response(Some(ct), FIXTURE)).unwrap_or_else(|e| panic!("{ct}: {e:?}"));
        assert!(a.warnings.is_empty(), "{ct}");
    }
    let a = accept_response(&response(Some("APPLICATION/JSON;charset=utf-8"), FIXTURE)).unwrap();
    assert!(matches!(
        a.warnings.as_slice(),
        [LoadWarning::MediaTypeGeneric]
    ));
}

#[test]
fn missing_or_repeated_content_type_is_refused() {
    assert!(matches!(
        accept_response(&response(None, FIXTURE)),
        Err(AcceptError::MediaType(None))
    ));
    let mut p = FetchPolicy::default();
    p.permit_loopback_http = true;
    let s = Server::start(vec![
        Reply::new(200)
            .header("Content-Type", "application/ai-catalog+json")
            .header("Content-Type", "application/json")
            .body(FIXTURE),
    ]);
    let r = fetch(&s.url("/"), &p);
    assert!(matches!(r, Err(FetchError::ContentType)), "{r:?}");
}
