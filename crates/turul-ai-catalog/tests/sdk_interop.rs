//! Interop with the official Rust SDK (ai-catalog, ai-catalog-validate, ai-catalog-trust):
//! every corpus file, both ways. The expected outcome of each file is held once, in `ROWS`.

mod common;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use common::spec_corpus::{Rule, SPEC_FIXTURES, wrapped};
use turul_ai_catalog::canonical::to_canonical_vec;
use turul_ai_catalog::model::ArtifactSource;
use turul_ai_catalog::parse::parse;
use turul_ai_catalog::validate::{Conformance, Severity, validate};

// ---- corpus ----

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn listed(dir: &str) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(fixtures_dir().join(dir))
        .unwrap()
        .map(|f| f.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.ends_with(".json"))
        .collect();
    v.sort();
    v
}

/// Every corpus input as (key, bytes): spec examples wrapped by their rule, then upstream and interop files.
fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    for (name, rule) in SPEC_FIXTURES {
        if *rule != Rule::Exempt {
            out.push((format!("spec/{name}"), wrapped(name)));
        }
    }
    for dir in ["upstream", "interop"] {
        for n in listed(dir) {
            let b = fs::read(fixtures_dir().join(dir).join(&n)).unwrap();
            out.push((format!("{dir}/{n}"), b));
        }
    }
    out
}

fn input(key: &str) -> Vec<u8> {
    corpus()
        .into_iter()
        .find(|(k, _)| k == key)
        .unwrap_or_else(|| panic!("{key} not in corpus"))
        .1
}

// ---- expected outcomes ----

/// What `parse` then `validate` do with the input.
#[derive(Debug)]
enum Ours {
    Accept,
    /// `parse` fails: error kind name and the JSON Pointer, when pinned.
    Parse(&'static str, Option<&'static str>),
    /// `parse` succeeds and `validate` reports these error codes.
    Invalid(&'static [&'static str]),
}

/// What `parse_slice` then `ai_catalog_validate::validate` do with the input.
#[derive(Debug)]
enum Theirs {
    Accept,
    Parse,
    /// These error codes (their `Code` variant names) are among the errors.
    Invalid(&'static [&'static str]),
}

struct Row {
    files: &'static [&'static str],
    ours: Ours,
    theirs: Theirs,
    /// "agree", or the RD id that decides the disagreement.
    decided_by: &'static str,
}

