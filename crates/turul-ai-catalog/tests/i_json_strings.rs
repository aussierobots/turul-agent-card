//! The writer refuses noncharacters in strings and member names.

use turul_ai_catalog::canonical::CanonicalError;
use turul_ai_catalog::json::read;

/// JSON text with `@u` standing for a backslash-u escape.
fn json(input: &str) -> String {
    input.replace("@u", "\\u")
}

fn refusal(input: &str) -> String {
    let text = json(input);
    let v = read(text.as_bytes()).unwrap_or_else(|e| panic!("{text}: {e}"));
    match v.to_canonical_vec() {
        Err(CanonicalError::NotIJson { pointer, .. }) => pointer.to_string(),
        other => panic!("expected NotIJson for {text}, got {other:?}"),
    }
}

// RFC 8785 section 3.1, RFC 7493 section 2.1: no noncharacters in a string value
#[test]
fn noncharacter_in_a_string_value_is_refused_with_its_pointer() {
    assert_eq!(refusal(r#"{"a":["x","@uFFFF"]}"#), "/a/1");
    assert_eq!(refusal(r#"{"a":"@uFFFE"}"#), "/a");
    assert_eq!(refusal(r#"{"a":"@uFDD0"}"#), "/a");
    assert_eq!(refusal(r#"{"a":"@uFDEF"}"#), "/a");
    // a supplementary-plane noncharacter, escaped as a surrogate pair (U+1FFFE) and raw (U+10FFFF)
    assert_eq!(refusal(r#"{"a":"@uD83F@uDFFE"}"#), "/a");
    assert_eq!(refusal("[\"ok\",\"\u{10FFFF}\"]"), "/1");
}

// RFC 8785 section 3.1: a member name holding a noncharacter is refused at the member
#[test]
fn noncharacter_in_a_member_name_is_refused_with_the_member_pointer() {
    assert_eq!(refusal(r#"{"o":{"@uFDD0":1}}"#), "/o/\u{FDD0}");
    assert_eq!(refusal(r#"{"o":{"a@uFFFF":1}}"#), "/o/a\u{FFFF}");
}

// RFC 7493 section 2.1: only noncharacters are refused; their neighbours are written unchanged
#[test]
fn neighbours_of_noncharacters_are_written_unchanged() {
    let text = json(r#"["@uFDCF","@uFDF0","@uFFFD","@uDBFF@uDFFD"]"#);
    let v = read(text.as_bytes()).unwrap();
    let bytes = v.to_canonical_vec().unwrap();
    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        "[\"\u{FDCF}\",\"\u{FDF0}\",\"\u{FFFD}\",\"\u{10FFFD}\"]"
    );
}
