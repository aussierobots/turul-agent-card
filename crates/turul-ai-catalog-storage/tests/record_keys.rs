mod common;

use common::*;
use turul_ai_catalog::identity::{EntryIdentifier, EntryKey};
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::Members;
use turul_ai_catalog_storage::RecordKey;

fn key(identifier: &str, version: Option<&str>) -> RecordKey {
    RecordKey::entry(&EntryKey::new(
        EntryIdentifier::new(identifier),
        version.map(str::to_string),
    ))
    .unwrap()
}

#[test]
fn absent_and_empty_version_are_distinct_keys() {
    assert_ne!(key("urn:air:a", None), key("urn:air:a", Some("")));
}

#[test]
fn distinct_entry_keys_give_distinct_record_keys() {
    let keys = [
        key("urn:air:a", None),
        key("urn:air:a", Some("1")),
        key("urn:air:a", Some("2")),
        key("urn:air:b", None),
        key("urn:air:b", Some("1")),
    ];
    for (i, a) in keys.iter().enumerate() {
        for b in &keys[i + 1..] {
            assert_ne!(a, b);
        }
    }
    assert_eq!(key("urn:air:a", Some("1")), key("urn:air:a", Some("1")));
}

#[test]
fn entry_record_keys_have_fixed_length_whatever_the_identifier() {
    let long = "x".repeat(5000);
    assert_eq!(key("a", None).as_str().len(), 66);
    assert_eq!(key(&long, Some(&long)).as_str().len(), 66);
    assert!(key("a", None).is_entry());
    assert_eq!(RecordKey::HOST.as_str(), "h");
    assert!(!RecordKey::HOST.is_entry());
}

#[test]
fn extensions_do_not_change_the_record_key() {
    let plain = entry("urn:air:a", Some("1"));
    let mut extended = plain.clone();
    let mut ext = Members::default();
    ext.insert(
        "com.example.sourceIdentifier".to_string(),
        Json::String("old-id".to_string()),
    );
    extended.extensions = Some(ext);
    assert_eq!(
        RecordKey::entry(&plain.key()).unwrap(),
        RecordKey::entry(&extended.key()).unwrap()
    );
}
