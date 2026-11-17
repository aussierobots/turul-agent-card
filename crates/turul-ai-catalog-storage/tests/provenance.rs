mod common;

use common::*;
use turul_ai_catalog_storage::{Etag, Provenance, ProvenanceError};

/// The committed bytes are the schema digest of provenance.
#[test]
fn provenance_bytes_are_canonical() {
    assert_eq!(
        Provenance::Curated {
            writer: writer("op")
        }
        .to_bytes()
        .unwrap(),
        br#"{"mode":"curated","writer":"op"}"#
    );
    assert_eq!(
        lease("svc.1", "2026-11-17T12:00:00Z").to_bytes().unwrap(),
        br#"{"expiresAt":"2026-11-17T12:00:00Z","mode":"lease","writer":"svc.1"}"#
    );
    // A fractional second is written without trailing zeros.
    assert_eq!(
        lease("svc", "2026-11-17T12:00:00.250Z").to_bytes().unwrap(),
        br#"{"expiresAt":"2026-11-17T12:00:00.25Z","mode":"lease","writer":"svc"}"#
    );
}

#[test]
fn provenance_round_trips() {
    for p in [
        Provenance::Curated {
            writer: writer("op"),
        },
        lease("svc", "2026-11-17T12:00:00Z"),
        lease("svc", "2026-11-17T12:00:00.000000001Z"),
    ] {
        assert_eq!(Provenance::parse(&p.to_bytes().unwrap()), Ok(p));
    }
}

#[test]
fn provenance_outside_the_two_shapes_is_refused() {
    let cases: [(&[u8], ProvenanceError); 11] = [
        (b"[]", ProvenanceError::NotAnObject),
        (b"not json", ProvenanceError::NotAnObject),
        (br#"{"writer":"op"}"#, ProvenanceError::Mode),
        (br#"{"mode":"other","writer":"op"}"#, ProvenanceError::Mode),
        (br#"{"mode":"curated"}"#, ProvenanceError::Members),
        (
            br#"{"mode":"curated","writer":"op","x":1}"#,
            ProvenanceError::Members,
        ),
        (
            br#"{"expiresAt":"2026-11-17T12:00:00Z","mode":"curated","writer":"op"}"#,
            ProvenanceError::Members,
        ),
        (
            br#"{"mode":"lease","writer":"op"}"#,
            ProvenanceError::Members,
        ),
        (
            br#"{"expiresAt":"tomorrow","mode":"lease","writer":"op"}"#,
            ProvenanceError::ExpiresAt,
        ),
        (
            br#"{"mode":"curated","writer":"10.0.0.1:443"}"#,
            ProvenanceError::Writer,
        ),
        // The same instant in another offset is a different byte string.
        (
            br#"{"expiresAt":"2026-11-17T14:00:00+02:00","mode":"lease","writer":"op"}"#,
            ProvenanceError::NotCanonical,
        ),
    ];
    for (bytes, error) in cases {
        assert_eq!(
            Provenance::parse(bytes),
            Err(error),
            "{}",
            String::from_utf8_lossy(bytes)
        );
    }
}

/// An entry's version is the digest of `[entry JSON, provenance JSON]`; a refresh that changes only
/// the expiry changes it.
#[test]
fn etag_covers_entry_and_provenance() {
    let r = record("urn:air:a", None);
    let p = lease("svc", "2026-11-17T12:00:00Z");
    let expected = Etag::of(
        format!(
            "[{},{}]",
            String::from_utf8_lossy(r.bytes()),
            String::from_utf8_lossy(&p.to_bytes().unwrap())
        )
        .as_bytes(),
    );
    assert_eq!(Etag::of_entry(&r, &p).unwrap(), expected);
    assert_ne!(Etag::of_entry(&r, &p).unwrap(), Etag::of(r.bytes()));
    assert_ne!(
        Etag::of_entry(&r, &p).unwrap(),
        Etag::of_entry(&r, &lease("svc", "2026-11-17T12:00:01Z")).unwrap()
    );
    assert_ne!(
        Etag::of_entry(&r, &p).unwrap(),
        Etag::of_entry(&r, &lease("other", "2026-11-17T12:00:00Z")).unwrap()
    );
    assert_ne!(
        Etag::of_entry(
            &r,
            &Provenance::Curated {
                writer: writer("svc")
            }
        )
        .unwrap(),
        Etag::of_entry(&r, &p).unwrap()
    );
    assert_eq!(
        Etag::of_entry(&r, &p).unwrap(),
        Etag::of_entry(&r, &p).unwrap()
    );
}

/// An expiry outside the years 0000 to 9999 has no RFC 3339 form that reads back, so it has no
/// stored bytes and no version.
#[test]
fn provenance_that_would_not_read_back_is_refused() {
    use std::time::{Duration, UNIX_EPOCH};
    use turul_ai_catalog::timestamp::Timestamp;

    let r = record("urn:air:a", None);
    for instant in [
        UNIX_EPOCH + Duration::from_secs(253_402_300_800),
        UNIX_EPOCH - Duration::from_secs(62_167_219_201),
    ] {
        let p = Provenance::Lease {
            writer: writer("svc"),
            expires_at: Timestamp::from_system_time(instant),
        };
        assert_eq!(p.to_bytes(), Err(ProvenanceError::ExpiresAt), "{p:?}");
        assert_eq!(Etag::of_entry(&r, &p), Err(ProvenanceError::ExpiresAt));
    }
    let last = Provenance::Lease {
        writer: writer("svc"),
        expires_at: Timestamp::from_system_time(UNIX_EPOCH + Duration::new(253_402_300_799, 1)),
    };
    assert_eq!(
        last.to_bytes().unwrap(),
        br#"{"expiresAt":"9999-12-31T23:59:59.000000001Z","mode":"lease","writer":"svc"}"#
    );
}
