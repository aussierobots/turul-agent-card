//! The one RFC 3339 instant ([#catalog-entry](https://ai-catalog.io/specification/#catalog-entry) `updatedAt`).

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

/// An instant with nanosecond precision, equal and ordered by instant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp {
    secs: i64,
    nanos: u32,
}

/// The text is not an RFC 3339 section 5.6 `date-time`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimestampError;

impl fmt::Display for TimestampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not an RFC 3339 date-time")
    }
}

impl std::error::Error for TimestampError {}

/// Days from 1970-01-01 to a proleptic Gregorian date (Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_in_month(y: i64, m: i64) -> i64 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) => 29,
        _ => 28,
    }
}

/// Exactly `n` ASCII digits at `b[*i..]`.
fn digits(b: &[u8], i: &mut usize, n: usize) -> Option<i64> {
    let s = b.get(*i..*i + n)?;
    if !s.iter().all(u8::is_ascii_digit) {
        return None;
    }
    *i += n;
    Some(s.iter().fold(0, |a, d| a * 10 + i64::from(d - b'0')))
}

fn byte(b: &[u8], i: &mut usize, ok: impl Fn(u8) -> bool) -> Option<u8> {
    let c = *b.get(*i).filter(|c| ok(**c))?;
    *i += 1;
    Some(c)
}

impl Timestamp {
    /// Parses an RFC 3339 section 5.6 `date-time`; `:60` is a leap second and denotes the next
    /// second.
    ///
    /// # Errors
    ///
    /// Returns [`TimestampError`] when the text is not a `date-time`: a field is missing or out of
    /// range, the offset is absent, or anything follows the offset.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog::timestamp::Timestamp;
    ///
    /// // The same instant written in two offsets is equal.
    /// let utc = Timestamp::parse("2026-03-01T12:00:00Z")?;
    /// let offset = Timestamp::parse("2026-03-01T14:00:00+02:00")?;
    /// assert_eq!(utc, offset);
    /// assert_eq!(utc.to_string(), "2026-03-01T12:00:00Z");
    ///
    /// assert!(Timestamp::parse("2026-03-01").is_err());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(s: &str) -> Result<Self, TimestampError> {
        Self::parse_inner(s.as_bytes()).ok_or(TimestampError)
    }

    fn parse_inner(b: &[u8]) -> Option<Self> {
        let i = &mut 0;
        let y = digits(b, i, 4)?;
        byte(b, i, |c| c == b'-')?;
        let m = digits(b, i, 2)?;
        byte(b, i, |c| c == b'-')?;
        let d = digits(b, i, 2)?;
        byte(b, i, |c| matches!(c, b'T' | b't'))?;
        let hh = digits(b, i, 2)?;
        byte(b, i, |c| c == b':')?;
        let mm = digits(b, i, 2)?;
        byte(b, i, |c| c == b':')?;
        let ss = digits(b, i, 2)?;
        if !(1..=12).contains(&m)
            || d < 1
            || d > days_in_month(y, m)
            || hh > 23
            || mm > 59
            || ss > 60
        {
            return None;
        }
        let mut nanos = 0u32;
        if byte(b, i, |c| c == b'.').is_some() {
            let start = *i;
            while b.get(*i).is_some_and(u8::is_ascii_digit) {
                *i += 1;
            }
            if *i == start {
                return None;
            }
            for k in 0..9 {
                nanos = nanos * 10
                    + b.get(start + k)
                        .filter(|_| start + k < *i)
                        .map_or(0, |c| u32::from(c - b'0'));
            }
        }
        let offset = if byte(b, i, |c| matches!(c, b'Z' | b'z')).is_some() {
            0
        } else {
            let sign = if byte(b, i, |c| c == b'+').is_some() {
                1
            } else {
                byte(b, i, |c| c == b'-')?;
                -1
            };
            let oh = digits(b, i, 2)?;
            byte(b, i, |c| c == b':')?;
            let om = digits(b, i, 2)?;
            if oh > 23 || om > 59 {
                return None;
            }
            sign * (oh * 3600 + om * 60)
        };
        if *i != b.len() {
            return None;
        }
        let secs = days_from_civil(y, m, d) * 86400 + hh * 3600 + mm * 60 + ss - offset;
        Some(Self { secs, nanos })
    }

    /// Converts a [`SystemTime`] to the instant it denotes.
    pub fn from_system_time(t: SystemTime) -> Self {
        match t.duration_since(UNIX_EPOCH) {
            Ok(d) => Self {
                secs: d.as_secs() as i64,
                nanos: d.subsec_nanos(),
            },
            Err(e) => {
                let d = e.duration();
                match d.subsec_nanos() {
                    0 => Self {
                        secs: -(d.as_secs() as i64),
                        nanos: 0,
                    },
                    n => Self {
                        secs: -(d.as_secs() as i64) - 1,
                        nanos: 1_000_000_000 - n,
                    },
                }
            }
        }
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (y, m, d) = civil_from_days(self.secs.div_euclid(86400));
        let t = self.secs.rem_euclid(86400);
        write!(
            f,
            "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
            t / 3600,
            t % 3600 / 60,
            t % 60
        )?;
        if self.nanos > 0 {
            let frac = format!("{:09}", self.nanos);
            write!(f, ".{}", frac.trim_end_matches('0'))?;
        }
        f.write_str("Z")
    }
}
