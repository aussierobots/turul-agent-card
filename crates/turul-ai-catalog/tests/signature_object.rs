mod common;
#[path = "support/trust.rs"]
mod trust;

use common::*;
use trust::*;
use turul_ai_catalog::validate::Severity::*;
use turul_ai_catalog::validate::{DID_WEB_V1, RESERVED_PROFILES};

const C: &str = "did:web:example.com";
const ISSUED: &str = "2026-09-12T10:00:00Z";

/// A signed manifest of `contributor` over the matching subject.
fn signed(contributor: &str, sig: &str) -> String {
    manifest(
        contributor,
        &format!(r#","subject":{},"signature":{sig}"#, matching_subject()),
    )
}

fn with_sig(profile: &str, issued: &str, expires: Option<&str>) -> String {
    signed(C, &signature(C, profile, issued, expires))
}

fn root_signed(sig: &str) -> String {
    format!(r#"{{"specVersion":"1.0","signature":{sig},"entries":[]}}"#)
}

fn has(
    r: &turul_ai_catalog::validate::Report,
    code: &str,
    sev: turul_ai_catalog::validate::Severity,
    path: &str,
) -> bool {
    found(r)
        .iter()
        .any(|(c, s, p)| c == code && *s == sev && p == path)
}

// #contributor-identity: `signature.signer` "MUST exactly equal `contributor`"
#[test]
fn signer_must_equal_contributor() {
    let ok_m = signed(C, &good_signature(C));
    assert_clean(&vm(&ok_m));
    assert!(manifest_of(&doc_with_manifest(&ok_m)).signer_matches_contributor());

    let bad = signed(C, &good_signature("did:web:other.example"));
    assert_only(
        &vm(&bad),
        "signer-contributor-mismatch",
        Error,
        "/signature/signer",
    );
    assert_only(
        &report(&bound_doc(&entry_digest(), &[&bad])),
        "signer-contributor-mismatch",
        Error,
        "/entries/0/trustManifests/0/signature/signer",
    );
    assert!(!manifest_of(&doc_with_manifest(&bad)).signer_matches_contributor());

    // No signature: nothing matches.
    let unsigned = manifest(C, &format!(",{SUBSTANCE}"));
    assert!(!manifest_of(&doc_with_manifest(&unsigned)).signer_matches_contributor());
}

// #contributor-identity: compared "case-sensitively, without URI normalization"
#[test]
fn signer_and_contributor_compare_without_normalisation() {
    for (contributor, signer) in [
        ("did:web:acme.com", "did:web:Acme.com"),
        ("did:web:acme.com", "DID:web:acme.com"),
        ("urn:x:%41", "urn:x:A"),
        ("urn:x:A", "urn:x:%41"),
    ] {
        let m = signed(contributor, &good_signature(signer));
        let parsed = manifest_of(&doc_with_manifest(&m));
        assert!(
            !parsed.signer_matches_contributor(),
            "{contributor} vs {signer}"
        );
        assert!(
            has(
                &vm(&m),
                "signer-contributor-mismatch",
                Error,
                "/signature/signer"
            ),
            "{contributor} vs {signer}"
        );
    }
}

// #signature-object: `signer` is an absolute URI (RFC 3986 `absolute-URI`, no fragment)
#[test]
fn signer_must_be_an_rfc_3986_absolute_uri() {
    for good in [
        "did:web:example.com",
        "https://example.com/x",
        "urn:x:a",
        "foo:",
    ] {
        assert_clean(&vm(&signed(good, &good_signature(good))));
    }
    for bad in [
        "urn:x:<bad>",
        r#"urn:x:a\"b"#,
        "did:web:é.example",
        "urn:x:%zz",
        "http://[::1/x",
        "a:b#c",
        "did:web:x#k",
        "relative/path",
        "",
    ] {
        let m = signed(bad, &good_signature(bad));
        assert!(
            has(
                &vm(&m),
                "signer-not-absolute-uri",
                Error,
                "/signature/signer"
            ),
            "{bad}"
        );
        // The signer's own defect is reported once, whatever the contributor's.
        let n = found(&vm(&m))
            .iter()
            .filter(|(c, _, _)| c == "signer-not-absolute-uri")
            .count();
        assert_eq!(n, 1, "{bad}");
    }
}

// #profile-selection: "Independently defined profiles MUST use absolute URI identifiers";
// #signature-object: `profile` is "A non-empty string"
#[test]
fn profile_is_reserved_or_absolute_uri() {
    assert_eq!(DID_WEB_V1, "did-web-v1");
    assert_eq!(RESERVED_PROFILES, [DID_WEB_V1]);
    for good in [
        DID_WEB_V1,
        "https://example.com/profiles/v1",
        "urn:x:profile",
    ] {
        assert_clean(&vm(&with_sig(good, ISSUED, None)));
    }
    assert_only(
        &vm(&with_sig("did-web-v2", ISSUED, None)),
        "profile-unrecognised",
        Warning,
        "/signature/profile",
    );
    assert_only(
        &vm(&with_sig("", ISSUED, None)),
        "profile-empty",
        Error,
        "/signature/profile",
    );
    // A warning leaves the document usable: no error, so conformance is not lost to it.
    assert!(!vm(&with_sig("did-web-v2", ISSUED, None)).has_errors());
}

// #profile-selection: "Profile identifiers MUST be compared exactly and case-sensitively"
#[test]
fn profile_is_compared_exactly() {
    assert!(!RESERVED_PROFILES.contains(&"DID-WEB-V1"));
    assert_only(
        &vm(&with_sig("DID-WEB-V1", ISSUED, None)),
        "profile-unrecognised",
        Warning,
        "/signature/profile",
    );
    // An absolute URI is accepted as written, never normalised towards a reserved value.
    assert_clean(&vm(&with_sig("HTTPS://example.com/p", ISSUED, None)));
    assert_only(
        &vm(&with_sig(" did-web-v1", ISSUED, None)),
        "profile-unrecognised",
        Warning,
        "/signature/profile",
    );
}

// #signature-object: `expiresAt` "MUST denote an instant later than `issuedAt`"; timestamps are
// compared as instants, not strings
#[test]
fn expires_at_not_later_than_issued_at_is_an_error() {
    for expires in [
        "2026-09-12T10:00:00Z",      // equal
        "2026-09-12T20:00:00+10:00", // the same instant as issuedAt
        "2026-09-12T11:00:00+10:00", // earlier, though it sorts after as a string
        "2026-09-11T10:00:00Z",
    ] {
        assert_only(
            &vm(&with_sig(DID_WEB_V1, ISSUED, Some(expires))),
            "signature-expiry-order",
            Error,
            "/signature/expiresAt",
        );
    }
    for expires in [
        "2026-09-12T10:00:01Z",
        "2026-09-12T09:30:00-02:00", // later, though it sorts before as a string
        "2027-09-12T10:00:00Z",
    ] {
        assert_clean(&vm(&with_sig(DID_WEB_V1, ISSUED, Some(expires))));
    }
}

// #signature-acceptance-and-time: "MUST validate timestamp syntax and compare timestamps as instants"
#[test]
fn signature_timestamps_are_validated_and_compared_as_instants() {
    for bad in [
        "2026-09-12 10:00:00Z",
        "2026-09-12T10:00:00",
        "yesterday",
        "",
        "2026-13-01T00:00:00Z",
    ] {
        assert_only(
            &vm(&with_sig(DID_WEB_V1, bad, None)),
            "timestamp-invalid",
            Error,
            "/signature/issuedAt",
        );
        assert_only(
            &vm(&with_sig(DID_WEB_V1, ISSUED, Some(bad))),
            "timestamp-invalid",
            Error,
            "/signature/expiresAt",
        );
    }
    assert_clean(&vm(&with_sig(
        DID_WEB_V1,
        "2026-09-12T10:00:00.123+10:00",
        Some("2026-09-12T10:00:00.124+10:00"),
    )));
}

// An order between an invalid and a valid timestamp is undefined: only the syntax error is reported
#[test]
fn invalid_timestamp_gives_no_order_diagnostic() {
    let r = vm(&with_sig(
        DID_WEB_V1,
        "not a time",
        Some("2000-01-01T00:00:00Z"),
    ));
    assert_only(&r, "timestamp-invalid", Error, "/signature/issuedAt");
    let r = vm(&with_sig(DID_WEB_V1, ISSUED, Some("not a time")));
    assert_only(&r, "timestamp-invalid", Error, "/signature/expiresAt");
    let r = vm(&with_sig(DID_WEB_V1, "bad", Some("worse")));
    assert_eq!(
        found(&r)
            .iter()
            .map(|(c, _, p)| (c.as_str(), p.as_str()))
            .collect::<Vec<_>>(),
        [
            ("timestamp-invalid", "/signature/issuedAt"),
            ("timestamp-invalid", "/signature/expiresAt")
        ]
    );
}

// #signature-object, #signature-acceptance-and-time: the header, `did:web` shape and verification
// are not document defects; the document check never reads `jws` or applies `did:web` rules
#[test]
fn jws_content_and_did_web_shape_are_never_document_diagnostics() {
    for jws in ["", "not a jws", "a.b", "a..b", "ey.ey.ey", "\u{e9}"] {
        let sig = format!(
            r#"{{"signer":"did:web:Example.com","profile":"did-web-v1","issuedAt":"{ISSUED}","jws":"{jws}"}}"#
        );
        assert_clean(&vm(&signed("did:web:Example.com", &sig)));
    }
    // A did:web identifier of any shape is just an absolute URI here.
    for id in [
        "did:web:",
        "did:web:example.com:a:b",
        "did:web:example.com%3A8443",
        "did:web:exa mple",
    ] {
        let sig = good_signature(id);
        let r = vm(&signed(id, &sig));
        assert!(
            !found(&r)
                .iter()
                .any(|(c, _, _)| c.contains("did-web") || c.contains("jws")),
            "{id}: {:?}",
            found(&r)
        );
    }
}

// #signature-acceptance-and-time: the time window is the verifier's, so an expired signature is clean
#[test]
fn expired_signature_has_no_document_diagnostic() {
    assert_clean(&vm(&with_sig(
        DID_WEB_V1,
        "2000-01-01T00:00:00Z",
        Some("2000-01-02T00:00:00Z"),
    )));
    assert_clean(&vm(&with_sig(DID_WEB_V1, "2999-01-01T00:00:00Z", None)));
}

// #signature-object: "A Trust Manifest or catalog root MAY contain one `signature` object", with
// one member list for both; a root signature has no contributor or subject rule
#[test]
fn root_signature_members_are_checked() {
    assert_clean(&report(&root_signed(&good_signature(
        "did:web:example.com",
    ))));
    assert_clean(&report(&root_signed(&signature(
        "urn:x:any",
        "https://example.com/p",
        ISSUED,
        Some("2027-01-01T00:00:00Z"),
    ))));

    let at = |sig: &str, code: &str, sev, path: &str| {
        let r = report(&root_signed(sig));
        assert!(
            has(&r, code, sev, path),
            "{code} at {path}: {:?}",
            found(&r)
        );
        r
    };
    at(
        &signature("a:b#c", DID_WEB_V1, ISSUED, None),
        "signer-not-absolute-uri",
        Error,
        "/signature/signer",
    );
    at(
        &signature("did:web:x", "", ISSUED, None),
        "profile-empty",
        Error,
        "/signature/profile",
    );
    at(
        &signature("did:web:x", "other", ISSUED, None),
        "profile-unrecognised",
        Warning,
        "/signature/profile",
    );
    at(
        &signature("did:web:x", DID_WEB_V1, "bad", None),
        "timestamp-invalid",
        Error,
        "/signature/issuedAt",
    );
    at(
        &signature("did:web:x", DID_WEB_V1, ISSUED, Some(ISSUED)),
        "signature-expiry-order",
        Error,
        "/signature/expiresAt",
    );
}
