mod common;

use common::*;
use turul_ai_catalog::timestamp::Timestamp;
use turul_ai_catalog::validate::Severity::Error;

fn ts(s: &str) -> Timestamp {
    Timestamp::parse(s).unwrap_or_else(|e| panic!("{s}: {e:?}"))
}

fn updated_doc(t: &str) -> String {
    doc(&[&entry("a", &format!(r#","updatedAt":"{t}""#))])
}

// #catalog-entry `updatedAt` (`tdate`): RFC 3339 section 5.6 `date-time`
#[test]
fn rfc3339_forms_parse() {
    for s in [
        "2026-02-22T16:30:00Z",
        "2026-02-22t16:30:00z",
        "2026-02-22T16:30:00+10:00",
        "2026-02-22T16:30:00-00:00",
        "2026-02-22T16:30:00.1Z",
        "2026-02-22T16:30:00.123456789Z",
        "2026-02-22T16:30:00.123456789123Z",
        "2024-02-29T00:00:00Z",
        "2000-02-29T00:00:00Z",
        "2026-12-31T23:59:59-23:59",
        "2026-04-30T00:00:00+23:59",
        "9999-12-31T23:59:59Z",
    ] {
        ts(s);
    }
    assert_eq!(
        ts("2026-02-22T16:30:00Z").to_string(),
        "2026-02-22T16:30:00Z"
    );
    assert_eq!(
        ts("2026-01-01T10:00:00+10:00").to_string(),
        "2026-01-01T00:00:00Z"
    );
    assert_eq!(
        ts("2026-02-22T16:30:00.123456789Z").to_string(),
        "2026-02-22T16:30:00.123456789Z"
    );
}

// #catalog-entry `updatedAt`: anything else is not `date-time`
#[test]
fn non_rfc3339_forms_are_refused() {
    for s in [
        "",
        "yesterday",
        "2026-02-22 16:30:00Z",
        "2026-02-22T16:30:00",
        "2026-02-22",
        "2026-02-30T00:00:00Z",
        "2026-02-29T00:00:00Z",
        "1900-02-29T00:00:00Z",
        "2026-04-31T00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-00-10T00:00:00Z",
        "2026-01-00T00:00:00Z",
        "2026-01-01T24:00:00Z",
        "2026-01-01T00:60:00Z",
        "2026-01-01T00:00:61Z",
        "2026-01-01T00:00:00+24:00",
        "2026-01-01T00:00:00+00:60",
        "2026-01-01T00:00:00.Z",
        "2026-01-01T00:00:00+1000",
        "2026-01-01T00:00:00 Z",
        " 2026-01-01T00:00:00Z",
        "2026-01-01T00:00:00Z ",
        "2026-1-1T00:00:00Z",
        "2026-01-01T00:00Z",
        "+2026-01-01T00:00:00Z",
    ] {
        assert!(Timestamp::parse(s).is_err(), "{s:?} must be refused");
    }
}

// #catalog-entry `updatedAt`: an instant, so equality and order are by instant, not text
#[test]
fn offsets_compare_as_instants() {
    assert_eq!(ts("2026-01-01T10:00:00+10:00"), ts("2026-01-01T00:00:00Z"));
    assert_eq!(ts("2026-01-01T00:00:00-00:00"), ts("2026-01-01T00:00:00Z"));
    assert_eq!(ts("2025-12-31T19:00:00-05:00"), ts("2026-01-01T00:00:00Z"));
    assert!(ts("2026-01-01T10:00:00+10:00") < ts("2026-01-01T01:00:00Z"));
    assert!(ts("2026-01-01T00:00:00.4Z") < ts("2026-01-01T00:00:00.5Z"));
    // Precision is the nanosecond; further digits are truncated.
    assert_eq!(
        ts("2026-01-01T00:00:00.1234567891Z"),
        ts("2026-01-01T00:00:00.123456789Z")
    );
}

// RFC 3339 section 5.6 `time-second`: `:60` is accepted and denotes the next second
#[test]
fn leap_second_is_accepted() {
    assert_eq!(ts("2016-12-31T23:59:60Z"), ts("2017-01-01T00:00:00Z"));
    ts("2026-01-01T00:00:60Z");
}

// #catalog-entry `updatedAt`: a value that is not a `date-time` is reported at the member
#[test]
fn invalid_updated_at_is_timestamp_invalid() {
    for t in ["yesterday", "2026-02-30T00:00:00Z", "2026-02-22 16:30:00Z"] {
        assert_only(
            &report(&updated_doc(t)),
            "timestamp-invalid",
            Error,
            "/entries/0/updatedAt",
        );
    }
    assert_clean(&report(&updated_doc("2026-02-22T16:30:00Z")));
}

// #catalog-entry `updatedAt`: an empty string is present, and not a `date-time`
#[test]
fn empty_updated_at_is_timestamp_invalid() {
    assert_only(
        &report(&updated_doc("")),
        "timestamp-invalid",
        Error,
        "/entries/0/updatedAt",
    );
}
