mod common;

use common::*;
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::ArtifactSource;
use turul_ai_catalog::parse::ParseErrorKind::*;

fn data_doc(data: &str) -> String {
    doc_with_entry_body(&format!(r#""identifier":"i","type":"t","data":{data}"#))
}

fn data_of(data: &str) -> Json {
    match ok(&data_doc(data)).entries.remove(0).source {
        ArtifactSource::Data(j) => j,
        other => panic!("expected data, got {other:?}"),
    }
}

fn array_items(j: Json) -> Vec<Json> {
    match j {
        Json::Array(v) => v,
        other => panic!("expected array, got {other:?}"),
    }
}

fn lexeme(j: &Json) -> &str {
    match j {
        Json::Number(n) => n.as_str(),
        other => panic!("expected number, got {other:?}"),
    }
}

// RFC 8259 section 6: lexemes are kept byte-for-byte, nothing is rounded
#[test]
fn number_lexemes_are_kept_exactly() {
    let items = array_items(data_of(
        "[18446744073709551617, 1234567890123456789012345.5, 1.0, 1E+2, -0, 0.1000000000000000055511151231257827]",
    ));
    let got: Vec<&str> = items.iter().map(lexeme).collect();
    assert_eq!(
        got,
        [
            "18446744073709551617",
            "1234567890123456789012345.5",
            "1.0",
            "1E+2",
            "-0",
            "0.1000000000000000055511151231257827"
        ]
    );
    // Also inside an unrecognised member.
    let c = ok(&doc_with_entry_extra(r#""big":18446744073709551617"#));
    assert_eq!(
        lexeme(c.entries[0].unrecognized.get("big").unwrap()),
        "18446744073709551617"
    );
}

// Numbers are equal when they denote the same exact decimal value
#[test]
fn numbers_compare_by_decimal_value() {
    let items = array_items(data_of(
        "[1.0, 1, 1e0, 10e-1, -0, 0, 0.0, 2, 1.0000000000000000000001, 100, 1e2]",
    ));
    assert_eq!(items[0], items[1]);
    assert_eq!(items[1], items[2]);
    assert_eq!(items[2], items[3]);
    assert_eq!(items[4], items[5]);
    assert_eq!(items[5], items[6]);
    assert_ne!(items[1], items[7]);
    assert_ne!(items[0], items[8]);
    assert_eq!(items[9], items[10]);

    // Exponents beyond any fixed-width integer compare exactly and never overflow.
    let max = "170141183460469231731687303715884105727"; // i128::MAX
    let nines = "9".repeat(45);
    let big = format!("1{}", "0".repeat(40));
    let big_less_one = "9".repeat(40);
    let items = array_items(data_of(&format!(
        "[1e{max}, 1e170141183460469231731687303715884105728, 10e{max}, 1, \
         1.00e-{nines}, 1e{nines}, 1e{nines}0, 1e{big}, 10e{big_less_one}, \
         0.1e-{big_less_one}, 1e-{big}, 0e{nines}, 0, -1e{nines}, 1e{nines}]"
    )));
    assert_ne!(items[0], items[1]);
    assert_eq!(items[2], items[1]);
    assert_ne!(items[2], items[3]);
    assert_ne!(items[4], items[3]);
    assert_ne!(items[5], items[6]);
    assert_eq!(items[7], items[8]);
    assert_eq!(items[9], items[10]);
    assert_eq!(items[11], items[12]);
    assert_ne!(items[13], items[14]);
}

// RFC 8259 section 6: number grammar
#[test]
fn non_rfc_8259_numbers_are_syntax_errors() {
    for bad in [
        "01",
        "1.",
        ".5",
        "+1",
        "-",
        "1e",
        "1e+",
        "NaN",
        "Infinity",
        "-Infinity",
        "0x10",
        "1_0",
        "--1",
        "00",
        "-01",
    ] {
        let e = err(&data_doc(&format!("[{bad}]")));
        assert!(matches!(e.kind, Syntax), "{bad}: {:?}", e.kind);
        assert_eq!(e.pointer.to_string(), "/entries/0/data/0", "{bad}");
    }
}

// RFC 8259 section 2: only four whitespace characters; nothing after the value
#[test]
fn trailing_content_is_rejected() {
    let good = r#"{"specVersion":"1.0","entries":[]}"#;
    for tail in ["x", "{}", " 1", ",", "\u{0}"] {
        let e = err(&format!("{good}{tail}"));
        assert!(matches!(e.kind, Syntax), "{tail:?}: {:?}", e.kind);
        assert_eq!(e.pointer.to_string(), "", "{tail:?}");
    }
    ok(&format!(" \t\r\n{good} \t\r\n"));
    for ws in ["\u{b}", "\u{c}", "\u{a0}", "\u{2028}", "\u{feff}"] {
        let e = err(&format!("{ws}{good}"));
        assert!(matches!(e.kind, Syntax), "{ws:?}: {:?}", e.kind);
        let e = err(&format!("{good}{ws}"));
        assert!(matches!(e.kind, Syntax), "{ws:?}: {:?}", e.kind);
    }
    assert!(matches!(err("").kind, Syntax));
    assert!(matches!(err("   ").kind, Syntax));
}

// RFC 8259 section 8.1: a leading byte order mark is refused (strict reading)
#[test]
fn byte_order_mark_is_rejected() {
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(br#"{"specVersion":"1.0","entries":[]}"#);
    let e = err_bytes(&bytes);
    assert!(matches!(e.kind, Syntax), "{:?}", e.kind);
    assert_eq!(e.pointer.to_string(), "");
}

// RFC 8259 section 8.1: JSON text is UTF-8
#[test]
fn invalid_utf8_is_rejected() {
    for bad in [
        &[0xFFu8][..],
        &[0xC0, 0x80],
        &[0xED, 0xA0, 0x80],
        &[0xF8, 0x88, 0x80, 0x80, 0x80],
        &[0xC3],
    ] {
        let mut bytes = br#"{"specVersion":"1.0","entries":[],"x":""#.to_vec();
        bytes.extend_from_slice(bad);
        bytes.extend_from_slice(br#""}"#);
        let e = err_bytes(&bytes);
        assert!(matches!(e.kind, InvalidUtf8), "{bad:?}: {:?}", e.kind);
        assert_eq!(e.pointer.to_string(), "/x", "{bad:?}");
    }
    // Valid multi-byte UTF-8 is accepted.
    ok(r#"{"specVersion":"1.0","entries":[],"x":"é€😀"}"#);
}

fn nested(depth: usize) -> String {
    format!(
        r#"{{"specVersion":"1.0","entries":[],"x":{}{}}}"#,
        "[".repeat(depth),
        "]".repeat(depth)
    )
}

// RFC 8259 section 9: at most 128 nested containers; the root object is the first
#[test]
fn nesting_beyond_128_is_depth_limit() {
    ok(&nested(127));
    let e = err(&nested(128));
    assert!(matches!(e.kind, DepthLimit), "{:?}", e.kind);
    assert!(e.pointer.to_string().starts_with("/x"), "{}", e.pointer);
    // Object nesting counts alike.
    let objs = format!(
        r#"{{"specVersion":"1.0","entries":[],"x":{}1{}}}"#,
        r#"{"a":"#.repeat(128),
        "}".repeat(128)
    );
    let e = err(&objs);
    assert!(matches!(e.kind, DepthLimit), "{:?}", e.kind);
    assert!(e.pointer.to_string().starts_with("/x/a"), "{}", e.pointer);
    // An adversarial document returns an error instead of overflowing the stack.
    let huge = format!("{}{}", "[".repeat(200_000), "]".repeat(200_000));
    assert!(matches!(err(&huge).kind, DepthLimit));
}

// RFC 8259 section 4: an object is an unordered collection
#[test]
fn object_member_order_is_not_significant() {
    let a = ok(
        r#"{"specVersion":"1.0","entries":[{"identifier":"i","type":"t","data":{"x":1,"y":[{"p":1,"q":2}]},"zz":1,"aa":2}],"host":{"displayName":"H","identifier":"h"}}"#,
    );
    let b = ok(
        r#"{"host":{"identifier":"h","displayName":"H"},"entries":[{"aa":2,"zz":1,"data":{"y":[{"q":2,"p":1}],"x":1},"type":"t","identifier":"i"}],"specVersion":"1.0"}"#,
    );
    assert_eq!(a, b);
}
