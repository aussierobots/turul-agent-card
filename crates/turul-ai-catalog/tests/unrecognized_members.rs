mod common;

use common::*;
use turul_ai_catalog::canonical::to_canonical_vec;
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::ArtifactSource;
use turul_ai_catalog::parse::parse;

/// One document carrying an unrecognised member `zz` on every object kind the spec defines.
const EVERY_KIND: &str = r#"{
  "specVersion": "1.0",
  "zz": "catalog",
  "host": {"displayName": "H", "zz": "host"},
  "signature": {"signer": "s", "profile": "p", "issuedAt": "2026-09-12T10:00:00Z", "jws": "a..b", "zz": "signature"},
  "entries": [{
    "identifier": "i", "type": "t", "url": "u", "zz": "entry",
    "extensions": {"https://ai-catalog.org/extensions/policy-urls": {"privacyPolicyUrl": "https://e.example/p", "zz": "policy-urls"}},
    "publisher": {"identifier": "p", "displayName": "P", "zz": "publisher"},
    "trustManifests": [{
      "contributor": "c", "zz": "manifest",
      "subject": {"identifier": "i", "type": "t", "digest": "d", "zz": "subject"},
      "trustSchema": {"identifier": "i", "version": "1", "zz": "trust-schema"},
      "attestations": [{"type": "t", "uri": "u", "zz": "attestation"}],
      "provenance": [{"relation": "r", "sourceId": "s", "zz": "provenance"}]
    }]
  }]
}"#;

fn text(s: &str) -> Json {
    Json::String(s.to_string())
}

// #unrecognized-members, #cddl-schema, #consumer-behavior: kept on every object kind
#[test]
fn unknown_members_are_kept_on_every_object_kind() {
    let c = ok(EVERY_KIND);
    assert_eq!(c.unrecognized.get("zz"), Some(&text("catalog")));
    assert_eq!(
        c.host.as_ref().unwrap().unrecognized.get("zz"),
        Some(&text("host"))
    );
    assert_eq!(
        c.signature.as_ref().unwrap().unrecognized.get("zz"),
        Some(&text("signature"))
    );
    let e = &c.entries[0];
    assert_eq!(e.unrecognized.get("zz"), Some(&text("entry")));
    assert_eq!(
        e.publisher.as_ref().unwrap().unrecognized.get("zz"),
        Some(&text("publisher"))
    );
    let m = &e.trust_manifests.as_ref().unwrap()[0];
    assert_eq!(m.unrecognized.get("zz"), Some(&text("manifest")));
    assert_eq!(
        m.subject.as_ref().unwrap().unrecognized.get("zz"),
        Some(&text("subject"))
    );
    assert_eq!(
        m.trust_schema.as_ref().unwrap().unrecognized.get("zz"),
        Some(&text("trust-schema"))
    );
    assert_eq!(
        m.attestations.as_ref().unwrap()[0].unrecognized.get("zz"),
        Some(&text("attestation"))
    );
    assert_eq!(
        m.provenance.as_ref().unwrap()[0].unrecognized.get("zz"),
        Some(&text("provenance"))
    );

    // PolicyUrls is an `extensions` value: its unknown members stay inside that `Json` value.
    match e
        .extensions
        .as_ref()
        .unwrap()
        .get("https://ai-catalog.org/extensions/policy-urls")
    {
        Some(Json::Object(p)) => assert_eq!(p.get("zz"), Some(&text("policy-urls"))),
        other => panic!("{other:?}"),
    }

    // Defined members are not duplicated into the unrecognised map.
    assert_eq!(c.unrecognized.len(), 1);
    assert_eq!(e.unrecognized.len(), 1);
    assert!(m.signature.is_none());

    // A manifest's own signature keeps unknown members too.
    let ms = ok(&doc_with_manifest(
        r#"{"contributor":"c","signature":{"signer":"s","profile":"p","issuedAt":"t","jws":"j","zz":[1]}}"#,
    ));
    let sig = ms.entries[0].trust_manifests.as_ref().unwrap()[0]
        .signature
        .as_ref()
        .unwrap();
    assert_eq!(sig.unrecognized.len(), 1);
}

