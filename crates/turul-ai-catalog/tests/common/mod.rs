#![allow(dead_code)]

pub mod spec_corpus;

use turul_ai_catalog::model::AiCatalog;
use turul_ai_catalog::parse::{ParseError, parse};

pub const ENTRY: &str = r#"{"identifier":"urn:air:example.com:fixture:a","type":"application/mcp-server-card+json","url":"https://example.com/a.json"}"#;

/// Entry members without the braces, for composing entries with one defect.
pub const ENTRY_BODY: &str = r#""identifier":"urn:air:example.com:fixture:a","type":"application/mcp-server-card+json","url":"https://example.com/a.json""#;

pub fn ok(input: &str) -> AiCatalog {
    match parse(input.as_bytes()) {
        Ok(c) => c,
        Err(e) => panic!("expected parse to succeed, got {e:?} for {input}"),
    }
}

pub fn err(input: &str) -> ParseError {
    err_bytes(input.as_bytes())
}

pub fn err_bytes(input: &[u8]) -> ParseError {
    match parse(input) {
        Ok(_) => panic!(
            "expected parse to fail for {}",
            String::from_utf8_lossy(input)
        ),
        Err(e) => e,
    }
}

/// A catalog document holding one entry given as members without braces.
pub fn doc_with_entry_body(body: &str) -> String {
    format!(r#"{{"specVersion":"1.0","entries":[{{{body}}}]}}"#)
}

/// A catalog document whose one entry carries `extra` after the valid entry members.
pub fn doc_with_entry_extra(extra: &str) -> String {
    doc_with_entry_body(&format!("{ENTRY_BODY},{extra}"))
}

/// A catalog document whose one entry carries one Trust Manifest given as a JSON object.
pub fn doc_with_manifest(manifest: &str) -> String {
    doc_with_entry_extra(&format!(r#""trustManifests":[{manifest}]"#))
}

/// Asserts a parse failure of the given kind at the given JSON Pointer.
#[macro_export]
macro_rules! assert_err {
    ($input:expr, $kind:pat, $pointer:expr) => {{
        let e = $crate::common::err($input);
        assert!(matches!(e.kind, $kind), "kind {:?} for {}", e.kind, $input);
        assert_eq!(
            e.pointer.to_string().as_str(),
            AsRef::<str>::as_ref(&$pointer),
            "pointer for {}",
            $input
        );
    }};
}

// ---- validation helpers ----

use turul_ai_catalog::validate::{Report, Severity, validate};

/// Parses `input` (which must parse) and validates it at the root.
pub fn report(input: &str) -> Report {
    validate(&ok(input))
}

/// Every diagnostic as (code name, severity, pointer), in report order.
pub fn found(r: &Report) -> Vec<(String, Severity, String)> {
    r.diagnostics()
        .iter()
        .map(|d| (d.code.as_str().to_string(), d.severity, d.path.to_string()))
        .collect()
}

/// Asserts the report holds exactly one diagnostic, with this code, severity and pointer.
pub fn assert_only(r: &Report, code: &str, severity: Severity, path: &str) {
    assert_eq!(
        found(r),
        vec![(code.to_string(), severity, path.to_string())]
    );
}

pub fn assert_clean(r: &Report) {
    assert_eq!(found(r), vec![], "expected no diagnostics");
}

/// A catalog document from entry objects.
pub fn doc(entries: &[&str]) -> String {
    format!(
        r#"{{"specVersion":"1.0","entries":[{}]}}"#,
        entries.join(",")
    )
}

/// An entry object with a valid `url`; `extra` is further members, each with a leading comma.
pub fn entry(identifier: &str, extra: &str) -> String {
    format!(
        r#"{{"identifier":"{identifier}","type":"application/mcp-server-card+json","url":"https://example.com/a.json"{extra}}}"#
    )
}

/// RFC 6901 escaping of one reference token.
pub fn esc(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

/// A hex string of `n` characters.
pub fn hex(n: usize) -> String {
    "ab01".chars().cycle().take(n).collect()
}
