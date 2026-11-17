//! The strict RFC 8259 reader and its value type.

use std::fmt;

use crate::model::Members;
use crate::parse::{ParseError, ParseErrorKind};

/// Maximum nesting of arrays and objects (RFC 8259 section 9).
const MAX_DEPTH: usize = 128;

/// A JSON value. Numbers keep their lexeme; objects reject duplicate names.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Json>),
    Object(Members),
}

/// A JSON number kept byte-for-byte. Equality compares the exact decimal value.
#[derive(Clone, Debug)]
pub struct Number(String);

impl Number {
    pub(crate) fn new(lexeme: String) -> Self {
        Self(lexeme)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// (negative, significant digits without leading or trailing zeros, power of ten).
    /// The value is `digits × 10^power`, and the power is exact however long the exponent is.
    /// Zero is `(false, [], zero)`.
    fn decimal(&self) -> (bool, Vec<u8>, BigInt) {
        let s = self.0.as_bytes();
        let (neg, s) = match s.first() {
            Some(b'-') => (true, &s[1..]),
            _ => (false, s),
        };
        let (mant, exp) = match s.iter().position(|b| matches!(b, b'e' | b'E')) {
            Some(i) => {
                let rest = &s[i + 1..];
                let (exp_neg, rest) = match rest.first() {
                    Some(b'-') => (true, &rest[1..]),
                    Some(b'+') => (false, &rest[1..]),
                    _ => (false, rest),
                };
                (&s[..i], BigInt::from_ascii(exp_neg, rest))
            }
            None => (s, BigInt::default()),
        };
        let mut digits = Vec::new();
        let mut fraction = 0usize;
        let mut seen_dot = false;
        for &b in mant {
            if b == b'.' {
                seen_dot = true;
            } else {
                digits.push(b);
                if seen_dot {
                    fraction += 1;
                }
            }
        }
        let lead = digits.iter().take_while(|b| **b == b'0').count();
        digits.drain(..lead);
        let mut trailing = 0usize;
        while digits.last() == Some(&b'0') {
            digits.pop();
            trailing += 1;
        }
        if digits.is_empty() {
            return (false, digits, BigInt::default());
        }
        let exp = exp
            .add(&BigInt::from_usize(false, trailing))
            .add(&BigInt::from_usize(true, fraction));
        (neg, digits, exp)
    }
}

/// A signed decimal integer of any length: sign and magnitude digits (0–9), most significant
/// first, without leading zeros. Zero is `(false, [])`, so equal values have equal fields.
#[derive(Debug, Default, PartialEq)]
struct BigInt(bool, Vec<u8>);

impl BigInt {
    fn from_ascii(neg: bool, ascii: &[u8]) -> Self {
        let mag: Vec<u8> = ascii.iter().map(|d| d - b'0').collect();
        Self::new(neg, mag)
    }

    fn from_usize(neg: bool, n: usize) -> Self {
        Self::from_ascii(neg, n.to_string().as_bytes())
    }

    fn new(neg: bool, mut mag: Vec<u8>) -> Self {
        let lead = mag.iter().take_while(|d| **d == 0).count();
        mag.drain(..lead);
        Self(neg && !mag.is_empty(), mag)
    }

