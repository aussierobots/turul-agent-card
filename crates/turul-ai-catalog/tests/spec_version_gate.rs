mod common;

use common::*;
use turul_ai_catalog::parse::ParseErrorKind::*;

// #compatibility-rules: an unsupported major SHOULD be rejected with an informative error
// #consumer-behavior: reject a higher major "with an informative error message"
#[test]
fn higher_major_is_rejected_with_informative_error() {
    let e = err(r#"{"specVersion":"2.0","entries":[]}"#);
    assert!(matches!(e.kind, UnsupportedMajor { .. }), "{:?}", e.kind);
    assert_eq!(e.pointer.to_string(), "/specVersion");
    let shown = e.to_string();
    assert!(shown.contains("2.0"), "names the value found: {shown}");
    assert!(shown.contains('1'), "names the supported major: {shown}");
    // A 2.0 document may have a different shape entirely.
    assert_err!(
        r#"{"specVersion":"2.0"}"#,
        UnsupportedMajor { .. },
        "/specVersion"
    );
    assert_err!(
        r#"{"specVersion":"2.0","entries":{"shape":"changed"}}"#,
        UnsupportedMajor { .. },
        "/specVersion"
    );
    // A major beyond any integer type still compares as a number.
    assert_err!(
        r#"{"specVersion":"99999999999999999999999999.0","entries":[]}"#,
        UnsupportedMajor { .. },
        "/specVersion"
    );
}

// #consumer-behavior: "Parse the `specVersion` field before processing the document"
#[test]
fn spec_version_is_gated_before_shape() {
    assert_err!(
        r#"{"specVersion":"2.0","entries":5,"host":7}"#,
        UnsupportedMajor { .. },
        "/specVersion"
    );
    assert_err!(r#"[]"#, WrongType, "");
    assert_err!(r#""x""#, WrongType, "");
    assert_err!(r#"{"entries":5}"#, SpecVersionInvalid, "/specVersion");
    assert_err!(
        r#"{"specVersion":1.0,"entries":[]}"#,
        SpecVersionInvalid,
        "/specVersion"
    );
    assert_err!(
        r#"{"specVersion":null,"entries":[]}"#,
        SpecVersionInvalid,
        "/specVersion"
    );
    assert_err!(
        r#"{"specVersion":["1.0"],"entries":[]}"#,
        SpecVersionInvalid,
        "/specVersion"
    );
}

// #consumer-behavior: accept a matching major "regardless of minor version differences"
#[test]
fn any_minor_of_major_one_is_accepted() {
    for v in ["1.0", "1.1", "1.2", "1.99", "1.100000"] {
        let c = ok(&format!(r#"{{"specVersion":"{v}","entries":[]}}"#));
        assert_eq!(c.spec_version, v);
    }
}

// #version-format: "Major and minor components are non-negative integers"
#[test]
fn malformed_spec_version_is_invalid() {
    for v in [
        "+1.0", "1", "1.0.0", "v1.0", " 1.0", "1.0 ", "", "1.", ".1", "-1.0", "1.-0", "1,0",
        "1.0\n", "１.０", "1.0a", "0x1.0",
    ] {
        let input = format!(
            r#"{{"specVersion":"{}","entries":[]}}"#,
            v.replace('\n', "\\n")
        );
        assert_err!(&input, SpecVersionInvalid, "/specVersion");
    }
}

// #version-format: numeric components, so leading zeros pass the gate
#[test]
fn leading_zero_spec_version_parses() {
    for v in ["01.0", "1.00", "001.001"] {
        let c = ok(&format!(r#"{{"specVersion":"{v}","entries":[]}}"#));
        assert_eq!(c.spec_version, v);
    }
}

// #compatibility-rules: major 0 is not major 1
#[test]
fn major_zero_is_unsupported() {
    assert_err!(
        r#"{"specVersion":"0.9","entries":[]}"#,
        UnsupportedMajor { .. },
        "/specVersion"
    );
    assert_err!(
        r#"{"specVersion":"00.1","entries":[]}"#,
        UnsupportedMajor { .. },
        "/specVersion"
    );
}
