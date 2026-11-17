//! The one number rule, RFC 8785 section 3.2.2.3: a number is written as the ECMAScript form
//! of the IEEE 754 double nearest to its lexeme, even when that form denotes another decimal
//! value (`9007199254740993` is written `9007199254740992`). Only a lexeme whose nearest
//! double is not finite (`1e400`) has no form; RFC 8785 section 3.1 accepts only I-JSON.

use crate::json::Number;

/// The ECMAScript `Number::toString` form of the double nearest to `lexeme`, or `None` when
/// that double is not finite.
pub(crate) fn to_canonical(lexeme: &str) -> Option<String> {
    let x: f64 = lexeme.parse().ok()?;
    x.is_finite().then(|| ecmascript(x))
}

/// What validation reports about a number; `None` when its RFC 8785 form denotes its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Defect {
    /// No RFC 8785 form: the nearest double is not finite, which RFC 8785 section 3.2.2.3
    /// makes an error.
    NotIJson,
    /// The RFC 8785 form denotes another decimal value: the number has more precision than a
    /// double, which RFC 7493 section 2.2 says SHOULD NOT be sent.
    Inexact,
}

pub(crate) fn defect(lexeme: &str) -> Option<Defect> {
    let Some(form) = to_canonical(lexeme) else {
        return Some(Defect::NotIJson);
    };
    (Number::new(form) != Number::new(lexeme.to_string())).then_some(Defect::Inexact)
}

fn ecmascript(x: f64) -> String {
    if x == 0.0 {
        return "0".to_string();
    }
    let sign = if x < 0.0 { "-" } else { "" };
    // Shortest round-trip digits: "d[.ddd]e<exp>".
    // Rust's shortest form breaks a tie upward; ECMA-262 picks the even digit, which is the
    // correctly rounded (half-even) k-digit form whenever that still round-trips.
    let shortest = format!("{:e}", x.abs());
    let k = shortest
        .split_once('e')
        .expect("{:e} has an exponent")
        .0
        .replace('.', "")
        .len();
    let exact = format!("{:.*e}", k - 1, x.abs());
    let sci = if exact.parse::<f64>() == Ok(x.abs()) {
        exact
    } else {
        shortest
    };
    let (mant, exp) = sci.split_once('e').expect("{:e} has an exponent");
    let digits: String = mant.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exp.parse::<i32>().expect("exponent is an integer") + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let e = n - 1;
        let sgn = if e < 0 { '-' } else { '+' };
        let (d, rest) = digits.split_at(1);
        let frac = if rest.is_empty() {
            String::new()
        } else {
            format!(".{rest}")
        };
        format!("{d}{frac}e{sgn}{}", e.abs())
    };
    format!("{sign}{body}")
}