    fn add(&self, other: &Self) -> Self {
        if self.0 == other.0 {
            return Self::new(self.0, mag_add(&self.1, &other.1));
        }
        match mag_cmp(&self.1, &other.1) {
            std::cmp::Ordering::Less => Self::new(other.0, mag_sub(&other.1, &self.1)),
            _ => Self::new(self.0, mag_sub(&self.1, &other.1)),
        }
    }
}

fn mag_cmp(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

fn mag_add(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(a.len().max(b.len()) + 1);
    let (mut i, mut j, mut carry) = (a.len(), b.len(), 0u8);
    while i > 0 || j > 0 || carry > 0 {
        let mut d = carry;
        if i > 0 {
            i -= 1;
            d += a[i];
        }
        if j > 0 {
            j -= 1;
            d += b[j];
        }
        out.push(d % 10);
        carry = d / 10;
    }
    out.reverse();
    out
}

/// `a - b` for magnitudes with `a >= b`.
fn mag_sub(a: &[u8], b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(a.len());
    let (mut j, mut borrow) = (b.len(), 0u8);
    for &x in a.iter().rev() {
        let mut y = borrow;
        if j > 0 {
            j -= 1;
            y += b[j];
        }
        if x >= y {
            out.push(x - y);
            borrow = 0;
        } else {
            out.push(x + 10 - y);
            borrow = 1;
        }
    }
    out.reverse();
    out
}

impl PartialEq for Number {
    fn eq(&self, other: &Self) -> bool {
        self.decimal() == other.decimal()
    }
}

/// An RFC 6901 JSON Pointer. `Display` escapes `~` and `/`; the root is the empty string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct JsonPointer(Vec<String>);

impl JsonPointer {
    pub fn root() -> Self {
        Self::default()
    }

    pub fn child(&self, name: &str) -> Self {
        let mut t = self.0.clone();
        t.push(name.to_string());
        Self(t)
    }

    pub fn index(&self, i: usize) -> Self {
        self.child(&i.to_string())
    }

    /// This pointer followed by every token of `rest`.
    pub(crate) fn join(&self, rest: &JsonPointer) -> Self {
        let mut t = self.0.clone();
        t.extend(rest.0.iter().cloned());
        Self(t)
    }

    fn push(&mut self, token: String) {
        self.0.push(token);
    }

    fn pop(&mut self) {
        self.0.pop();
    }
}

impl fmt::Display for JsonPointer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for t in &self.0 {
            f.write_str("/")?;
            f.write_str(&t.replace('~', "~0").replace('/', "~1"))?;
        }
        Ok(())
    }
}

/// A Unicode noncharacter (RFC 7493 section 2.1): U+FDD0..U+FDEF or a code point ending FFFE or FFFF.
pub(crate) fn is_noncharacter(c: char) -> bool {
    (0xFDD0..=0xFDEF).contains(&(c as u32)) || (c as u32 & 0xFFFF) >= 0xFFFE
}

/// Reads one JSON text. The only JSON reader in the crate.
pub fn read(input: &[u8]) -> Result<Json, ParseError> {
    let mut r = Reader {
        b: input,
        pos: 0,
        path: JsonPointer::root(),
    };
    r.ws();
    let v = r.value(0)?;
    r.ws();
    if r.pos < r.b.len() {
        return Err(r.err(ParseErrorKind::Syntax));
    }
    Ok(v)
}

struct Reader<'a> {
    b: &'a [u8],
    pos: usize,
    path: JsonPointer,
}

