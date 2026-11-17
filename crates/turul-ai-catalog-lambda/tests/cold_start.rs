//! Cold-start contract: `ServedCatalog::assemble` over a store, and the configuration that selects it.

mod support;

use async_trait::async_trait;
use futures::executor::block_on;
use support::{body_bytes, catalog_id, request, served_from};
use turul_ai_catalog::identity::{EntryKey, WELL_KNOWN_PATH};
use turul_ai_catalog::parse::parse;
use turul_ai_catalog::validate::{Conformance, Severity, validate};
use turul_ai_catalog_lambda::{
    ColdStartError, ServedCatalog, catalog_id as id_from, handle, max_age,
};
use turul_ai_catalog_storage::{
    AssembleError, CatalogId, CatalogStore, Condition, EntryRecord, Etag, HostRecord, ImportError,
    InMemoryCatalogStore, StoreError, Stored, import,
};

const SPEC_EXAMPLE: &str = include_str!("fixtures/ai-catalog.json");

const ENTRY: &str = r#"{"identifier":"urn:example:a","type":"application/a2a-agent-card+json","url":"https://example.com/a.json"}"#;

fn doc(entry: &str) -> String {
    format!(r#"{{"specVersion":"1.0","entries":[{entry}]}}"#)
}

/// The spec example with its one signature's `jws` replaced.
fn with_jws(jws: &str) -> String {
    let start = SPEC_EXAMPLE.find(r#""jws": ""#).expect("jws member") + r#""jws": ""#.len();
    let end = start + SPEC_EXAMPLE[start..].find('"').expect("jws end");
    format!("{}{}{}", &SPEC_EXAMPLE[..start], jws, &SPEC_EXAMPLE[end..])
}

fn assert_starts_and_serves(document: &str) {
    let served = served_from(document);
    let r = handle(&served, &request("GET", WELL_KNOWN_PATH));
    assert_eq!(r.status().as_u16(), 200);
    let body = body_bytes(&r);
    assert!(!validate(&parse(&body).expect("body parses")).has_errors());
}

/// A store that fails every read.
struct Unreachable;

#[async_trait]
impl CatalogStore for Unreachable {
    fn backend_name(&self) -> &'static str {
        "unreachable"
    }
    async fn put_entry(
        &self,
        _: &CatalogId,
        _: &EntryRecord,
        _: Condition,
    ) -> Result<Etag, StoreError> {
        Err(StoreError::Backend("unreachable".into()))
    }
    async fn get_entry(
        &self,
        _: &CatalogId,
        _: &EntryKey,
    ) -> Result<Option<Stored<EntryRecord>>, StoreError> {
        Err(StoreError::Backend("unreachable".into()))
    }
    async fn delete_entry(
        &self,
        _: &CatalogId,
        _: &EntryKey,
        _: Option<&Etag>,
    ) -> Result<(), StoreError> {
        Err(StoreError::Backend("unreachable".into()))
    }
    async fn list_entries(&self, _: &CatalogId) -> Result<Vec<Stored<EntryRecord>>, StoreError> {
        Err(StoreError::Backend("unreachable".into()))
    }
    async fn put_host(
        &self,
        _: &CatalogId,
        _: &HostRecord,
        _: Condition,
    ) -> Result<Etag, StoreError> {
        Err(StoreError::Backend("unreachable".into()))
    }
    async fn get_host(&self, _: &CatalogId) -> Result<Option<Stored<HostRecord>>, StoreError> {
        Err(StoreError::Backend("unreachable".into()))
    }
}

/// A store holding `urn:example:a` both unversioned and versioned, as one written before
/// `put_entry` refused other-kind entries can.
struct WrittenBeforeTheGuard(Vec<EntryRecord>);

fn written_before_the_guard() -> WrittenBeforeTheGuard {
    let entry = |document: &str| {
        let entry = parse(document.as_bytes())
            .expect("parses")
            .entries
            .remove(0);
        EntryRecord::new(entry).expect("entry passes the record gate")
    };
    WrittenBeforeTheGuard(vec![
        entry(&doc(&ENTRY.replace("}", r#","version":"1"}"#))),
        entry(&doc(ENTRY)),
    ])
}

#[async_trait]
impl CatalogStore for WrittenBeforeTheGuard {
    fn backend_name(&self) -> &'static str {
        "written-before-the-guard"
    }
    async fn put_entry(
        &self,
        _: &CatalogId,
        _: &EntryRecord,
        _: Condition,
    ) -> Result<Etag, StoreError> {
        unreachable!()
    }
    async fn get_entry(
        &self,
        _: &CatalogId,
        _: &EntryKey,
    ) -> Result<Option<Stored<EntryRecord>>, StoreError> {
        unreachable!()
    }
    async fn delete_entry(
        &self,
        _: &CatalogId,
        _: &EntryKey,
        _: Option<&Etag>,
    ) -> Result<(), StoreError> {
        unreachable!()
    }
    async fn list_entries(&self, _: &CatalogId) -> Result<Vec<Stored<EntryRecord>>, StoreError> {
        Ok(self
            .0
            .iter()
            .map(|r| Stored {
                record: r.clone(),
                etag: Etag::of(r.bytes()),
            })
            .collect())
    }
    async fn put_host(
        &self,
        _: &CatalogId,
        _: &HostRecord,
        _: Condition,
    ) -> Result<Etag, StoreError> {
        unreachable!()
    }
    async fn get_host(&self, _: &CatalogId) -> Result<Option<Stored<HostRecord>>, StoreError> {
        Ok(None)
    }
}

// #top-level-structure: `entries` "MAY be empty", so an empty store is a valid catalog.
#[test]
fn empty_store_serves_a_valid_empty_catalog() {
    let served = block_on(ServedCatalog::assemble(
        &InMemoryCatalogStore::new(),
        &catalog_id(),
    ))
    .expect("empty store assembles");
    let r = handle(&served, &request("GET", WELL_KNOWN_PATH));
    assert_eq!(body_bytes(&r), br#"{"entries":[],"specVersion":"1.0"}"#);
}

// #multi-version-entries: an unversioned entry beside a versioned one breaks uniqueness. Every
// backend refuses such a put, so the records come from a store written before it did.
#[test]
fn invalid_store_fails_cold_start() {
    match block_on(ServedCatalog::assemble(
        &written_before_the_guard(),
        &catalog_id(),
    )) {
        Err(ColdStartError::Assemble(AssembleError::Invalid(report))) => {
            assert!(report.has_errors())
        }
        Err(e) => panic!("wrong error: {e}"),
        Ok(_) => panic!("an error diagnostic must fail cold start"),
    }
    match block_on(ServedCatalog::assemble(&Unreachable, &catalog_id())) {
        Err(ColdStartError::Assemble(AssembleError::Store(StoreError::Backend(_)))) => {}
        Err(e) => panic!("wrong error: {e}"),
        Ok(_) => panic!("an unreachable store must fail cold start"),
    }
}

#[test]
fn cold_start_error_names_each_error_code_and_pointer() {
    let store = written_before_the_guard();
    let Err(err) = block_on(ServedCatalog::assemble(&store, &catalog_id())) else {
        panic!("must fail cold start");
    };
    let ColdStartError::Assemble(AssembleError::Invalid(report)) = &err else {
        panic!("wrong error: {err}");
    };
    let text = err.to_string();
    let errors: Vec<_> = report
        .diagnostics()
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .collect();
    assert!(!errors.is_empty());
    for d in errors {
        assert!(text.contains(d.code.as_str()), "{text}");
        assert!(text.contains(&d.path.to_string()), "{text}");
    }
}

#[cfg(feature = "dynamodb")]
#[test]
fn missing_table_configuration_fails_cold_start() {
    use turul_ai_catalog_lambda::table_name;
    for raw in [None, Some("")] {
        match table_name(raw) {
            Err(ColdStartError::Config(m)) => assert!(m.contains("TURUL_AI_CATALOG_TABLE"), "{m}"),
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(table_name(Some("t")).expect("named"), "t");
    match id_from(None) {
        Err(ColdStartError::Config(m)) => assert!(m.contains("TURUL_AI_CATALOG_ID"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn catalog_id_and_refresh_interval_are_validated() {
    assert_eq!(id_from(Some("prod.v1")).expect("valid").as_str(), "prod.v1");
    assert!(matches!(
        id_from(Some("a b")),
        Err(ColdStartError::Config(_))
    ));
    assert_eq!(max_age(None).expect("default").as_secs(), 60);
    assert_eq!(max_age(Some("0")).expect("zero").as_secs(), 0);
    assert!(matches!(
        max_age(Some("soon")),
        Err(ColdStartError::Config(_))
    ));
    assert!(matches!(
        max_age(Some("-1")),
        Err(ColdStartError::Config(_))
    ));
}

// #cddl-schema "digest": an unknown algorithm is a warning; #catalog-entry nesting limit gives Undetermined.
#[test]
fn warnings_do_not_fail_cold_start() {
    let unknown_digest = doc(&format!(
        r#"{{"identifier":"urn:example:a","type":"application/a2a-agent-card+json","url":"https://example.com/a.json","digest":"sha3-256:{}"}}"#,
        "a".repeat(64)
    ));
    let report = validate(&parse(unknown_digest.as_bytes()).expect("parses"));
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|d| d.code.as_str() == "digest-algorithm-unknown"
                && d.severity == Severity::Warning)
    );
    assert_starts_and_serves(&unknown_digest);

    // Root plus four nested inline catalogs: the fifth level is not descended.
    let mut nested = format!(r#"{{"specVersion":"1.0","entries":[{ENTRY}]}}"#);
    for n in 0..4 {
        nested = format!(
            r#"{{"specVersion":"1.0","entries":[{{"identifier":"urn:example:n{n}","type":"application/ai-catalog+json","data":{nested}}}]}}"#
        );
    }
    let report = validate(&parse(nested.as_bytes()).expect("parses"));
    assert_eq!(report.conformance(), Conformance::Undetermined);
    assert!(!report.has_errors());
    assert_starts_and_serves(&nested);
}

// #signed-payload-and-jws-construction, #did-web: header and signer shape are never checked by the format crate.
#[test]
fn signature_defects_do_not_fail_cold_start() {
    // base64url of `{}` and of `{"alg":"none"}`
    assert_starts_and_serves(&with_jws("e30..detached-jws-signature"));
    assert_starts_and_serves(&with_jws("eyJhbGciOiJub25lIn0..detached-jws-signature"));
    // `contributor` changes with `signer`: #contributor-identity "MUST exactly equal" stays met, so
    // the only defect is the `did:web` signer shape.
    let signer = SPEC_EXAMPLE
        .replacen(
            r#""contributor": "did:web:acme-corp.com""#,
            r#""contributor": "did:web:Example.com""#,
            1,
        )
        .replacen(
            r#""signer": "did:web:acme-corp.com""#,
            r#""signer": "did:web:Example.com""#,
            1,
        );
    assert_eq!(signer.matches("did:web:Example.com").count(), 2);
    assert_starts_and_serves(&signer);
}

// #nested-catalog-entries: a `url`-nested catalog is a separate document and is not fetched.
#[test]
fn url_nested_children_are_not_fetched_at_cold_start() {
    assert_starts_and_serves(&doc(
        r#"{"identifier":"urn:example:child","type":"application/ai-catalog+json","url":"https://127.0.0.1/x.json"}"#,
    ));
}

// #signed-payload-and-jws-construction: the served bytes are canonical, so a number that is not
// I-JSON never reaches the store.
#[test]
fn non_i_json_numbers_are_refused_before_they_can_be_served() {
    let document = parse(
        doc(r#"{"identifier":"urn:example:a","type":"application/vnd.example+json","data":{"n":1e400}}"#)
            .as_bytes(),
    )
    .expect("parses");
    let store = InMemoryCatalogStore::new();
    assert!(matches!(
        block_on(import(&store, &catalog_id(), &document)),
        Err(ImportError::Record(_))
    ));
}