const ROWS: &[Row] = &[
    Row {
        files: &[
            "spec/examples/ai-catalog.json",
            "spec/blocks/01-top-level-structure.json",
            "spec/blocks/02-host-info.json",
            "spec/blocks/03-multi-version-entries.json",
            "spec/blocks/04-catalog-projection.json",
            "spec/blocks/05-optional-members.json",
            "spec/blocks/06-trust-schema-object.json",
            "spec/blocks/07-attestation-object.json",
            "spec/blocks/08-provenance-link-object.json",
            "spec/blocks/10-nested-catalog-entries.json",
            "spec/blocks/11-nested-catalog-entries.json",
            "spec/blocks/13-policy-urls-extension.json",
            "spec/blocks/14-unrecognized-members-examples.json",
            "spec/blocks/15-unrecognized-members-examples.json",
            "spec/blocks/17-example-hierarchical-catalog.json",
            "spec/blocks/18-example-dual-protocol-agent-mcp-a2a.json",
            "upstream/catalog.json",
            "upstream/discoverable.json",
            "upstream/future_member.json",
            "upstream/minimal.json",
            "upstream/nested_max.json",
            "upstream/spec-example.json",
            "upstream/trust_clean.json",
            "upstream/unsigned_trust.json",
            "interop/unknown-members-every-kind.json",
            "interop/empty-arrays.json",
            "interop/nested-signed-manifest.json",
        ],
        ours: Ours::Accept,
        theirs: Theirs::Accept,
        decided_by: "agree",
    },
    // #catalog-entry "MUST contain exactly one of"; the spec example lacks both
    Row {
        files: &["spec/blocks/12-format-and-key-naming.json"],
        ours: Ours::Parse("ArtifactSourceMissing", Some("/entries/0")),
        theirs: Theirs::Invalid(&["EntryArtifact"]),
        decided_by: "agree",
    },
    // placeholder digest `sha256:99998888...`
    Row {
        files: &["spec/blocks/16-example-multi-artifact-catalog-with-nested-catalog.json"],
        ours: Ours::Invalid(&["digest-invalid"]),
        theirs: Theirs::Invalid(&["DigestInvalid"]),
        decided_by: "agree",
    },
    Row {
        files: &["upstream/invalid.json"],
        ours: Ours::Parse("MissingMember", Some("/host/displayName")),
        theirs: Theirs::Invalid(&[]),
        decided_by: "agree",
    },
    // not a catalog: a map of manifest shapes
    Row {
        files: &["upstream/manifest_shapes.json"],
        ours: Ours::Parse("SpecVersionInvalid", None),
        theirs: Theirs::Invalid(&[]),
        decided_by: "agree",
    },
    Row {
        files: &["upstream/weak_signature.json"],
        ours: Ours::Invalid(&["digest-weak"]),
        theirs: Theirs::Invalid(&[]),
        decided_by: "agree",
    },
    // #nested-catalog-entries "A depth limit of 4 is RECOMMENDED"
    Row {
        files: &["upstream/nested_deep.json"],
        ours: Ours::Accept,
        theirs: Theirs::Invalid(&["NestedDepth"]),
        decided_by: "RD-08",
    },
    // #signed-payload-and-jws-construction, #verification-result
    Row {
        files: &["upstream/profile.json"],
        ours: Ours::Accept,
        theirs: Theirs::Invalid(&["SignatureB64", "SignatureMalformed", "SignatureForm"]),
        decided_by: "RD-15",
    },
    Row {
        files: &["interop/url-empty.json"],
        ours: Ours::Invalid(&["url-empty"]),
        theirs: Theirs::Invalid(&["EntryArtifact"]),
        decided_by: "agree",
    },
    Row {
        files: &["interop/lone-surrogate.json"],
        ours: Ours::Parse("LoneSurrogate", None),
        theirs: Theirs::Parse,
        decided_by: "agree",
    },
    // #catalog-entry: `data` is "any JSON value"
    Row {
        files: &["interop/data-null.json"],
        ours: Ours::Accept,
        theirs: Theirs::Invalid(&["EntryArtifact"]),
        decided_by: "RD-02",
    },
    // #top-level-structure
    Row {
        files: &["interop/missing-entries.json"],
        ours: Ours::Parse("MissingMember", Some("/entries")),
        theirs: Theirs::Accept,
        decided_by: "RD-01",
    },
    // #signed-payload-and-jws-construction
    Row {
        files: &["interop/duplicate-member.json"],
        ours: Ours::Parse("DuplicateMember", None),
        theirs: Theirs::Accept,
        decided_by: "RD-04",
    },
    // #version-format
    Row {
        files: &["interop/spec-version-plus.json"],
        ours: Ours::Parse("SpecVersionInvalid", Some("/specVersion")),
        theirs: Theirs::Accept,
        decided_by: "RD-14",
    },
    // #digest-format
    Row {
        files: &["interop/digest-uppercase.json"],
        ours: Ours::Invalid(&["digest-invalid"]),
        theirs: Theirs::Accept,
        decided_by: "RD-13",
    },
    // #contributor-identity
    Row {
        files: &["interop/contributor-not-rfc3986.json"],
        ours: Ours::Invalid(&["contributor-not-absolute-uri"]),
        theirs: Theirs::Accept,
        decided_by: "RD-12",
    },
];

fn row_of(key: &str) -> &'static Row {
    ROWS.iter()
        .find(|r| r.files.contains(&key))
        .unwrap_or_else(|| panic!("{key}: no outcome row"))
}

// ---- observation ----

fn variant_name(debug: String) -> String {
    debug
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect()
}

fn our_error_codes(b: &[u8]) -> Option<Vec<String>> {
    let c = parse(b).ok()?;
    Some(
        validate(&c)
            .diagnostics()
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .map(|d| d.code.as_str().to_string())
            .collect(),
    )
}