impl Reader<'_> {
    fn err(&self, kind: ParseErrorKind) -> ParseError {
        ParseError {
            kind,
            pointer: self.path.clone(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.b.get(self.pos).copied()
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\r' | b'\n')) {
            self.pos += 1;
        }
    }

    fn literal(&mut self, word: &[u8], v: Json) -> Result<Json, ParseError> {
        if self.b[self.pos..].starts_with(word) {
            self.pos += word.len();
            Ok(v)
        } else {
            Err(self.err(ParseErrorKind::Syntax))
        }
    }

    /// `depth` is the number of arrays and objects enclosing this value.
    fn value(&mut self, depth: usize) -> Result<Json, ParseError> {
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Json::String(self.string()?)),
            Some(b't') => self.literal(b"true", Json::Bool(true)),
            Some(b'f') => self.literal(b"false", Json::Bool(false)),
            Some(b'n') => self.literal(b"null", Json::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            _ => Err(self.err(ParseErrorKind::Syntax)),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, ParseError> {
        if depth >= MAX_DEPTH {
            return Err(self.err(ParseErrorKind::DepthLimit));
        }
        self.pos += 1;
        let mut m = Members::default();
        self.ws();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Json::Object(m));
        }
        loop {
            self.ws();
            if self.peek() != Some(b'"') {
                return Err(self.err(ParseErrorKind::Syntax));
            }
            let name = self.string()?;
            if m.get(&name).is_some() {
                return Err(ParseError {
                    kind: ParseErrorKind::DuplicateMember,
                    pointer: self.path.child(&name),
                });
            }
            self.ws();
            if self.peek() != Some(b':') {
                return Err(self.err(ParseErrorKind::Syntax));
            }
            self.pos += 1;
            self.ws();
            self.path.push(name.clone());
            let v = self.value(depth + 1)?;
            m.insert(name, v);
            self.ws();
            // The delimiter check runs while the member is still on the path, so a defect
            // right after a value is reported at that value.
            let last = match self.peek() {
                Some(b',') => false,
                Some(b'}') => true,
                _ => return Err(self.err(ParseErrorKind::Syntax)),
            };
            self.pos += 1;
            self.path.pop();
            if last {
                return Ok(Json::Object(m));
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Json, ParseError> {
        if depth >= MAX_DEPTH {
            return Err(self.err(ParseErrorKind::DepthLimit));
        }
        self.pos += 1;
        let mut items = Vec::new();
        self.ws();
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Json::Array(items));
        }
        loop {
            self.ws();
            self.path.push(items.len().to_string());
            let v = self.value(depth + 1)?;
            items.push(v);
            self.ws();
            let last = match self.peek() {
                Some(b',') => false,
                Some(b']') => true,
                _ => return Err(self.err(ParseErrorKind::Syntax)),
            };
            self.pos += 1;
            self.path.pop();
            if last {
                return Ok(Json::Array(items));
            }
        }
    }

    fn number(&mut self) -> Result<Json, ParseError> {
        let start = self.pos;
        while matches!(
            self.peek(),
            Some(b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'.' | b'+' | b'-' | b'_')
        ) {
            self.pos += 1;
        }
        let t = &self.b[start..self.pos];
        if !is_rfc_8259_number(t) {
            return Err(self.err(ParseErrorKind::Syntax));
        }
        // Tokens are ASCII by the scan above.
        Ok(Json::Number(Number(
            String::from_utf8_lossy(t).into_owned(),
        )))
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let mut v = 0u32;
        for _ in 0..4 {
            let d = self.peek().and_then(|b| (b as char).to_digit(16));
            match d {
                Some(d) => v = v * 16 + d,
                None => return Err(self.err(ParseErrorKind::Syntax)),
            }
            self.pos += 1;
        }
        Ok(v)
    }

    /// Reads a string starting at its opening quote.
    fn string(&mut self) -> Result<String, ParseError> {
        self.pos += 1;
        let mut buf: Vec<u8> = Vec::new();
        loop {
            let Some(b) = self.peek() else {
                return Err(self.err(ParseErrorKind::Syntax));
            };
            self.pos += 1;
            match b {
                b'"' => break,
                b'\\' => {
                    let Some(e) = self.peek() else {
                        return Err(self.err(ParseErrorKind::Syntax));
                    };
                    self.pos += 1;
                    let c = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            match hi {
                                0xD800..=0xDBFF => {
                                    if !self.b[self.pos..].starts_with(b"\\u") {
                                        return Err(self.err(ParseErrorKind::LoneSurrogate));
                                    }
                                    self.pos += 2;
                                    let lo = self.hex4()?;
                                    if !(0xDC00..=0xDFFF).contains(&lo) {
                                        return Err(self.err(ParseErrorKind::LoneSurrogate));
                                    }
                                    char::from_u32(0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00))
                                        .expect("a surrogate pair is a scalar value")
                                }
                                0xDC00..=0xDFFF => {
                                    return Err(self.err(ParseErrorKind::LoneSurrogate));
                                }
                                _ => char::from_u32(hi).expect("non-surrogate BMP value"),
                            }
                        }
                        _ => return Err(self.err(ParseErrorKind::Syntax)),
                    };
                    let mut tmp = [0u8; 4];
                    buf.extend_from_slice(c.encode_utf8(&mut tmp).as_bytes());
                }
                0..=0x1F => return Err(self.err(ParseErrorKind::Syntax)),
                _ => buf.push(b),
            }
        }
        String::from_utf8(buf).map_err(|_| self.err(ParseErrorKind::InvalidUtf8))
    }
}

/// RFC 8259 section 6: `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`
fn is_rfc_8259_number(t: &[u8]) -> bool {
    let mut i = 0;
    let digits = |i: &mut usize| {
        let s = *i;
        while t.get(*i).is_some_and(u8::is_ascii_digit) {
            *i += 1;
        }
        *i - s
    };
    if t.get(i) == Some(&b'-') {
        i += 1;
    }
    match t.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            digits(&mut i);
        }
        _ => return false,
    }
    if t.get(i) == Some(&b'.') {
        i += 1;
        if digits(&mut i) == 0 {
            return false;
        }
    }
    if matches!(t.get(i), Some(b'e' | b'E')) {
        i += 1;
        if matches!(t.get(i), Some(b'+' | b'-')) {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == t.len()
}
