mod common;

use common::*;
use turul_ai_catalog::validate::{Conformance, ConformanceLevel, Severity::Warning};

fn logo_doc(url: &str) -> String {
    format!(
        r#"{{"specVersion":"1.0","host":{{"displayName":"h","logoUrl":"{url}"}},"entries":[]}}"#
    )
}

// #privacy-considerations: "Logo URLs SHOULD use Data URIs"; a SHOULD is a warning
#[test]
fn non_data_logo_url_is_a_warning() {
    for u in [
        "https://example.com/logo.png",
        "http://example.com/logo.png",
        "ftp://example.com/logo.png",
        "//example.com/logo.png",
        "logo.png",
        "",
    ] {
        let r = report(&logo_doc(u));
        assert_only(&r, "logo-url-not-data-uri", Warning, "/host/logoUrl");
        assert_eq!(
            r.conformance(),
            Conformance::Met(ConformanceLevel::Discoverable)
        );
    }
}

// #privacy-considerations: the scheme is compared without regard to case (RFC 3986 section 3.1); the Data URI's syntax is not checked
#[test]
fn data_logo_url_gives_no_diagnostic() {
    for u in [
        "data:image/png;base64,AAAA",
        "DATA:image/svg+xml,<svg/>",
        "Data:,x",
        "data:",
    ] {
        assert_clean(&report(&logo_doc(u)));
    }
}
