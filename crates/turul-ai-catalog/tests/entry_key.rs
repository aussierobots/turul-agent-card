mod common;

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

use common::*;
use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier, EntryKey, HostIdentifier};
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::{ArtifactSource, CatalogEntry, Subject};

fn spec_fixture(rel: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/spec")
        .join(rel);
    fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn entry(identifier: &str, version: Option<&str>) -> CatalogEntry {
    let v = match version {
        Some(v) => format!(r#","version":{}"#, quote(v)),
        None => String::new(),
    };
    let doc = format!(
        r#"{{"specVersion":"1.0","entries":[{{"identifier":{},"type":"application/mcp-server-card+json","url":"https://example.com/a.json"{v}}}]}}"#,
        quote(identifier)
    );
    ok(&doc).entries.into_iter().next().unwrap()
}

fn quote(s: &str) -> String {
    format!("\"{s}\"")
}

fn hash_of(k: &EntryKey) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    k.hash(&mut h);
    h.finish()
}

// #catalog-entry: an unrecognised identifier scheme "MUST" be treated as opaque
#[test]
fn unrecognised_scheme_identifier_is_opaque_and_compared_exactly() {
    assert_ne!(
        EntryIdentifier::new("x-local:Foo/Bar"),
        EntryIdentifier::new("x-local:foo/bar")
    );
    assert_ne!(
        EntryIdentifier::new("urn:air:example.com:agent:a"),
        EntryIdentifier::new("URN:AIR:example.com:agent:a")
    );
    assert_ne!(EntryIdentifier::new("a"), EntryIdentifier::new("a "));
    assert_ne!(EntryIdentifier::new("a"), EntryIdentifier::new(" a"));
    // An identifier without a scheme separator is a valid key.
    let e = entry("no-colon-at-all", None);
    assert_eq!(e.key().identifier().as_str(), "no-colon-at-all");
    // Ordering and hashing follow the stored text exactly.
    let set: HashSet<EntryIdentifier> = ["x:A", "x:a", "x:A"]
        .into_iter()
        .map(EntryIdentifier::new)
        .collect();
    assert_eq!(set.len(), 2);
    assert!(EntryIdentifier::new("A") < EntryIdentifier::new("a"));
}

// #catalog-entry: the identifier is an open text format, kept as written through
// parse and serialisation
#[test]
fn identifier_survives_parse_and_serialisation_verbatim() {
    let odd = "  URN:AIR:Example.COM/Ünï \u{1F600} x-local:Foo ";
    let cat = ok(&format!(
        r#"{{"specVersion":"1.0","entries":[{{"identifier":"{odd}","type":"t","url":"u"}}]}}"#
    ));
    assert_eq!(cat.entries[0].identifier.as_str(), odd);
    let json = cat.to_json();
    let Json::Object(top) = json else {
        panic!("catalog is an object")
    };
    let Some(Json::Array(entries)) = top.get("entries") else {
        panic!("entries array")
    };
    let Json::Object(first) = &entries[0] else {
        panic!("entry object")
    };
    match first.get("identifier") {
        Some(Json::String(s)) => assert_eq!(s, odd),
        other => panic!("identifier written as {other:?}"),
    }
}

// #multi-version-entries: with `version` present the key is (identifier, version);
// absent, identifier alone; an empty string version is present
#[test]
fn absent_empty_and_nonempty_versions_give_distinct_keys() {
    let none = entry("urn:air:example.com:agent:a", None).key();
    let empty = entry("urn:air:example.com:agent:a", Some("")).key();
    let one = entry("urn:air:example.com:agent:a", Some("1")).key();
    assert_ne!(none, empty);
    assert_ne!(none, one);
    assert_ne!(empty, one);
    assert_eq!(none.version(), None);
    assert_eq!(empty.version(), Some(""));
    assert_eq!(one.version(), Some("1"));

    // The identifier alone is not the key: the spec's two versions of one agent differ.
    let cat = turul_ai_catalog::parse::parse(&spec_fixture("blocks/03-multi-version-entries.json"))
        .expect("spec example parses");
    let (a, b) = (cat.entries[0].key(), cat.entries[1].key());
    assert_eq!(a.identifier(), b.identifier());
    assert_ne!(a, b);
    assert_eq!(a.version(), Some("2.1.0"));
    assert_eq!(b.version(), Some("2.0.0"));
}

// #multi-version-entries: equal (identifier, version) is the same key
#[test]
fn same_identifier_and_version_give_equal_keys() {
    let a = entry("urn:air:example.com:agent:a", Some("1.0.0")).key();
    let b = entry("urn:air:example.com:agent:a", Some("1.0.0")).key();
    assert_eq!(a, b);
    assert_eq!(a.cmp(&b), std::cmp::Ordering::Equal);
    assert_eq!(hash_of(&a), hash_of(&b));
    let c = entry("urn:air:example.com:agent:a", Some("1.0.1")).key();
    assert_ne!(a, c);
    let d = entry("urn:air:example.com:agent:b", Some("1.0.0")).key();
    assert_ne!(a, d);
    // Version text is compared as written, not as semver.
    assert_ne!(
        entry("i", Some("1.0")).key(),
        entry("i", Some("1.0.0")).key()
    );
    let _ = a.clone();
    let _ = format!("{a:?}");
}

// #catalog-entry: `identifier` is CDDL `text`, so an empty value reaches the model
#[test]
fn empty_identifier_parses_into_the_model() {
    let e = entry("", None);
    assert_eq!(e.identifier.as_str(), "");
    assert_eq!(e.key().identifier().as_str(), "");
    assert_eq!(
        EntryIdentifier::new(""),
        EntryIdentifier::new(String::new())
    );
    let cat =
        ok(r#"{"specVersion":"1.0","host":{"displayName":"h","identifier":""},"entries":[]}"#);
    assert_eq!(cat.host.unwrap().identifier.unwrap().as_str(), "");
}

// #host-info: the host `identifier` is opaque text
#[test]
fn host_identifier_is_kept_verbatim() {
    for text in [
        "did:web:Example.COM",
        "Example.com",
        " spaced ",
        "urn:AIR:x",
    ] {
        let cat = ok(&format!(
            r#"{{"specVersion":"1.0","host":{{"displayName":"h","identifier":"{text}"}},"entries":[]}}"#
        ));
        assert_eq!(cat.host.unwrap().identifier.unwrap().as_str(), text);
    }
    assert_ne!(
        HostIdentifier::new("did:web:a.com"),
        HostIdentifier::new("did:web:A.com")
    );
}

// #catalog-entry, #subject-object, #host-info: identity fields carry typed values.
// Type ascriptions fail to compile if a field reverts to `String`.
#[test]
fn identity_fields_are_typed() {
    let cat = ok(
        r#"{"specVersion":"1.0","host":{"displayName":"h","identifier":"did:web:h"},"entries":[{"identifier":"i","type":"t","url":"u","trustManifests":[{"contributor":"did:web:c","subject":{"identifier":"i","type":"t","digest":"sha256:00"}}]}]}"#,
    );
    let _: &Option<HostIdentifier> = &cat.host.as_ref().unwrap().identifier;
    let e = &cat.entries[0];
    let _: &EntryIdentifier = &e.identifier;
    let _: &ArtifactType = &e.artifact_type;
    let s: &Subject = e.trust_manifests.as_ref().unwrap()[0]
        .subject
        .as_ref()
        .unwrap();
    let _: &EntryIdentifier = &s.identifier;
    let _: &ArtifactType = &s.artifact_type;

    let built = CatalogEntry::new(
        EntryIdentifier::new("i"),
        ArtifactType::new("t"),
        ArtifactSource::Url("u".to_string()),
    );
    assert_eq!(built.identifier, e.identifier);
    assert_eq!(built.artifact_type, e.artifact_type);
}