fn check_ours(key: &str, b: &[u8], want: &Ours) {
    match (parse(b), want) {
        (Err(e), Ours::Parse(kind, pointer)) => {
            assert_eq!(variant_name(format!("{:?}", e.kind)), *kind, "{key}");
            if let Some(p) = pointer {
                assert_eq!(e.pointer.to_string(), *p, "{key}");
            }
        }
        (Ok(_), Ours::Accept) => {
            assert_eq!(our_error_codes(b).unwrap(), Vec::<String>::new(), "{key}")
        }
        (Ok(_), Ours::Invalid(codes)) => {
            let got = our_error_codes(b).unwrap();
            assert!(!got.is_empty(), "{key}: validated clean");
            for c in *codes {
                assert!(got.iter().any(|g| g == c), "{key}: {c} not in {got:?}");
            }
        }
        (got, want) => panic!("{key}: ours {:?}, expected {want:?}", got.map(|_| "parsed")),
    }
}

fn check_theirs(key: &str, b: &[u8], want: &Theirs) {
    match (ai_catalog::parse_slice(b), want) {
        (Err(_), Theirs::Parse) => {}
        (Ok(t), Theirs::Accept) => {
            let v = ai_catalog_validate::validate(&t);
            assert!(v.is_valid, "{key}: {:?}", v.errors);
        }
        (Ok(t), Theirs::Invalid(codes)) => {
            let v = ai_catalog_validate::validate(&t);
            assert!(!v.is_valid, "{key}: accepted");
            let got: Vec<String> = v.errors.iter().map(|d| format!("{:?}", d.code)).collect();
            for c in *codes {
                assert!(got.iter().any(|g| g == c), "{key}: {c} not in {got:?}");
            }
        }
        (Err(_), Theirs::Invalid([])) => {}
        (got, want) => panic!(
            "{key}: theirs {:?}, expected {want:?}",
            got.map(|_| "parsed")
        ),
    }
}

fn ours_accepts(b: &[u8]) -> bool {
    matches!(our_error_codes(b), Some(v) if v.is_empty())
}

fn theirs_accepts(b: &[u8]) -> bool {
    ai_catalog::parse_slice(b)
        .map(|t| ai_catalog_validate::validate(&t).is_valid)
        .unwrap_or(false)
}

// ---- tests: the table ----

// Corpus: every vendored file has exactly one outcome row, and every row names a vendored file.
#[test]
fn every_corpus_file_has_an_outcome_row() {
    let on_disk: BTreeSet<String> = corpus().into_iter().map(|(k, _)| k).collect();
    let mut rowed = BTreeSet::new();
    for r in ROWS {
        for f in r.files {
            assert!(rowed.insert(f.to_string()), "{f} has two rows");
        }
    }
    assert_eq!(on_disk, rowed);
}

// Corpus: the interop fixtures of ours are all listed in SOURCE.md, and it lists nothing else.
#[test]
fn every_interop_fixture_is_listed() {
    let source = fs::read_to_string(fixtures_dir().join("interop/SOURCE.md")).unwrap();
    let files = listed("interop");
    assert_eq!(files.len(), 11);
    for f in &files {
        assert!(
            source.contains(&format!("`{f}`")),
            "{f} missing from SOURCE.md"
        );
        assert!(
            ROWS.iter()
                .any(|r| r.files.contains(&format!("interop/{f}").as_str())),
            "{f} missing from the outcome table"
        );
    }
    assert_eq!(
        source.lines().filter(|l| l.starts_with("| `")).count(),
        files.len()
    );
}

// #conformance-levels: ours and theirs agree on accept/reject, or the row names the deciding RD id
#[test]
fn accept_reject_agrees_or_is_a_recorded_divergence() {
    for (key, b) in corpus() {
        let row = row_of(&key);
        check_ours(&key, &b, &row.ours);
        check_theirs(&key, &b, &row.theirs);
        let agree = ours_accepts(&b) == theirs_accepts(&b);
        if row.decided_by == "agree" {
            assert!(agree, "{key}: disagreement without an RD id");
        } else {
            assert!(row.decided_by.starts_with("RD-"), "{key}");
            assert!(!agree, "{key}: {} is stale, outcomes agree", row.decided_by);
        }
    }
}

// ---- tests: laws ----

fn both_parse(b: &[u8]) -> bool {
    parse(b).is_ok() && ai_catalog::parse_slice(b).is_ok()
}

fn assert_ours_to_theirs(key: &str, b: &[u8]) {
    let c = parse(b).unwrap();
    let ours = to_canonical_vec(&c).unwrap_or_else(|e| panic!("{key}: {e:?}"));
    let t_back = ai_catalog::parse_slice(&ours).unwrap_or_else(|e| panic!("{key}: {e}"));
    assert_eq!(t_back, ai_catalog::parse_slice(b).unwrap(), "{key}");
}

