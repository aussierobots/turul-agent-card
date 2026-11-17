mod common;

use common::*;
use turul_ai_catalog::parse::ParseErrorKind::*;

// #top-level-structure, #media-type-registration-application-ai-catalog-json
#[test]
fn missing_required_catalog_member_is_parse_error() {
    assert_err!(r#"{"specVersion":"1.0"}"#, MissingMember, "/entries");
    // The version gate runs first: an absent specVersion is a gate failure.
    assert_err!(r#"{"entries":[]}"#, SpecVersionInvalid, "/specVersion");
}

// #top-level-structure: `entries` "MAY be empty"
#[test]
fn empty_entries_array_parses() {
    assert!(
        ok(r#"{"specVersion":"1.0","entries":[]}"#)
            .entries
            .is_empty()
    );
}

// #top-level-structure: host, extensions, signature, additionalSignatures are OPTIONAL
#[test]
fn optional_catalog_members_may_be_absent_or_present() {
    let bare = ok(r#"{"specVersion":"1.0","entries":[]}"#);
    assert!(bare.host.is_none());
    assert!(bare.extensions.is_none());
    assert!(bare.signature.is_none());
    assert!(bare.additional_signatures.is_none());

    let full = ok(r#"{"specVersion":"1.0","entries":[],
        "host":{"displayName":"H"},
        "extensions":{"https://example.com/x":1},
        "signature":{"signer":"did:web:example.com","profile":"did-web-v1","issuedAt":"2026-09-12T10:00:00Z","jws":"a..b"},
        "additionalSignatures":[]}"#);
    assert!(full.host.is_some());
    assert!(full.extensions.is_some());
    assert!(full.signature.is_some());
    assert!(full.additional_signatures.is_some());
}

// #host-info: Host Info "MUST contain" displayName
#[test]
fn missing_host_display_name_is_parse_error() {
    assert_err!(
        r#"{"specVersion":"1.0","host":{},"entries":[]}"#,
        MissingMember,
        "/host/displayName"
    );
}

// #host-info: identifier, documentationUrl, logoUrl OPTIONAL
#[test]
fn optional_host_members_may_be_absent_or_present() {
    let bare = ok(r#"{"specVersion":"1.0","host":{"displayName":"H"},"entries":[]}"#);
    let h = bare.host.unwrap();
    assert_eq!(h.display_name, "H");
    assert!(h.identifier.is_none() && h.documentation_url.is_none() && h.logo_url.is_none());

    let full = ok(
        r#"{"specVersion":"1.0","host":{"displayName":"H","identifier":"did:web:h.example",
        "documentationUrl":"https://h.example/docs","logoUrl":"https://h.example/l.svg"},"entries":[]}"#,
    );
    let h = full.host.unwrap();
    assert_eq!(
        h.identifier.as_ref().map(|i| i.as_str()),
        Some("did:web:h.example")
    );
    assert_eq!(
        h.documentation_url.as_deref(),
        Some("https://h.example/docs")
    );
    assert_eq!(h.logo_url.as_deref(), Some("https://h.example/l.svg"));
}

// #catalog-entry: entry "MUST contain" identifier and type
#[test]
fn missing_required_entry_member_is_parse_error() {
    assert_err!(
        &doc_with_entry_body(r#""type":"t/x","url":"https://example.com/a""#),
        MissingMember,
        "/entries/0/identifier"
    );
    assert_err!(
        &doc_with_entry_body(r#""identifier":"i","url":"https://example.com/a""#),
        MissingMember,
        "/entries/0/type"
    );
    // Nothing is fabricated: the second entry's pointer names its own index.
    assert_err!(
        &format!(r#"{{"specVersion":"1.0","entries":[{ENTRY},{{"type":"t/x","url":"u"}}]}}"#),
        MissingMember,
        "/entries/1/identifier"
    );
}

// #catalog-entry, #optional-members: optional entry members, `description`, Trust Manifest OPTIONAL
#[test]
fn optional_entry_members_may_be_absent_or_present() {
    let bare = ok(&format!(r#"{{"specVersion":"1.0","entries":[{ENTRY}]}}"#));
    let e = &bare.entries[0];
    assert!(e.display_name.is_none() && e.version.is_none() && e.description.is_none());
    assert!(e.tags.is_none() && e.publisher.is_none() && e.digest.is_none());
    assert!(e.trust_manifests.is_none() && e.updated_at.is_none() && e.extensions.is_none());

    let full = ok(&doc_with_entry_extra(
        r#""displayName":"D","version":"1.2.3","description":"desc","tags":["a","b"],
        "publisher":{"identifier":"did:web:p.example","displayName":"P"},
        "digest":"sha256:00","trustManifests":[{"contributor":"did:web:c.example"}],
        "updatedAt":"2026-03-15T10:00:00Z","extensions":{"https://example.com/x":true}"#,
    ));
    let e = &full.entries[0];
    assert_eq!(e.display_name.as_deref(), Some("D"));
    assert_eq!(e.version.as_deref(), Some("1.2.3"));
    assert_eq!(e.description.as_deref(), Some("desc"));
    assert_eq!(
        e.tags.as_deref(),
        Some(&["a".to_string(), "b".to_string()][..])
    );
    assert!(e.publisher.is_some());
    assert_eq!(e.digest.as_deref(), Some("sha256:00"));
    assert_eq!(e.trust_manifests.as_ref().unwrap().len(), 1);
    assert_eq!(e.updated_at.as_deref(), Some("2026-03-15T10:00:00Z"));
    assert!(e.extensions.is_some());
}

// #trust-manifest: "Each manifest MUST contain `contributor`"
#[test]
fn missing_manifest_contributor_is_parse_error() {
    assert_err!(
        &doc_with_manifest(r#"{"attestations":[]}"#),
        MissingMember,
        "/entries/0/trustManifests/0/contributor"
    );
}

// #trust-manifest: manifest "MAY contain the optional members"
#[test]
fn optional_manifest_members_may_be_absent_or_present() {
    let bare = ok(&doc_with_manifest(r#"{"contributor":"did:web:c.example"}"#));
    let m = &bare.entries[0].trust_manifests.as_ref().unwrap()[0];
    assert_eq!(m.contributor, "did:web:c.example");
    assert!(m.subject.is_none() && m.signature.is_none() && m.additional_signatures.is_none());
    assert!(m.trust_schema.is_none() && m.attestations.is_none() && m.provenance.is_none());
    assert!(m.extensions.is_none());

    let full = ok(&doc_with_manifest(
        r#"{"contributor":"did:web:c.example",
        "subject":{"identifier":"i","type":"t/x","digest":"sha256:00"},
        "signature":{"signer":"s","profile":"p","issuedAt":"2026-09-12T10:00:00Z","jws":"a..b"},
        "additionalSignatures":[],
        "trustSchema":{"identifier":"i","version":"1"},
        "attestations":[],"provenance":[],"extensions":{}}"#,
    ));
    let m = &full.entries[0].trust_manifests.as_ref().unwrap()[0];
    assert!(m.subject.is_some() && m.signature.is_some() && m.additional_signatures.is_some());
    assert!(m.trust_schema.is_some() && m.extensions.is_some());
    assert_eq!(m.attestations.as_ref().unwrap().len(), 0);
    assert_eq!(m.provenance.as_ref().unwrap().len(), 0);
}

// #subject-object: Subject "MUST contain" identifier, type, digest
#[test]
fn missing_required_subject_member_is_parse_error() {
    let full = [
        ("identifier", "i"),
        ("type", "t/x"),
        ("digest", "sha256:00"),
    ];
    for (missing, _) in full {
        let members: Vec<String> = full
            .iter()
            .filter(|(k, _)| *k != missing)
            .map(|(k, v)| format!(r#""{k}":"{v}""#))
            .collect();
        let m = format!(
            r#"{{"contributor":"c","subject":{{{}}}}}"#,
            members.join(",")
        );
        assert_err!(
            &doc_with_manifest(&m),
            MissingMember,
            &format!("/entries/0/trustManifests/0/subject/{missing}")
        );
    }
    // `version` is the one optional member.
    let ok_doc = ok(&doc_with_manifest(
        r#"{"contributor":"c","subject":{"identifier":"i","type":"t/x","digest":"d","version":"2"}}"#,
    ));
    let s = ok_doc.entries[0].trust_manifests.as_ref().unwrap()[0]
        .subject
        .as_ref()
        .unwrap();
    assert_eq!(s.artifact_type.as_str(), "t/x");
    assert_eq!(s.version.as_deref(), Some("2"));
}

// #trust-schema-object: "MUST contain" identifier, version
#[test]
fn missing_required_trust_schema_member_is_parse_error() {
    assert_err!(
        &doc_with_manifest(r#"{"contributor":"c","trustSchema":{"version":"1"}}"#),
        MissingMember,
        "/entries/0/trustManifests/0/trustSchema/identifier"
    );
    assert_err!(
        &doc_with_manifest(r#"{"contributor":"c","trustSchema":{"identifier":"i"}}"#),
        MissingMember,
        "/entries/0/trustManifests/0/trustSchema/version"
    );
}

// #trust-schema-object: governanceUri, verificationMethods OPTIONAL
#[test]
fn optional_trust_schema_members_may_be_absent_or_present() {
    let bare = ok(&doc_with_manifest(
        r#"{"contributor":"c","trustSchema":{"identifier":"i","version":"1"}}"#,
    ));
    let t = bare.entries[0].trust_manifests.as_ref().unwrap()[0]
        .trust_schema
        .as_ref()
        .unwrap();
    assert!(t.governance_uri.is_none() && t.verification_methods.is_none());

    let full = ok(&doc_with_manifest(
        r#"{"contributor":"c","trustSchema":{"identifier":"i","version":"1",
        "governanceUri":"https://g.example","verificationMethods":["did:web:v.example#k"]}}"#,
    ));
    let t = full.entries[0].trust_manifests.as_ref().unwrap()[0]
        .trust_schema
        .as_ref()
        .unwrap();
    assert_eq!(t.governance_uri.as_deref(), Some("https://g.example"));
    assert_eq!(t.verification_methods.as_ref().unwrap().len(), 1);
}

// #attestation-object: "MUST contain" type, uri
#[test]
fn missing_required_attestation_member_is_parse_error() {
    assert_err!(
        &doc_with_manifest(r#"{"contributor":"c","attestations":[{"uri":"https://a.example"}]}"#),
        MissingMember,
        "/entries/0/trustManifests/0/attestations/0/type"
    );
    assert_err!(
        &doc_with_manifest(r#"{"contributor":"c","attestations":[{"type":"SOC2"}]}"#),
        MissingMember,
        "/entries/0/trustManifests/0/attestations/0/uri"
    );
}

// #attestation-object: digest, size, description OPTIONAL
#[test]
fn optional_attestation_members_may_be_absent_or_present() {
    let bare = ok(&doc_with_manifest(
        r#"{"contributor":"c","attestations":[{"type":"SOC2","uri":"https://a.example"}]}"#,
    ));
    let a = &bare.entries[0].trust_manifests.as_ref().unwrap()[0]
        .attestations
        .as_ref()
        .unwrap()[0];
    assert!(a.digest.is_none() && a.size.is_none() && a.description.is_none());
    assert_eq!(a.attestation_type, "SOC2");

    let full = ok(&doc_with_manifest(
        r#"{"contributor":"c","attestations":[{"type":"SOC2","uri":"https://a.example",
        "digest":"sha256:00","size":10,"description":"d"}]}"#,
    ));
    let a = &full.entries[0].trust_manifests.as_ref().unwrap()[0]
        .attestations
        .as_ref()
        .unwrap()[0];
    assert_eq!(a.digest.as_deref(), Some("sha256:00"));
    assert_eq!(a.size, Some(10));
    assert_eq!(a.description.as_deref(), Some("d"));
}

// #provenance-link-object: "MUST contain" relation, sourceId
#[test]
fn missing_required_provenance_link_member_is_parse_error() {
    assert_err!(
        &doc_with_manifest(r#"{"contributor":"c","provenance":[{"sourceId":"s"}]}"#),
        MissingMember,
        "/entries/0/trustManifests/0/provenance/0/relation"
    );
    assert_err!(
        &doc_with_manifest(r#"{"contributor":"c","provenance":[{"relation":"r"}]}"#),
        MissingMember,
        "/entries/0/trustManifests/0/provenance/0/sourceId"
    );
}

// #provenance-link-object, #provenance-statements: four optional members, incl. statementUri and signatureRef
#[test]
fn optional_provenance_link_members_may_be_absent_or_present() {
    let bare = ok(&doc_with_manifest(
        r#"{"contributor":"c","provenance":[{"relation":"r","sourceId":"s"}]}"#,
    ));
    let p = &bare.entries[0].trust_manifests.as_ref().unwrap()[0]
        .provenance
        .as_ref()
        .unwrap()[0];
    assert!(p.source_digest.is_none() && p.registry_uri.is_none());
    assert!(p.statement_uri.is_none() && p.signature_ref.is_none());

    let full = ok(&doc_with_manifest(
        r##"{"contributor":"c","provenance":[{"relation":"r","sourceId":"s","sourceDigest":"sha256:00",
        "registryUri":"https://r.example","statementUri":"https://s.example/st","signatureRef":"#sig"}]}"##,
    ));
    let p = &full.entries[0].trust_manifests.as_ref().unwrap()[0]
        .provenance
        .as_ref()
        .unwrap()[0];
    assert_eq!(p.source_digest.as_deref(), Some("sha256:00"));
    assert_eq!(p.registry_uri.as_deref(), Some("https://r.example"));
    assert_eq!(p.statement_uri.as_deref(), Some("https://s.example/st"));
    assert_eq!(p.signature_ref.as_deref(), Some("#sig"));
}

const SIG: &str = r#"{"signer":"did:web:example.com","profile":"did-web-v1","issuedAt":"2026-09-12T10:00:00Z","jws":"a..b"}"#;

// #signature-object: a Trust Manifest or catalog root MAY contain one `signature`
#[test]
fn signature_parses_on_catalog_root_and_manifest() {
    let root = ok(&format!(
        r#"{{"specVersion":"1.0","entries":[],"signature":{SIG}}}"#
    ));
    let s = root.signature.unwrap();
    assert_eq!(s.signer, "did:web:example.com");
    assert_eq!(s.profile, "did-web-v1");
    assert_eq!(s.issued_at, "2026-09-12T10:00:00Z");
    assert_eq!(s.jws, "a..b");

    let m = ok(&doc_with_manifest(&format!(
        r#"{{"contributor":"c","signature":{SIG}}}"#
    )));
    assert!(
        m.entries[0].trust_manifests.as_ref().unwrap()[0]
            .signature
            .is_some()
    );
}

// #signature-object: "MUST contain `signer`, `profile`, `issuedAt`, and `jws`"
#[test]
fn missing_required_signature_member_is_parse_error() {
    let members = [
        ("signer", "did:web:example.com"),
        ("profile", "did-web-v1"),
        ("issuedAt", "2026-09-12T10:00:00Z"),
        ("jws", "a..b"),
    ];
    for (missing, _) in members {
        let body: Vec<String> = members
            .iter()
            .filter(|(k, _)| *k != missing)
            .map(|(k, v)| format!(r#""{k}":"{v}""#))
            .collect();
        let sig = format!("{{{}}}", body.join(","));
        assert_err!(
            &format!(r#"{{"specVersion":"1.0","entries":[],"signature":{sig}}}"#),
            MissingMember,
            &format!("/signature/{missing}")
        );
        assert_err!(
            &doc_with_manifest(&format!(r#"{{"contributor":"c","signature":{sig}}}"#)),
            MissingMember,
            &format!("/entries/0/trustManifests/0/signature/{missing}")
        );
    }
}

// #signature-object: "MAY contain `expiresAt`", "An OPTIONAL RFC 3339 timestamp"
#[test]
fn signature_expires_at_may_be_absent_or_present() {
    let without = ok(&format!(
        r#"{{"specVersion":"1.0","entries":[],"signature":{SIG}}}"#
    ));
    assert!(without.signature.unwrap().expires_at.is_none());

    let with = ok(&format!(
        r#"{{"specVersion":"1.0","entries":[],"signature":{}}}"#,
        SIG.replace(r#""jws""#, r#""expiresAt":"2027-09-12T10:00:00Z","jws""#)
    ));
    assert_eq!(
        with.signature.unwrap().expires_at.as_deref(),
        Some("2027-09-12T10:00:00Z")
    );
}

// #publisher-object: "MUST contain" identifier, displayName
#[test]
fn missing_required_publisher_member_is_parse_error() {
    assert_err!(
        &doc_with_entry_extra(r#""publisher":{"displayName":"P"}"#),
        MissingMember,
        "/entries/0/publisher/identifier"
    );
    assert_err!(
        &doc_with_entry_extra(r#""publisher":{"identifier":"did:web:p.example"}"#),
        MissingMember,
        "/entries/0/publisher/displayName"
    );
}

// #publisher-object: identityType OPTIONAL
#[test]
fn optional_publisher_members_may_be_absent_or_present() {
    let bare = ok(&doc_with_entry_extra(
        r#""publisher":{"identifier":"i","displayName":"P"}"#,
    ));
    assert!(
        bare.entries[0]
            .publisher
            .as_ref()
            .unwrap()
            .identity_type
            .is_none()
    );
    let full = ok(&doc_with_entry_extra(
        r#""publisher":{"identifier":"i","displayName":"P","identityType":"did"}"#,
    ));
    assert_eq!(
        full.entries[0]
            .publisher
            .as_ref()
            .unwrap()
            .identity_type
            .as_deref(),
        Some("did")
    );
}

// #cddl-schema: a defined member of the wrong JSON type is refused, `null` included
#[test]
fn defined_member_of_wrong_type_is_parse_error() {
    assert_err!(r#"[]"#, WrongType, "");
    assert_err!(
        r#"{"specVersion":"1.0","entries":{}}"#,
        WrongType,
        "/entries"
    );
    assert_err!(
        r#"{"specVersion":"1.0","entries":null}"#,
        WrongType,
        "/entries"
    );
    assert_err!(
        r#"{"specVersion":"1.0","entries":[1]}"#,
        WrongType,
        "/entries/0"
    );
    assert_err!(
        r#"{"specVersion":"1.0","entries":[],"host":null}"#,
        WrongType,
        "/host"
    );
    assert_err!(
        r#"{"specVersion":"1.0","entries":[],"host":[]}"#,
        WrongType,
        "/host"
    );
    assert_err!(
        r#"{"specVersion":"1.0","entries":[],"host":{"displayName":null}}"#,
        WrongType,
        "/host/displayName"
    );
    assert_err!(
        r#"{"specVersion":"1.0","entries":[],"host":{"displayName":"H","logoUrl":5}}"#,
        WrongType,
        "/host/logoUrl"
    );
    assert_err!(
        r#"{"specVersion":"1.0","entries":[],"extensions":[]}"#,
        WrongType,
        "/extensions"
    );
    assert_err!(
        r#"{"specVersion":"1.0","entries":[],"signature":"s"}"#,
        WrongType,
        "/signature"
    );
    for (member, value) in [
        ("description", "null"),
        ("description", "1"),
        ("tags", r#""x""#),
        ("tags", "null"),
        ("publisher", "null"),
        ("trustManifests", "{}"),
        ("updatedAt", "123"),
        ("extensions", "null"),
    ] {
        assert_err!(
            &doc_with_entry_extra(&format!(r#""{member}":{value}"#)),
            WrongType,
            &format!("/entries/0/{member}")
        );
    }
    assert_err!(
        &doc_with_entry_extra(r#""tags":["a",1]"#),
        WrongType,
        "/entries/0/tags/1"
    );
    assert_err!(
        &doc_with_entry_body(r#""identifier":5,"type":"t","url":"u""#),
        WrongType,
        "/entries/0/identifier"
    );
    assert_err!(
        &doc_with_manifest(r#"{"contributor":null}"#),
        WrongType,
        "/entries/0/trustManifests/0/contributor"
    );
    assert_err!(
        &doc_with_manifest(
            r#"{"contributor":"c","attestations":[{"type":"t","uri":"u","digest":null}]}"#
        ),
        WrongType,
        "/entries/0/trustManifests/0/attestations/0/digest"
    );
    // `data` and `additionalSignatures` are CDDL `any`: null is a value.
    ok(&doc_with_entry_body(
        r#""identifier":"i","type":"t","data":null"#,
    ));
    ok(r#"{"specVersion":"1.0","entries":[],"additionalSignatures":null}"#);
}

// #catalog-entry, #host-info, #publisher-object, #trust-schema-object, #provenance-link-object,
// #attestation-object: opaque text, and tdate members are text at parse
#[test]
fn opaque_text_members_are_kept_verbatim() {
    let c = ok(
        r#"{"specVersion":"1.0","host":{"displayName":"H","identifier":"not a uri","logoUrl":"::not a url::"},
        "entries":[{"identifier":"not a urn","type":"not a media type","url":"%%",
        "publisher":{"identifier":"x","displayName":"P","identityType":"weird-kind"},
        "updatedAt":"yesterday",
        "trustManifests":[{"contributor":"anything",
          "trustSchema":{"identifier":"i","version":"v","governanceUri":"oci://example.com/gov"},
          "provenance":[{"relation":"r","sourceId":"s","registryUri":"oci://registry.example/repo:tag"}],
          "signature":{"signer":"s","profile":"p","issuedAt":"not-a-date","expiresAt":"never","jws":"x"}}]}]}"#,
    );
    let h = c.host.as_ref().unwrap();
    assert_eq!(h.identifier.as_ref().map(|i| i.as_str()), Some("not a uri"));
    assert_eq!(h.logo_url.as_deref(), Some("::not a url::"));
    let e = &c.entries[0];
    assert_eq!(e.updated_at.as_deref(), Some("yesterday"));
    assert_eq!(
        e.publisher.as_ref().unwrap().identity_type.as_deref(),
        Some("weird-kind")
    );
    let m = &e.trust_manifests.as_ref().unwrap()[0];
    assert_eq!(
        m.trust_schema.as_ref().unwrap().governance_uri.as_deref(),
        Some("oci://example.com/gov")
    );
    assert_eq!(
        m.provenance.as_ref().unwrap()[0].registry_uri.as_deref(),
        Some("oci://registry.example/repo:tag")
    );
    let s = m.signature.as_ref().unwrap();
    assert_eq!(s.issued_at, "not-a-date");
    assert_eq!(s.expires_at.as_deref(), Some("never"));
}

// #attestation-object: `size` is CDDL `uint`, 0..2^64-1 (RFC 8610 Appendix D)
#[test]
fn attestation_size_is_an_unsigned_64_bit_integer() {
    let doc = |size: &str| {
        doc_with_manifest(&format!(
            r#"{{"contributor":"c","attestations":[{{"type":"t","uri":"u","size":{size}}}]}}"#
        ))
    };
    let size_of = |size: &str| {
        ok(&doc(size)).entries[0].trust_manifests.as_ref().unwrap()[0]
            .attestations
            .as_ref()
            .unwrap()[0]
            .size
    };
    assert_eq!(size_of("0"), Some(0));
    assert_eq!(size_of("245760"), Some(245760));
    assert_eq!(size_of("18446744073709551615"), Some(u64::MAX));
    let ptr = "/entries/0/trustManifests/0/attestations/0/size";
    for bad in [
        "-1",
        "1.0",
        "1e3",
        "-0",
        "18446744073709551616",
        r#""5""#,
        "null",
        "true",
    ] {
        assert_err!(&doc(bad), WrongType, ptr);
    }
}

// #cddl-schema: optional arrays are `[* T]`; absent and empty are distinct values
#[test]
fn optional_arrays_absent_and_empty_are_distinct() {
    let absent = ok(&format!(r#"{{"specVersion":"1.0","entries":[{ENTRY}]}}"#));
    let empty = ok(&doc_with_entry_extra(r#""tags":[],"trustManifests":[]"#));
    assert!(absent.entries[0].tags.is_none());
    assert_eq!(empty.entries[0].tags.as_ref().map(Vec::len), Some(0));
    assert_eq!(
        empty.entries[0].trust_manifests.as_ref().map(Vec::len),
        Some(0)
    );
    assert_ne!(absent.entries[0], empty.entries[0]);
}
