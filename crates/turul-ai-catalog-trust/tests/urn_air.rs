//! `urn:air` identifier components.

use turul_ai_catalog_trust::UrnAir;

#[test]
fn components_split_into_publisher_namespace_and_name() {
    let u = UrnAir::parse("urn:air:example.com:finance:agent:billing").unwrap();
    assert_eq!(u.publisher(), "example.com");
    assert_eq!(u.namespace(), "finance:agent");
    assert_eq!(u.name(), "billing");
    let u = UrnAir::parse("urn:air:example.com:finance:billing").unwrap();
    assert_eq!(
        (u.publisher(), u.namespace(), u.name()),
        ("example.com", "finance", "billing")
    );
}

#[test]
fn fewer_than_three_components_is_none() {
    assert_eq!(UrnAir::parse("urn:air:example.com:billing"), None);
    assert_eq!(UrnAir::parse("urn:air:example.com"), None);
    assert_eq!(UrnAir::parse("urn:air:"), None);
}

#[test]
fn empty_component_is_none() {
    for s in [
        "urn:air:example.com::billing",
        "urn:air::a:b",
        "urn:air:example.com:a:",
    ] {
        assert_eq!(UrnAir::parse(s), None, "{s}");
    }
}

#[test]
fn prefix_is_case_sensitive() {
    assert_eq!(UrnAir::parse("URN:AIR:example.com:a:b"), None);
    assert_eq!(UrnAir::parse("urn:AIR:example.com:a:b"), None);
    assert_eq!(UrnAir::parse("xurn:air:example.com:a:b"), None);
}

#[test]
fn publisher_failing_the_domain_rule_still_parses() {
    let u = UrnAir::parse("urn:air:Example.com:a:b").unwrap();
    assert_eq!(u.publisher(), "Example.com");
}