fn assert_theirs_to_ours(key: &str, b: &[u8]) {
    let t = ai_catalog::parse_slice(b).unwrap();
    let text = ai_catalog::to_string(&t).unwrap();
    let back = parse(text.as_bytes()).unwrap_or_else(|e| panic!("{key}: {e:?}"));
    assert_eq!(back, parse(b).unwrap(), "{key}");
}

fn assert_canonical_equals_trust(key: &str, b: &[u8]) {
    let ours = to_canonical_vec(&parse(b).unwrap()).unwrap_or_else(|e| panic!("{key}: {e:?}"));
    let theirs = ai_catalog_trust::canonicalize(b).unwrap().into_bytes();
    assert_eq!(ours, theirs, "{key}");
}

// ours -> theirs: the canonical bytes parse in ai-catalog to the same value
#[test]
fn our_canonical_bytes_parse_equal_in_ai_catalog() {
    let mut n = 0;
    for (key, b) in corpus() {
        if both_parse(&b) {
            assert_ours_to_theirs(&key, &b);
            n += 1;
        }
    }
    assert!(n > 20);
}

// theirs -> ours, except the two recorded inequalities
#[test]
fn their_serialisation_parses_equal_in_ours() {
    for (key, b) in corpus() {
        if both_parse(&b)
            && !key.ends_with("interop/data-null.json")
            && !key.ends_with("interop/empty-arrays.json")
        {
            assert_theirs_to_ours(&key, &b);
        }
    }
}

// #signed-payload-and-jws-construction: our canonical bytes equal ai-catalog-trust canonicalize on the original bytes
#[test]
fn canonical_bytes_equal_ai_catalog_trust_canonicalize() {
    // Corpus files with a number that is not a finite double, which neither writes; empty at this revision.
    const NOT_I_JSON: &[&str] = &[];
    for (key, b) in corpus() {
        if parse(&b).is_ok() && !NOT_I_JSON.contains(&key.as_str()) {
            assert_canonical_equals_trust(&key, &b);
        }
    }
}

// RFC 8785 section 3.2.2.3: both write every finite number as the form of its nearest double
// (ai-catalog-trust through `serde_jcs`, which writes integers through `f64` too), also when that
// form denotes another value; neither writes a number that is not a finite double
#[test]
fn numbers_canonicalise_as_ai_catalog_trust_does() {
    let doc = |n: &str| {
        format!(
            r#"{{"specVersion":"1.0","entries":[{{"identifier":"a","type":"t","data":[{n}]}}]}}"#
        )
    };
    for n in [
        "1.0",
        "1e2",
        "0.1",
        "-0",
        "9007199254740992",
        "9007199254740993",
        "-9007199254740993",
        "9223372036854775808",
        "18446744073709551615",
        "18446744073709551617",
        "123456789012345678901234567890",
        "0.1234567890123456789012345",
        "333333333.33333329",
        "1e-400",
    ] {
        assert_canonical_equals_trust(n, doc(n).as_bytes());
    }
    let b = doc("9007199254740993");
    assert_eq!(
        ai_catalog_trust::canonicalize(b.as_bytes()).unwrap(),
        r#"{"entries":[{"data":[9007199254740992],"identifier":"a","type":"t"}],"specVersion":"1.0"}"#
    );
    let b = doc("1e400");
    assert!(to_canonical_vec(&parse(b.as_bytes()).unwrap()).is_err());
    assert!(ai_catalog_trust::canonicalize(b.as_bytes()).is_err());
}

// #unrecognized-members: an unknown member on all 11 object kinds crosses both ways
#[test]
fn unknown_members_on_every_kind_cross_both_ways() {
    let key = "interop/unknown-members-every-kind.json";
    let b = input(key);
    assert!(ours_accepts(&b) && theirs_accepts(&b));
    assert_ours_to_theirs(key, &b);
    assert_theirs_to_ours(key, &b);
    assert_canonical_equals_trust(key, &b);
    let canon = String::from_utf8(to_canonical_vec(&parse(&b).unwrap()).unwrap()).unwrap();
    assert_eq!(canon.matches("\"zz\":").count(), 12);
}

