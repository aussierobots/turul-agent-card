mod common;
#[path = "support/trust.rs"]
mod trust;

use common::*;
use trust::*;
use turul_ai_catalog::validate::Severity::*;
use turul_ai_catalog::validate::validate_manifest;

const C: &str = "did:web:example.com";

fn sub(extra: &str) -> String {
    manifest(C, &format!(",{extra}"))
}

// #contributor-identity: `contributor` "MUST be an absolute URI"; RFC 3986 grammar, no fragment
#[test]
fn contributor_must_be_an_rfc_3986_absolute_uri() {
    for good in [
        "did:web:example.com",
        "https://example.com/x",
        "urn:x:a",
        "foo:",
    ] {
        assert_clean(&vm(&manifest(good, &format!(",{SUBSTANCE}"))));
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
        assert_only(
            &vm(&manifest(bad, &format!(",{SUBSTANCE}"))),
            "contributor-not-absolute-uri",
            Error,
            "/contributor",
        );
    }
}

// #independent-contributions-and-updates: several manifests from one contributor, different profiles
#[test]
fn several_manifests_from_one_contributor_are_valid() {
    let a = sub(&format!(
        r#""subject":{},"signature":{}"#,
        matching_subject(),
        good_signature(C)
    ));
    let b = sub(&format!(
        r#""subject":{},"signature":{}"#,
        matching_subject(),
        signature(
            C,
            "https://example.com/profiles/v1",
            "2026-09-12T10:00:00Z",
            None
        )
    ));
    assert_clean(&report(&bound_doc(&entry_digest(), &[&a, &b])));
}

// #independent-contributions-and-updates: an updated manifest for the same release
#[test]
fn updated_manifest_for_the_same_release_is_valid() {
    let m = |issued: &str| {
        sub(&format!(
            r#""subject":{},"signature":{}"#,
            matching_subject(),
            signature(C, "did-web-v1", issued, None)
        ))
    };
    let (old, new) = (m("2026-09-12T10:00:00Z"), m("2026-10-01T10:00:00Z"));
    assert_clean(&report(&bound_doc(&entry_digest(), &[&old, &new])));
}

