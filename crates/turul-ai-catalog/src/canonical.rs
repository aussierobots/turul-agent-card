//! RFC 8785 (JCS) serialisation: the only code in the workspace that emits JSON text.

use std::fmt;

use crate::json::{Json, JsonPointer, is_noncharacter};
use crate::model::{AiCatalog, Members};
use crate::number;

/// A value RFC 8785 cannot canonicalise: it accepts only I-JSON (RFC 8785 section 3.1).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum CanonicalError {
    /// A number whose nearest double is not finite, or a string or member name holding a
    /// noncharacter.
    NotIJson { pointer: JsonPointer },
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotIJson { pointer } => write!(f, "not I-JSON at {pointer}"),
        }
    }
}

impl std::error::Error for CanonicalError {}

/// Canonical bytes of a document: `catalog.to_json().to_canonical_vec()`.
pub fn to_canonical_vec(catalog: &AiCatalog) -> Result<Vec<u8>, CanonicalError> {
    catalog.to_json().to_canonical_vec()
}

impl Json {
    /// RFC 8785 bytes of this value.
    pub fn to_canonical_vec(&self) -> Result<Vec<u8>, CanonicalError> {
        write_canonical(self)
    }
}

fn write_canonical(v: &Json) -> Result<Vec<u8>, CanonicalError> {
    let mut out = String::new();
    value(v, &JsonPointer::root(), &mut out)?;
    Ok(out.into_bytes())
}

fn refuse<T>(p: &JsonPointer) -> Result<T, CanonicalError> {
    Err(CanonicalError::NotIJson { pointer: p.clone() })
}

fn value(v: &Json, p: &JsonPointer, out: &mut String) -> Result<(), CanonicalError> {
    match v {
        Json::Null => out.push_str("null"),
        Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Json::Number(n) => match number::to_canonical(n.as_str()) {
            Some(s) => out.push_str(&s),
            None => return refuse(p),
        },
        Json::String(s) => string(s, p, out)?,
        Json::Array(items) => {
            out.push('[');
            for (i, x) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                value(x, &p.index(i), out)?;
            }
            out.push(']');
        }
        Json::Object(m) => object(m, p, out)?,
    }
    Ok(())
}

fn object(m: &Members, p: &JsonPointer, out: &mut String) -> Result<(), CanonicalError> {
    let mut members: Vec<_> = m.iter().collect();
    members.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
    out.push('{');
    for (i, (name, v)) in members.into_iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let cp = p.child(name);
        string(name, &cp, out)?;
        out.push(':');
        value(v, &cp, out)?;
    }
    out.push('}');
    Ok(())
}

fn string(s: &str, p: &JsonPointer, out: &mut String) -> Result<(), CanonicalError> {
    if s.chars().any(is_noncharacter) {
        return refuse(p);
    }
    out.push('"');
    for c in s.chars() {
        match c {
            '\u{8}' => out.push_str("\\b"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\u{c}' => out.push_str("\\f"),
            '\r' => out.push_str("\\r"),
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    Ok(())
}