// #nested-catalog-entries: an inline nested catalog with a signed manifest crosses both ways
#[test]
fn nested_catalog_with_signed_manifest_crosses_both_ways() {
    let key = "interop/nested-signed-manifest.json";
    let b = input(key);
    assert!(ours_accepts(&b) && theirs_accepts(&b));
    assert_ours_to_theirs(key, &b);
    assert_theirs_to_ours(key, &b);
    assert_canonical_equals_trust(key, &b);
}

// ---- tests: pins ----

// #top-level-structure
#[test]
fn missing_required_member_is_refused_by_us_and_defaulted_by_them() {
    let b = input("interop/missing-entries.json");
    let e = parse(&b).unwrap_err();
    assert_eq!(variant_name(format!("{:?}", e.kind)), "MissingMember");
    assert_eq!(e.pointer.to_string(), "/entries");
    let t = ai_catalog::parse_slice(&b).unwrap();
    assert!(t.entries.is_empty());
}

// #catalog-entry: `data` is "any JSON value"
#[test]
fn null_data_is_an_artifact_source_and_their_writer_drops_it() {
    let b = input("interop/data-null.json");
    let c = parse(&b).unwrap();
    assert!(matches!(c.entries[0].source, ArtifactSource::Data(_)));
    assert!(ours_accepts(&b));
    let t = ai_catalog::parse_slice(&b).unwrap();
    assert!(!theirs_accepts(&b));
    let text = ai_catalog::to_string(&t).unwrap();
    assert!(!text.contains("\"data\""), "{text}");
    let e = parse(text.as_bytes()).unwrap_err();
    assert_eq!(
        variant_name(format!("{:?}", e.kind)),
        "ArtifactSourceMissing"
    );
}

// #signed-payload-and-jws-construction: JCS of the object as given; dropping an empty array changes the signed bytes
#[test]
fn empty_arrays_survive_our_round_trip_and_not_theirs() {
    let b = input("interop/empty-arrays.json");
    let ours = parse(&b).unwrap();
    let canon = String::from_utf8(to_canonical_vec(&ours).unwrap()).unwrap();
    for member in [
        "\"tags\":[]",
        "\"trustManifests\":[]",
        "\"attestations\":[]",
        "\"provenance\":[]",
        "\"verificationMethods\":[]",
    ] {
        assert!(canon.contains(member), "{member} lost: {canon}");
    }
    assert_eq!(parse(canon.as_bytes()).unwrap(), ours);
    let t = ai_catalog::parse_slice(&b).unwrap();
    let theirs_text = ai_catalog::to_string(&t).unwrap();
    assert_ne!(parse(theirs_text.as_bytes()).unwrap(), ours);
}

// #signed-payload-and-jws-construction
#[test]
fn duplicate_member_is_refused_by_us_and_last_wins_for_them() {
    let b = input("interop/duplicate-member.json");
    let e = parse(&b).unwrap_err();
    assert_eq!(variant_name(format!("{:?}", e.kind)), "DuplicateMember");
    let t = ai_catalog::parse_slice(&b).unwrap();
    let text = ai_catalog::to_string(&t).unwrap();
    assert!(
        text.contains("\"k\":2") && !text.contains("\"k\":1"),
        "{text}"
    );
}

// #conformance-levels: a document with errors is not Met where they report Minimal
#[test]
fn document_with_errors_is_not_met_where_they_report_minimal() {
    let b = input("upstream/weak_signature.json");
    let r = validate(&parse(&b).unwrap());
    assert_eq!(r.conformance(), Conformance::NotMet);
    let v = ai_catalog_validate::validate(&ai_catalog::parse_slice(&b).unwrap());
    assert_eq!(
        v.conformance_level,
        ai_catalog_validate::ConformanceLevel::Minimal
    );
    assert!(!v.is_valid);
}

// #nested-catalog-entries: "A depth limit of 4 is RECOMMENDED"
#[test]
fn over_depth_inline_nesting_is_a_warning_for_us_and_an_error_for_them() {
    let b = input("upstream/nested_deep.json");
    let r = validate(&parse(&b).unwrap());
    assert!(!r.has_errors());
    assert!(
        r.diagnostics().iter().any(|d| {
            d.code.as_str() == "nested-depth-limit" && d.severity == Severity::Warning
        })
    );
    assert_eq!(r.conformance(), Conformance::Undetermined);
    let v = ai_catalog_validate::validate(&ai_catalog::parse_slice(&b).unwrap());
    assert!(!v.is_valid);
    assert!(
        v.errors
            .iter()
            .any(|d| format!("{:?}", d.code) == "NestedDepth")
    );
}

