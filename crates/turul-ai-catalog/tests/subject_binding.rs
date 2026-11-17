mod common;
#[path = "support/trust.rs"]
mod trust;

use common::*;
use trust::*;
use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
use turul_ai_catalog::model::{CatalogEntry, Subject};
use turul_ai_catalog::validate::Severity::*;

const C: &str = "did:web:example.com";
const M: &str = "/entries/0/trustManifests/0";

/// The entry of a one-entry document: the valid entry members, then `extra` (members with a leading comma).
fn entry_of(extra: &str) -> CatalogEntry {
    ok(&doc_with_entry_body(&format!("{ENTRY_BODY}{extra}"))).entries[0].clone()
}

fn subject_of(json: &str) -> Subject {
    manifest_of(&doc_with_manifest(&manifest(
        C,
        &format!(r#","subject":{json},{SUBSTANCE}"#),
    )))
    .subject
    .expect("subject")
}

/// An unsigned manifest carrying `subject`, in an entry with `entry_extra`.
fn unsigned(subject: &str) -> String {
    manifest(C, &format!(r#","subject":{subject},{SUBSTANCE}"#))
}

fn mismatches(r: &turul_ai_catalog::validate::Report) -> Vec<String> {
    let mut v: Vec<String> = found(r)
        .into_iter()
        .filter(|(c, s, _)| c == "subject-mismatch" && *s == Error)
        .map(|(_, _, p)| p)
        .collect();
    v.sort();
    v
}

// #subject-object: `subject.identifier` "MUST exactly equal the containing entry's `identifier`"
#[test]
fn identifier_mismatch_is_an_error() {
    let m = unsigned(&subject(
        "urn:air:example.com:fixture:b",
        ENTRY_TYPE,
        &digest(),
        None,
    ));
    assert_only(
        &report(&bound_doc(&entry_digest(), &[&m])),
        "subject-mismatch",
        Error,
        &format!("{M}/subject/identifier"),
    );
}

// #subject-object: `subject.type` "MUST exactly equal the containing entry's `type`"
#[test]
fn type_mismatch_is_an_error() {
    let m = unsigned(&subject(
        ENTRY_ID,
        "application/a2a-agent-card+json",
        &digest(),
        None,
    ));
    assert_only(
        &report(&bound_doc(&entry_digest(), &[&m])),
        "subject-mismatch",
        Error,
        &format!("{M}/subject/type"),
    );
}

// #subject-object: `subject.digest` "MUST exactly equal the containing entry's `digest`"
#[test]
fn digest_mismatch_is_an_error() {
    let other = format!("sha256:{}", "cd".repeat(32));
    let m = unsigned(&subject(ENTRY_ID, ENTRY_TYPE, &other, None));
    let path = format!("{M}/subject/digest");
    assert_only(
        &report(&bound_doc(&entry_digest(), &[&m])),
        "subject-mismatch",
        Error,
        &path,
    );
    // The subject is compared even when the entry has no digest: it does not bind.
    let m = unsigned(&matching_subject());
    assert_only(
        &report(&bound_doc("", &[&m])),
        "subject-mismatch",
        Error,
        &path,
    );
}

// #subject-object: the entry digest "MUST be present when an entry carries a signed Trust
// Manifest"; #entry-release-coverage: "The entry MUST contain `digest`"
#[test]
fn signed_manifest_on_entry_without_digest_is_an_error() {
    let m = manifest(
        C,
        &format!(
            r#","subject":{},"signature":{}"#,
            matching_subject(),
            good_signature(C)
        ),
    );
    let mut f = found(&report(&bound_doc("", &[&m])));
    f.sort_by(|a, b| a.0.cmp(&b.0));
    let mut want = vec![
        (
            "entry-digest-missing".to_string(),
            Error,
            "/entries/0/digest".to_string(),
        ),
        (
            "subject-mismatch".to_string(),
            Error,
            format!("{M}/subject/digest"),
        ),
    ];
    want.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(f, want);
}

// #subject-object: `subject.version` is "present exactly when `entry.version` is present" and
// equal; #entry-release-coverage: both absent, or both present and equal
#[test]
fn version_presence_and_value_must_match() {
    let bound = |entry_version: Option<&str>, subject_version: Option<&str>| {
        let extra = entry_version
            .map(|v| format!(r#","version":"{v}""#))
            .unwrap_or_default();
        let e = entry_of(&format!("{}{extra}", entry_digest()));
        subject_of(&subject(ENTRY_ID, ENTRY_TYPE, &digest(), subject_version)).binds(&e)
    };
    assert!(bound(None, None));
    assert!(bound(Some("1.2.0"), Some("1.2.0")));
    assert!(!bound(Some("1.2.0"), Some("1.2.1")));
    assert!(!bound(Some("1.2.0"), None));
    assert!(!bound(None, Some("1.2.0")));
    // `""` is a present version.
    assert!(bound(Some(""), Some("")));
    assert!(!bound(Some(""), None));
    assert!(!bound(None, Some("")));

    // validate reports each failing case at the subject's version pointer.
    let path = format!("{M}/subject/version");
    for (entry_version, subject_version) in [("1.0", Some("2.0")), ("1.0", None)] {
        let m = unsigned(&subject(ENTRY_ID, ENTRY_TYPE, &digest(), subject_version));
        let r = report(&bound_doc(
            &format!(r#"{},"version":"{entry_version}""#, entry_digest()),
            &[&m],
        ));
        assert_only(&r, "subject-mismatch", Error, &path);
    }
    let m = unsigned(&subject(ENTRY_ID, ENTRY_TYPE, &digest(), Some("1.0")));
    assert_only(
        &report(&bound_doc(&entry_digest(), &[&m])),
        "subject-mismatch",
        Error,
        &path,
    );
}

// #subject-object: a subject is checked whether or not the manifest is signed
#[test]
fn unsigned_manifest_subject_is_bound_too() {
    let m = unsigned(&subject(
        "urn:air:example.com:fixture:b",
        "x/y",
        &digest(),
        Some("1"),
    ));
    let r = report(&bound_doc(&entry_digest(), &[&m]));
    assert_eq!(
        mismatches(&r),
        vec![
            format!("{M}/subject/identifier"),
            format!("{M}/subject/type"),
            format!("{M}/subject/version"),
        ]
    );
    assert_eq!(found(&r).len(), 3);
}

// #entry-release-coverage: the signed subject binds the entry it was built from
#[test]
fn for_entry_binds_its_entry() {
    for version in [None, Some("1.2.0"), Some("")] {
        let v = version
            .map(|v| format!(r#","version":"{v}""#))
            .unwrap_or_default();
        let e = entry_of(&format!("{}{v}", entry_digest()));
        let s = Subject::for_entry(&e).expect("entry has a digest");
        assert!(s.binds(&e));
        assert_eq!(s.identifier, e.identifier);
        assert_eq!(s.artifact_type, e.artifact_type);
        assert_eq!(Some(&s.digest), e.digest.as_ref());
        assert_eq!(s.version, e.version);
        assert_eq!(s.version.as_deref(), version);
    }
}

// #entry-release-coverage: "The entry MUST contain `digest`": none, no subject
#[test]
fn entry_without_digest_has_no_subject() {
    assert!(Subject::for_entry(&entry_of("")).is_none());
    // A subject never binds an entry without a digest.
    let e = entry_of("");
    assert!(!subject_of(&matching_subject()).binds(&e));
}

// #entry-release-coverage: identifier, type, digest and version must each equal
#[test]
fn each_member_mismatch_breaks_binding() {
    let e = entry_of(&format!("{},\"version\":\"1\"", entry_digest()));
    let s = Subject::for_entry(&e).unwrap();
    assert!(s.binds(&e));
    let mut t = s.clone();
    t.identifier = EntryIdentifier::new("urn:air:example.com:fixture:b");
    assert!(!t.binds(&e));
    let mut t = s.clone();
    t.artifact_type = ArtifactType::new("text/plain");
    assert!(!t.binds(&e));
    let mut t = s.clone();
    t.digest = format!("sha256:{}", "cd".repeat(32));
    assert!(!t.binds(&e));
    let mut t = s.clone();
    t.version = Some("2".to_string());
    assert!(!t.binds(&e));
    let mut t = s;
    t.version = None;
    assert!(!t.binds(&e));
}

// #entry-release-coverage: comparisons "MUST be case-sensitive, without URI normalization";
// the subject digest is compared as text, and empty members are compared, not rejected
#[test]
fn binding_compares_exact_strings() {
    let e = entry_of(&entry_digest());
    let s = Subject::for_entry(&e).unwrap();

    let mut t = s.clone();
    t.identifier = EntryIdentifier::new(ENTRY_ID.to_uppercase());
    assert!(!t.binds(&e));
    let mut t = s.clone();
    t.identifier = EntryIdentifier::new(ENTRY_ID.replace("fixture:a", "fixture:%61"));
    assert!(!t.binds(&e));
    let mut t = s.clone();
    t.artifact_type = ArtifactType::new(ENTRY_TYPE.to_uppercase());
    assert!(!t.binds(&e));
    let mut t = s.clone();
    t.digest = s.digest.replacen("sha256", "SHA256", 1);
    assert!(!t.binds(&e));
    // Both sides empty: equal, so bound.
    let blank = entry_of(r#","digest":"""#);
    let mut t = Subject::for_entry(&blank).unwrap();
    assert!(t.binds(&blank));
    t.digest = "x".to_string();
    assert!(!t.binds(&blank));
}

// #entry-release-coverage: "MUST NOT combine fields from different manifests"
#[test]
fn fields_from_two_manifests_never_combine() {
    let other_id = "urn:air:example.com:fixture:b";
    let other_digest = format!("sha256:{}", "cd".repeat(32));
    // A matches identifier and type but not digest; B matches digest but not identifier.
    let a = unsigned(&subject(ENTRY_ID, ENTRY_TYPE, &other_digest, None));
    let b = unsigned(&subject(other_id, ENTRY_TYPE, &digest(), None));
    let d = bound_doc(&entry_digest(), &[&a, &b]);
    let cat = ok(&d);
    let e = &cat.entries[0];
    for m in e.trust_manifests.as_ref().unwrap() {
        assert!(!m.subject.as_ref().unwrap().binds(e));
    }
    let r = report(&d);
    assert_eq!(
        found(&r),
        vec![
            (
                "subject-mismatch".to_string(),
                Error,
                format!("{M}/subject/digest")
            ),
            (
                "subject-mismatch".to_string(),
                Error,
                "/entries/0/trustManifests/1/subject/identifier".to_string()
            ),
        ]
    );
}