// #consumer-requirements, #compatibility-rules: "MUST NOT reject a document because it contains unrecognized members"
#[test]
fn document_with_unknown_members_is_not_rejected() {
    ok(EVERY_KIND);
    let c = ok(&doc_with_entry_extra(
        r#""a":null,"b":[],"c":{},"d":{"https://example.com/x":1},"Identifier":"case differs","url2":"x""#,
    ));
    assert_eq!(c.entries[0].unrecognized.len(), 6);
}

// #consumer-requirements: unrecognised members never change a defined member
#[test]
fn unknown_member_does_not_change_defined_members() {
    let plain = ok(&doc_with_entry_extra(
        r#""version":"1.0.0","description":"d""#,
    ));
    let noisy = ok(&doc_with_entry_extra(
        r#""version":"1.0.0","description":"d","Identifier":"other","url2":"https://other.example",
        "lifecycle":"deprecated","type2":"x","Type":"y","data2":{}"#,
    ));
    let (p, n) = (&plain.entries[0], &noisy.entries[0]);
    assert_eq!(p.identifier, n.identifier);
    assert_eq!(p.artifact_type, n.artifact_type);
    assert_eq!(p.source, n.source);
    assert_eq!(p.version, n.version);
    assert_eq!(p.description, n.description);
    assert_eq!(p.tags, n.tags);
    assert_eq!(n.unrecognized.len(), 6);
}