// #contributor-identity: "contributor MUST be an absolute URI [RFC3986]"
#[test]
fn contributor_outside_rfc3986_is_refused_by_us_only() {
    let b = input("interop/contributor-not-rfc3986.json");
    assert!(!ours_accepts(&b));
    assert!(
        our_error_codes(&b)
            .unwrap()
            .contains(&"contributor-not-absolute-uri".to_string())
    );
    assert!(theirs_accepts(&b));
}

// #digest-format
#[test]
fn uppercase_digest_hex_is_refused_by_us_only() {
    let b = input("interop/digest-uppercase.json");
    assert!(
        our_error_codes(&b)
            .unwrap()
            .contains(&"digest-invalid".to_string())
    );
    assert!(theirs_accepts(&b));
}

// #version-format
#[test]
fn plus_signed_spec_version_is_refused_by_us_only() {
    let b = input("interop/spec-version-plus.json");
    let e = parse(&b).unwrap_err();
    assert_eq!(variant_name(format!("{:?}", e.kind)), "SpecVersionInvalid");
    assert!(theirs_accepts(&b));
}

// #signed-payload-and-jws-construction, #verification-result: JWS structure is never a document error for us
#[test]
fn jws_structure_is_never_a_document_error() {
    let b = input("upstream/profile.json");
    let c = parse(&b).unwrap();
    let r = validate(&c);
    assert!(!r.has_errors());
    assert!(r.diagnostics().iter().all(|d| {
        let p = d.path.to_string();
        !(p.ends_with("/signature/jws") && d.severity == Severity::Error)
    }));
    let v = ai_catalog_validate::validate(&ai_catalog::parse_slice(&b).unwrap());
    let got: Vec<String> = v.errors.iter().map(|d| format!("{:?}", d.code)).collect();
    for c in ["SignatureB64", "SignatureMalformed", "SignatureForm"] {
        assert!(got.iter().any(|g| g == c), "{c} not in {got:?}");
    }
}

// #the-did-web-signer-profile, #verification-result: did:web signer shape is never a document diagnostic for us
#[test]
fn did_web_signer_shape_is_never_a_document_diagnostic() {
    let b = input("upstream/profile.json");
    let r = validate(&parse(&b).unwrap());
    for d in r.diagnostics() {
        let p = d.path.to_string();
        assert!(
            !(p.ends_with("/signature/signer") || p.ends_with("/signature/jws")),
            "{} at {p}",
            d.code.as_str()
        );
    }
    let v = ai_catalog_validate::validate(&ai_catalog::parse_slice(&b).unwrap());
    let got: Vec<String> = v.warnings.iter().map(|d| format!("{:?}", d.code)).collect();
    for c in [
        "ProfileSigner",
        "ProfileAlgorithm",
        "ProfileKeyId",
        "ProfileHeader",
    ] {
        assert!(got.iter().any(|g| g == c), "{c} not in {got:?}");
    }
}

// The vendored upstream spec-example is stale against the spec's own example, so it is an upstream fixture only.
#[test]
fn stale_upstream_spec_example_is_an_upstream_fixture() {
    let upstream = fs::read(fixtures_dir().join("upstream/spec-example.json")).unwrap();
    let spec = fs::read(fixtures_dir().join("spec/examples/ai-catalog.json")).unwrap();
    assert_ne!(upstream, spec);
    assert!(
        SPEC_FIXTURES
            .iter()
            .all(|(n, _)| !n.ends_with("spec-example.json"))
    );
    assert!(
        ROWS.iter()
            .any(|r| r.files.contains(&"upstream/spec-example.json"))
    );
}

// #signed-payload-and-jws-construction (I-JSON): an unpaired surrogate escape is refused by both
#[test]
fn both_refuse_an_unpaired_surrogate_escape() {
    let b = input("interop/lone-surrogate.json");
    let e = parse(&b).unwrap_err();
    assert_eq!(variant_name(format!("{:?}", e.kind)), "LoneSurrogate");
    assert!(ai_catalog::parse_slice(&b).is_err());
}