// #manifest-validity: "MUST contain at least one of" signature, trustSchema, attestations,
// provenance, extensions; `subject`, `additionalSignatures` and unknown members do not count
#[test]
fn manifest_without_substance_is_an_error() {
    let sig = format!(
        r#""subject":{},"signature":{}"#,
        matching_subject(),
        good_signature(C)
    );
    for good in [
        sig.as_str(),
        r#""trustSchema":{"identifier":"i","version":"1"}"#,
        r#""attestations":[{"type":"t","uri":"https://a.example/r"}]"#,
        r#""provenance":[{"relation":"publishedFrom","sourceId":"https://a.example/s"}]"#,
        SUBSTANCE,
    ] {
        assert_clean(&vm(&sub(good)));
    }
    let subject_only = format!(r#""subject":{}"#, matching_subject());
    for bad in [
        subject_only.as_str(),
        r#""additionalSignatures":[]"#,
        r#""https://example.com/unknown":1"#,
        r#""attestations":[]"#,
        r#""provenance":[]"#,
        r#""extensions":{}"#,
    ] {
        assert_only(&vm(&sub(bad)), "manifest-not-substantive", Error, "");
    }
}

// #manifest-validity: "A manifest containing a `signature` MUST also contain `subject`";
// #optional-members: `subject` "REQUIRED when `signature` is present"
#[test]
fn signed_manifest_without_subject_is_an_error() {
    let m = sub(&format!(r#""signature":{}"#, good_signature(C)));
    assert_only(&vm(&m), "manifest-subject-missing", Error, "/subject");
    // The entry carries `digest`, so only the missing subject is reported.
    let r = report(&bound_doc(&entry_digest(), &[&m]));
    assert_only(
        &r,
        "manifest-subject-missing",
        Error,
        "/entries/0/trustManifests/0/subject",
    );
}

// #manifest-validity: a signed manifest MAY endorse only the release identified by its subject;
// #the-did-web-publisher-profile: a manifest of only contributor, subject and signature
#[test]
fn release_only_signed_manifest_is_valid() {
    let m = sub(&format!(
        r#""subject":{},"signature":{}"#,
        matching_subject(),
        good_signature(C)
    ));
    assert_clean(&vm(&m));
    assert_clean(&report(&bound_doc(&entry_digest(), &[&m])));
}

// #manifest-validity: "Empty manifests MUST be omitted"
#[test]
fn manifest_with_only_contributor_is_an_error() {
    let m = manifest(C, "");
    assert_only(&vm(&m), "manifest-not-substantive", Error, "");
    assert_only(
        &report(&bound_doc("", &[&m])),
        "manifest-not-substantive",
        Error,
        "/entries/0/trustManifests/0",
    );
}

// #optional-members: the signed manifest "lets the signed manifest travel independently of the
// entry": a standalone manifest is validated without an entry, so no binding diagnostic exists
#[test]
fn standalone_manifest_is_validated_alone() {
    let m = sub(&format!(
        r#""subject":{},"signature":{}"#,
        subject(
            "urn:air:other.example:x:y",
            "text/plain",
            &digest(),
            Some("9")
        ),
        good_signature(C)
    ));
    assert_clean(&vm(&m));
    // The same manifest in an entry it does not describe is a binding failure, found by `validate` only.
    let r = report(&bound_doc(&entry_digest(), &[&m]));
    assert!(found(&r).iter().all(|(c, _, _)| c == "subject-mismatch"));
    assert!(!found(&r).is_empty());
}

// #level-1-minimal-catalog: each manifest "when present at any level, MUST satisfy Manifest
// Validity", including one in an inline nested catalog
#[test]
fn manifest_validity_applies_in_inline_nested_catalogs() {
    let inner = doc(&[&entry(
        "leaf",
        &format!(r#","trustManifests":[{}]"#, manifest(C, "")),
    )]);
    let outer = doc(&[&format!(
        r#"{{"identifier":"n","type":"application/ai-catalog+json","data":{inner}}}"#
    )]);
    assert_only(
        &report(&outer),
        "manifest-not-substantive",
        Error,
        "/entries/0/data/entries/0/trustManifests/0",
    );
}

// #level-3-trusted-catalog: "Additional contributors MAY provide their own Trust Manifests"
#[test]
fn manifests_from_several_contributors_are_valid() {
    let a = manifest("did:web:acme.com", &format!(",{SUBSTANCE}"));
    let b = manifest(
        "did:web:assessor.example",
        r#","attestations":[{"type":"SOC2-Type2","uri":"https://assessor.example/r.pdf"}]"#,
    );
    assert_clean(&report(&bound_doc("", &[&a, &b])));
}

// #level-3-trusted-catalog: "MAY include signed attestations, provenance, and manifest extensions"
#[test]
fn signed_manifest_with_attestations_provenance_and_extensions_is_valid() {
    let m = sub(&format!(
        r#""subject":{},"signature":{},
        "attestations":[{{"type":"SOC2-Type2","uri":"https://a.example/r.pdf","digest":"{d}"}}],
        "provenance":[{{"relation":"publishedFrom","sourceId":"https://a.example/s","sourceDigest":"{d}"}}],
        "extensions":{{"https://example.com/x":{{"k":1}}}}"#,
        matching_subject(),
        good_signature(C),
        d = digest()
    ));
    assert_clean(&vm(&m));
    assert_clean(&report(&bound_doc(&entry_digest(), &[&m])));
}

// #format-and-key-naming, #digest-format: manifest `extensions` keys and the manifest's digests
// are checked at their manifest-relative pointers
#[test]
fn manifest_extension_keys_and_digests_are_checked() {
    let m = sub(&format!(
        r#""extensions":{{"bad key":1}},
        "attestations":[{{"type":"t","uri":"u","digest":"sha256:ab"}}],
        "provenance":[{{"relation":"r","sourceId":"s","sourceDigest":"nope"}}],
        "subject":{},"signature":{}"#,
        subject(ENTRY_ID, ENTRY_TYPE, "md5:abcd", None),
        good_signature(C)
    ));
    let r = vm(&m);
    let f = found(&r);
    for (code, path) in [
        ("extension-key-invalid", "/extensions/bad key"),
        ("digest-weak", "/attestations/0/digest"),
        ("digest-invalid", "/provenance/0/sourceDigest"),
        ("digest-weak", "/subject/digest"),
    ] {
        assert!(
            f.iter()
                .any(|(c, s, p)| c == code && *s == Error && p == path),
            "{code} at {path} in {f:?}"
        );
    }
}

// Agreement law: `validate` reports, under each manifest, what `validate_manifest` reports
// for it, apart from binding diagnostics
#[test]
fn validate_reports_validate_manifest_diagnostics_for_every_corpus_manifest() {
    let manifests = [
        manifest(C, ""),
        manifest("a:b#c", &format!(",{SUBSTANCE}")),
        sub(&format!(
            r#""signature":{}"#,
            signature("x", "", "yesterday", None)
        )),
        sub(&format!(
            r#""subject":{},"signature":{}"#,
            matching_subject(),
            signature(
                "did:web:other.example",
                "p",
                "2026-09-12T10:00:00Z",
                Some("2026-09-12T10:00:00Z")
            )
        )),
        sub(r#""extensions":{"bad key":1}"#),
    ];
    for m in &manifests {
        let own = found(&vm(m));
        assert!(!own.is_empty() || m.contains("subject"), "{m}");
        let r = report(&bound_doc(&entry_digest(), &[m]));
        let prefix = "/entries/0/trustManifests/0";
        let mut got = under(&r, prefix);
        got.retain(|(c, _, _)| c != "subject-mismatch");
        assert_eq!(got, own, "{m}");
    }
    // A fixture manifest from the spec is clean standalone too.
    let spec = ok(std::str::from_utf8(
        &std::fs::read(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "tests/fixtures/spec/blocks/16-example-multi-artifact-catalog-with-nested-catalog.json",
        ))
        .unwrap(),
    )
    .unwrap());
    for e in &spec.entries {
        for m in e.trust_manifests.iter().flatten() {
            let r = validate_manifest(m);
            assert!(!r.has_errors(), "{:?}", found(&r));
        }
    }
}

// a manifest inside a signed catalog root is strict even when it is unsigned, so a
// non-finite number in it is an error there and a warning in an unsigned catalog
#[test]
fn unsigned_manifest_in_a_signed_catalog_is_strict() {
    let m = sub(r#""attestations":[{"type":"t","uri":"u","x":1e400}]"#);
    let unsigned = report(&bound_doc("", &[&m]));
    let f = found(&unsigned);
    assert_eq!(f.len(), 1, "{f:?}");
    assert_eq!(f[0].1, Warning);
    assert_eq!(f[0].0, "number-not-i-json");

    let signed_doc = bound_doc("", &[&m]).replacen(
        r#"{"specVersion":"1.0","#,
        &format!(
            r#"{{"specVersion":"1.0","signature":{},"#,
            good_signature("did:web:example.com")
        ),
        1,
    );
    let signed = found(&report(&signed_doc));
    assert_eq!(signed.len(), 1, "{signed:?}");
    assert_eq!(
        (signed[0].0.as_str(), signed[0].1),
        (f[0].0.as_str(), Error)
    );
    assert_eq!(signed[0].2, f[0].2);

    // A signed manifest is strict on its own payload, standalone.
    let own = sub(&format!(
        r#""subject":{},"signature":{},"attestations":[{{"type":"t","uri":"u","x":1e400}}]"#,
        matching_subject(),
        good_signature(C)
    ));
    assert_only(&vm(&own), &f[0].0, Error, "/attestations/0/x");
}
