//! The RFC 8785 number rule: every number is written as the ECMAScript form of its nearest
//! double, even when that form denotes another decimal value; only a number whose nearest
//! double is not finite is refused, at its pointer.

use std::fs;
use std::path::PathBuf;

use turul_ai_catalog::canonical::CanonicalError;
use turul_ai_catalog::json::read;

fn golden(name: &str) -> Vec<u8> {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/canonical")
        .join(name);
    fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn canonical(input: &str) -> String {
    let v = read(input.as_bytes()).unwrap_or_else(|e| panic!("{input}: {e}"));
    String::from_utf8(
        v.to_canonical_vec()
            .unwrap_or_else(|e| panic!("{input}: {e:?}")),
    )
    .unwrap()
}

fn refusal(input: &str) -> String {
    let v = read(input.as_bytes()).unwrap_or_else(|e| panic!("{input}: {e}"));
    match v.to_canonical_vec() {
        Err(CanonicalError::NotIJson { pointer, .. }) => pointer.to_string(),
        other => panic!("expected NotIJson for {input}, got {other:?}"),
    }
}

// RFC 8785 section 3.2.2.3, Appendix B Table 1
#[test]
fn rfc_8785_appendix_b_numbers_serialise_as_tabulated() {
    let table = String::from_utf8(golden("rfc8785-appendix-b.tsv")).unwrap();
    let mut rows = 0;
    for line in table.lines() {
        let (hex, rep) = line.split_once('\t').expect("hex<TAB>representation");
        let bits = u64::from_str_radix(hex, 16).unwrap();
        // the copy itself is checked independently of the writer
        assert_eq!(
            f64::from_bits(bits),
            rep.parse::<f64>().unwrap(),
            "{hex} {rep}"
        );
        // the tabulated representation is a fixed point of the writer
        assert_eq!(canonical(rep), rep, "{hex}");
        // the double's exact decimal value, written in full, is written as the representation
        let exact = format!("{:.800e}", f64::from_bits(bits));
        assert_eq!(canonical(&exact), rep, "{hex}");
        rows += 1;
    }
    assert_eq!(rows, 24);
}

// RFC 8785 section 3.2.2.3: equal-value spellings give the shortest ECMAScript form
#[test]
fn equal_value_spellings_serialise_to_the_shortest_form() {
    for (lexeme, expected) in [
        ("1.0", "1"),
        ("1e0", "1"),
        ("1e2", "100"),
        ("1E+2", "100"),
        ("0.1", "0.1"),
        ("4.50", "4.5"),
        ("-0", "0"),
        ("0", "0"),
        ("1E30", "1e+30"),
        ("100000000000000000000000", "1e+23"),
        ("0.000001", "0.000001"),
        ("0.0000001", "1e-7"),
        ("1e21", "1e+21"),
        ("999999999999999900000", "999999999999999900000"),
        ("295147905179352830000", "295147905179352830000"),
        ("-1.5e3", "-1500"),
    ] {
        assert_eq!(
            canonical(&format!("[{lexeme}]")),
            format!("[{expected}]"),
            "{lexeme}"
        );
    }
}

// RFC 8785 section 3.2.2.3: a number is written as the ECMAScript form of its nearest double,
// also when that form denotes another decimal value. The expected forms are from
// `JSON.stringify(JSON.parse("[" + lexeme + "]"))` in Node.js 26, an independent producer.
#[test]
fn numbers_are_written_as_the_form_of_their_nearest_double() {
    for (lexeme, jcs_form) in [
        ("9007199254740993", "9007199254740992"),
        ("18446744073709551617", "18446744073709552000"),
        ("18446744073709551615", "18446744073709552000"),
        ("9223372036854775808", "9223372036854776000"),
        ("123456789012345678901234567890", "1.2345678901234568e+29"),
        ("0.1234567890123456789012345", "0.12345678901234568"),
        ("0.1000000000000000055511151231257827", "0.1"),
        ("0.10000000000000001", "0.1"),
        ("295147905179352825856", "295147905179352830000"),
        ("1424953923781206.25", "1424953923781206.2"),
        ("333333333.33333329", "333333333.3333333"),
        ("1e-400", "0"),
        ("-1e-400", "0"),
    ] {
        assert_eq!(
            canonical(&format!(r#"{{"n":[1,{lexeme}]}}"#)),
            format!(r#"{{"n":[1,{jcs_form}]}}"#),
            "{lexeme}"
        );
        // the form is a fixed point
        let form = format!("[{jcs_form}]");
        assert_eq!(canonical(&form), form, "{jcs_form}");
    }
}

// RFC 8785 section 3.2.2.3: a value that is not a finite double is an error, at its pointer
#[test]
fn non_finite_numbers_are_refused_with_their_pointer() {
    for lexeme in ["1e400", "-1e400", "1.8e308", "-1.8e308"] {
        assert_eq!(
            refusal(&format!(r#"{{"n":[1,{lexeme}]}}"#)),
            "/n/1",
            "{lexeme}"
        );
    }
}

// The writer walks in canonical order and stops at the first defect
#[test]
fn first_defect_in_canonical_order_is_reported() {
    assert_eq!(refusal(r#"{"b":1e400,"a":[2e400]}"#), "/a/0");
}

// #consumer-requirements: `1.0` in `data` is written `1`, and the re-parsed model compares equal
// because `Number` equality is by exact decimal value
#[test]
fn round_trip_keeps_number_values() {
    use turul_ai_catalog::parse::parse;
    let doc = br#"{"specVersion":"1.0","entries":[{"identifier":"i","type":"t","data":{"n":1.0,"m":1E2}}]}"#;
    let m = parse(doc).unwrap();
    let bytes = turul_ai_catalog::canonical::to_canonical_vec(&m).unwrap();
    let text = String::from_utf8(bytes.clone()).unwrap();
    assert!(text.contains(r#""data":{"m":100,"n":1}"#), "{text}");
    assert_eq!(parse(&bytes).unwrap(), m);
}

// #attestation-object `size` is a `uint` and the writer writes it as RFC 8785 writes any
// number. A `size` above 2^53 that a double does not hold is written as another integer, so the
// re-parsed ceiling differs (`2^53+1` becomes `2^53`) or is refused (`2^64-1` becomes
// `18446744073709552000`, beyond a `u64`); validation reports it as `number-inexact`.
#[test]
fn attestation_size_is_written_as_rfc_8785_writes_it() {
    use turul_ai_catalog::canonical::to_canonical_vec;
    use turul_ai_catalog::parse::parse;
    let doc = |n: &str| {
        format!(
            r#"{{"specVersion":"1.0","entries":[{{"identifier":"a","type":"t","data":null}},{{"identifier":"b","type":"t","url":"u","trustManifests":[{{"contributor":"c","attestations":[{{"type":"t","uri":"u","size":1}},{{"type":"t","uri":"u","size":{n}}}]}}]}}]}}"#
        )
    };
    let written = |n: &str| format!(r#"{{"size":{n},"type":"t","uri":"u"}}"#);
    // 2^53+1, 2^63 (a double holds it, and JCS writes it 9223372036854776000), 2^64-1
    for (n, jcs_form) in [
        ("9007199254740993", "9007199254740992"),
        ("9223372036854775808", "9223372036854776000"),
        ("18446744073709551615", "18446744073709552000"),
    ] {
        let m = parse(doc(n).as_bytes()).unwrap();
        let bytes = to_canonical_vec(&m).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains(&written(jcs_form)), "{n}: {text}");
        assert_eq!(m.to_json().to_canonical_vec().unwrap(), bytes, "{n}: law 3");
        let manifest = &m.entries[1].trust_manifests.as_ref().unwrap()[0];
        let manifest_text =
            String::from_utf8(manifest.to_json().to_canonical_vec().unwrap()).unwrap();
        assert!(manifest_text.contains(&written(jcs_form)), "{n}: manifest");
        assert_ne!(parse(&bytes).ok(), Some(m), "{n}: law 1 does not hold");
    }
    // JCS writes each of these as itself, so law 1 holds: 0, 2^53, 2^54
    for n in ["0", "245760", "9007199254740992", "18014398509481984"] {
        let m = parse(doc(n).as_bytes()).unwrap();
        let bytes = to_canonical_vec(&m).unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains(&written(n)), "{n}: {text}");
        assert_eq!(parse(&bytes).unwrap(), m, "{n}");
        assert_eq!(
            to_canonical_vec(&parse(&bytes).unwrap()).unwrap(),
            bytes,
            "{n}"
        );
    }
}

// #catalog-entry `data` may hold any JSON value, here an inline catalog. Its `size` is a plain
// `Json` number, written as RFC 8785 writes it.
#[test]
fn size_in_an_inline_catalog_in_data_is_written_as_rfc_8785_writes_it() {
    use turul_ai_catalog::canonical::to_canonical_vec;
    use turul_ai_catalog::parse::parse;
    for (n, jcs_form) in [
        ("9007199254740993", "9007199254740992"),
        ("18446744073709551615", "18446744073709552000"),
    ] {
        let doc = format!(
            r#"{{"specVersion":"1.0","entries":[{{"identifier":"outer","type":"application/ai-catalog+json","data":{{"specVersion":"1.0","entries":[{{"identifier":"inner","type":"t","url":"u","trustManifests":[{{"contributor":"c","attestations":[{{"type":"t","uri":"u","size":{n}}}]}}]}}]}}}}]}}"#
        );
        let m = parse(doc.as_bytes()).unwrap();
        let text = String::from_utf8(to_canonical_vec(&m).unwrap()).unwrap();
        assert!(
            text.contains(&format!(r#"{{"size":{jcs_form},"type":"t","uri":"u"}}"#)),
            "{n}: {text}"
        );
    }
}