// #consumer-requirements: "SHOULD preserve unrecognized members, unchanged"
#[test]
fn unknown_member_values_are_kept_unchanged() {
    let values = [
        "null",
        "true",
        r#""text""#,
        "18446744073709551617",
        "1.0",
        "[1,[2.50,null],{}]",
        r#"{"nested":{"big":1234567890123456789012345,"s":"é😀"},"list":[false]}"#,
    ];
    for v in values {
        let c = ok(&doc_with_entry_extra(&format!(r#""k":{v}"#)));
        let kept = c.entries[0].unrecognized.get("k").expect("kept");
        // The same text read as `data` is the reference value.
        let reference = ok(&doc_with_entry_body(&format!(
            r#""identifier":"i","type":"t","data":{v}"#
        )));
        match &reference.entries[0].source {
            ArtifactSource::Data(j) => assert_eq!(kept, j, "{v}"),
            other => panic!("{other:?}"),
        }
    }
    // Lexemes survive, not just values.
    let c = ok(&doc_with_entry_extra(r#""k":[1.0,1E2]"#));
    match c.entries[0].unrecognized.get("k").unwrap() {
        Json::Array(items) => {
            let lex: Vec<&str> = items
                .iter()
                .map(|j| match j {
                    Json::Number(n) => n.as_str(),
                    o => panic!("{o:?}"),
                })
                .collect();
            assert_eq!(lex, ["1.0", "1E2"]);
        }
        o => panic!("{o:?}"),
    }
}

// #top-level-structure, #optional-members: additionalSignatures is accepted with any value and kept
#[test]
fn additional_signatures_of_any_type_is_accepted_and_kept() {
    for v in [
        "null",
        "true",
        "1",
        r#""s""#,
        "[]",
        r#"[{"x":1}]"#,
        "{}",
        r#"{"a":[1]}"#,
    ] {
        let root = ok(&format!(
            r#"{{"specVersion":"1.0","entries":[],"additionalSignatures":{v}}}"#
        ));
        let reference = ok(&doc_with_entry_body(&format!(
            r#""identifier":"i","type":"t","data":{v}"#
        )));
        let expected = match &reference.entries[0].source {
            ArtifactSource::Data(j) => j.clone(),
            o => panic!("{o:?}"),
        };
        assert_eq!(
            root.additional_signatures,
            Some(expected.clone()),
            "root {v}"
        );
        assert!(
            root.unrecognized.is_empty(),
            "not an unrecognised member: {v}"
        );

        let m = ok(&doc_with_manifest(&format!(
            r#"{{"contributor":"c","additionalSignatures":{v}}}"#
        )));
        let manifest = &m.entries[0].trust_manifests.as_ref().unwrap()[0];
        assert_eq!(
            manifest.additional_signatures,
            Some(expected),
            "manifest {v}"
        );
        assert!(manifest.unrecognized.is_empty(), "{v}");
    }
}

fn occurrences(bytes: Vec<u8>, needle: &str) -> usize {
    String::from_utf8(bytes).unwrap().matches(needle).count()
}

// #consumer-requirements, #cddl-schema: unrecognised members are written back unchanged on every object kind
#[test]
fn unknown_members_survive_serialisation_on_every_object_kind() {
    let c = ok(EVERY_KIND);
    let bytes = to_canonical_vec(&c).unwrap();
    // catalog, host, signature, entry, policy-urls, publisher, manifest, subject, trust schema, attestation, provenance
    assert_eq!(occurrences(bytes.clone(), r#""zz":"#), 11);
    assert_eq!(parse(&bytes).unwrap(), c);

    // A Trust Manifest serialises on its own, writing the unknown member of all six trust object kinds.
    let all_six = ok(&doc_with_manifest(
        r#"{"contributor":"c","zz":"manifest",
        "signature":{"signer":"s","profile":"p","issuedAt":"t","jws":"j","zz":"signature"},
        "subject":{"identifier":"i","type":"t","digest":"d","zz":"subject"},
        "trustSchema":{"identifier":"i","version":"1","zz":"trust-schema"},
        "attestations":[{"type":"t","uri":"u","zz":"attestation"}],
        "provenance":[{"relation":"r","sourceId":"s","zz":"provenance"}]}"#,
    ));
    let manifest = &all_six.entries[0].trust_manifests.as_ref().unwrap()[0];
    let bytes = manifest.to_json().to_canonical_vec().unwrap();
    assert_eq!(occurrences(bytes, r#""zz":"#), 6);

    // Values come back unchanged, nested numbers and nulls included.
    let c = ok(&doc_with_entry_extra(
        r#""k":{"a":[null,true,2.5],"b":{"c":"é😀"}}"#,
    ));
    let back = parse(&to_canonical_vec(&c).unwrap()).unwrap();
    assert_eq!(
        back.entries[0].unrecognized.get("k"),
        c.entries[0].unrecognized.get("k")
    );
}

// #top-level-structure, #optional-members: additionalSignatures is written back with its value unchanged
#[test]
fn additional_signatures_survive_serialisation() {
    for v in [
        "null",
        "true",
        "1",
        r#""s""#,
        "[]",
        r#"[{"x":1}]"#,
        "{}",
        r#"{"a":[1]}"#,
    ] {
        let root = ok(&format!(
            r#"{{"specVersion":"1.0","entries":[],"additionalSignatures":{v}}}"#
        ));
        let back = parse(&to_canonical_vec(&root).unwrap()).unwrap();
        assert_eq!(
            back.additional_signatures, root.additional_signatures,
            "root {v}"
        );
        assert!(back.additional_signatures.is_some(), "root {v}");
        match root.to_json() {
            Json::Object(m) => assert_eq!(
                m.get("additionalSignatures"),
                root.additional_signatures.as_ref(),
                "root {v}"
            ),
            o => panic!("{o:?}"),
        }

        let m = ok(&doc_with_manifest(&format!(
            r#"{{"contributor":"c","additionalSignatures":{v}}}"#
        )));
        let manifest = &m.entries[0].trust_manifests.as_ref().unwrap()[0];
        match manifest.to_json() {
            Json::Object(o) => assert_eq!(
                o.get("additionalSignatures"),
                manifest.additional_signatures.as_ref(),
                "manifest {v}"
            ),
            o => panic!("{o:?}"),
        }
        assert_eq!(
            parse(&to_canonical_vec(&m).unwrap()).unwrap(),
            m,
            "manifest {v}"
        );
    }
}
