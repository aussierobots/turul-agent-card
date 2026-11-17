mod common;

use common::*;
use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
use turul_ai_catalog::model::{ArtifactSource, CatalogEntry, HostInfo, Members};
use turul_ai_catalog_storage::{CatalogId, EntryRecord, HostRecord, RecordError};

#[test]
fn host_record_refuses_an_invalid_host() {
    let err = HostRecord::new(HostInfo::new("a\u{FFFF}".to_string())).unwrap_err();
    assert!(
        matches!(err, RecordError::NotIJson(_) | RecordError::Invalid(_)),
        "{err:?}"
    );
    let ok = host();
    let text = std::str::from_utf8(ok.bytes()).unwrap();
    assert_eq!(text, r#"{"displayName":"Example Operator"}"#);
}

#[test]
fn entry_record_refuses_an_entry_with_an_error_diagnostic() {
    let bad = CatalogEntry::new(
        EntryIdentifier::new(""),
        ArtifactType::MCP_SERVER_CARD,
        ArtifactSource::Url("not a uri".to_string()),
    );
    let err = EntryRecord::new(bad).unwrap_err();
    assert!(matches!(err, RecordError::Invalid(_)), "{err:?}");
}

#[test]
fn entry_record_refuses_non_i_json() {
    let mut e = entry("urn:air:a", None);
    let mut ext = Members::default();
    ext.insert("com.example.n".to_string(), number("1e999"));
    e.extensions = Some(ext);
    let err = EntryRecord::new(e).unwrap_err();
    assert!(
        matches!(err, RecordError::NotIJson(_) | RecordError::Invalid(_)),
        "{err:?}"
    );
}

#[test]
fn entry_record_bytes_are_canonical() {
    let mut e = entry("urn:air:a", Some("1"));
    e.display_name = Some("A".to_string());
    let r = EntryRecord::new(e.clone()).unwrap();
    assert_eq!(
        r.bytes(),
        e.to_json().to_canonical_vec().unwrap().as_slice()
    );
    let text = std::str::from_utf8(r.bytes()).unwrap();
    assert!(
        text.starts_with(r#"{"displayName":"A","identifier":"urn:air:a""#),
        "{text}"
    );
    assert!(!text.contains(' '));
    assert_eq!(r.key(), e.key());
}

#[test]
fn catalog_id_accepts_only_its_alphabet_and_length() {
    assert!(CatalogId::new("a.B_c-1").is_ok());
    assert!(CatalogId::new("x".repeat(128)).is_ok());
    for bad in ["", "a b", "a/b", "é"] {
        assert_eq!(
            CatalogId::new(bad),
            Err(RecordError::InvalidCatalogId),
            "{bad}"
        );
    }
    assert_eq!(
        CatalogId::new("x".repeat(129)),
        Err(RecordError::InvalidCatalogId)
    );
}
