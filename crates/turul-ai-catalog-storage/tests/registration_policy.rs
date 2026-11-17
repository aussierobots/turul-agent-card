use std::time::Duration;

use turul_ai_catalog_storage::{PolicyError, RegistrationPolicy, WriterGrant, WriterId};

fn publishers(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

// #catalog-projection rule 3: no prefix may admit an identifier that looks like `urn:air`.
#[test]
fn urn_air_prefix_is_refused() {
    for prefix in ["urn:air:", "URN:AIR:", "Urn:Air:x", "urn:air:example.com:"] {
        assert_eq!(
            WriterGrant::new([], [prefix.to_string()], false),
            Err(PolicyError::UrnAirPrefix(prefix.to_string())),
            "{prefix}"
        );
    }
    for prefix in ["urn:", "URN", "urn:air", "urn:airline:"] {
        assert!(
            WriterGrant::new([], [prefix.to_string()], false).is_ok(),
            "{prefix}"
        );
    }
}

#[test]
fn empty_grant_is_refused() {
    assert_eq!(
        WriterGrant::new([], [], false),
        Err(PolicyError::EmptyGrant)
    );
    assert_eq!(WriterGrant::new([], [], true), Err(PolicyError::EmptyGrant));
}

#[test]
fn empty_prefix_is_refused() {
    assert_eq!(
        WriterGrant::new([], [String::new()], false),
        Err(PolicyError::EmptyPrefix)
    );
    assert_eq!(
        WriterGrant::new(publishers(&["example.com"]), [String::new()], false),
        Err(PolicyError::EmptyPrefix)
    );
}

// The publisher component "MUST satisfy the root did:web domain restrictions".
#[test]
fn publisher_that_is_not_a_root_did_web_domain_is_refused() {
    for bad in [
        "Example.com",
        "example.com.",
        "example.com:8443",
        "10.0.0.1",
        "0x7f.1",
        "bücher.example",
        "-a.example",
        "",
    ] {
        assert_eq!(
            WriterGrant::new(publishers(&[bad]), [], false),
            Err(PolicyError::InvalidPublisher(bad.to_string())),
            "{bad:?}"
        );
    }
    assert!(WriterGrant::new(publishers(&["xn--bcher-kva.example"]), [], false).is_ok());
}

#[test]
fn zero_lease_ttl_is_refused() {
    assert_eq!(
        RegistrationPolicy::new([], Duration::ZERO),
        Err(PolicyError::ZeroLeaseTtl)
    );
}

// RFC 3339 writes four-digit years, so a lease from the Unix epoch must end by
// 9999-12-31T23:59:59.999999999Z; a longer lease could never be stored.
#[test]
fn lease_ttl_past_year_9999_is_refused() {
    let last = Duration::new(253_402_300_799, 999_999_999);
    assert!(RegistrationPolicy::new([], last).is_ok());
    for ttl in [last + Duration::from_nanos(1), Duration::MAX] {
        assert_eq!(
            RegistrationPolicy::new([], ttl),
            Err(PolicyError::LeaseTtlOutOfRange),
            "{ttl:?}"
        );
    }
}

#[test]
fn duplicate_writer_is_refused() {
    let grant = WriterGrant::new(publishers(&["example.com"]), [], false).unwrap();
    let w = WriterId::new("svc").unwrap();
    assert_eq!(
        RegistrationPolicy::new(
            [(w.clone(), grant.clone()), (w.clone(), grant)],
            Duration::from_secs(60)
        ),
        Err(PolicyError::DuplicateWriter(w))
    );
}

#[cfg(feature = "in-memory")]
#[test]
fn signature_required_without_verifier_is_refused() {
    use turul_ai_catalog_storage::{CatalogId, InMemoryCatalogStore, Registrar};

    let store = std::sync::Arc::new(InMemoryCatalogStore::new());
    let grant = WriterGrant::new(publishers(&["example.com"]), [], true).unwrap();
    let policy = RegistrationPolicy::new(
        [(WriterId::new("svc").unwrap(), grant)],
        Duration::from_secs(60),
    )
    .unwrap();
    let refused = Registrar::new(store, CatalogId::new("main").unwrap(), policy, None);
    assert!(matches!(refused, Err(PolicyError::VerifierMissing)));
}
