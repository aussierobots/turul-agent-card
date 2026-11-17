mod common;

use common::*;
use turul_ai_catalog::digest::{DigestAlgorithm, DigestError, DigestValue};
use turul_ai_catalog::validate::Severity::{Error, Warning};

fn digest_doc(d: &str) -> String {
    doc(&[&entry("a", &format!(r#","digest":"{d}""#))])
}

fn assert_digest_diag(d: &str, code: &str, sev: turul_ai_catalog::validate::Severity) {
    assert_only(&report(&digest_doc(d)), code, sev, "/entries/0/digest");
}

// #digest-format: "Producers SHOULD use SHA-256 or stronger"; algorithm names compare without regard to case
#[test]
fn sha256_384_512_are_valid_in_any_name_case() {
    for (name, n, alg) in [
        ("sha256", 64, DigestAlgorithm::Sha256),
        ("SHA256", 64, DigestAlgorithm::Sha256),
        ("Sha384", 96, DigestAlgorithm::Sha384),
        ("sha384", 96, DigestAlgorithm::Sha384),
        ("sha512", 128, DigestAlgorithm::Sha512),
        ("SHA512", 128, DigestAlgorithm::Sha512),
    ] {
        let text = format!("{name}:{}", hex(n));
        let d = DigestValue::parse(&text).unwrap();
        assert_eq!(d.algorithm(), alg, "{text}");
        assert_eq!(d.hex(), hex(n));
        assert_clean(&report(&digest_doc(&text)));
    }
}

// #digest-format: "MUST reject digest values using algorithms shorter than SHA-256"
#[test]
fn short_output_is_digest_weak() {
    for text in [
        format!("md4:{}", hex(32)),
        format!("sha1:{}", hex(40)),
        format!("blake2s:{}", hex(62)),
        format!("sha256:{}", hex(32)),
        format!("sha512:{}", hex(62)),
    ] {
        assert_eq!(
            DigestValue::parse(&text).unwrap_err(),
            DigestError::Weak,
            "{text}"
        );
        assert_digest_diag(&text, "digest-weak", Error);
    }
}

// #digest-format: a known name must carry that algorithm's output length
#[test]
fn known_name_with_wrong_length_is_digest_invalid() {
    for text in [
        format!("sha512:{}", hex(64)),
        format!("sha384:{}", hex(64)),
        format!("sha256:{}", hex(66)),
        format!("sha256:{}", hex(128)),
        format!("SHA512:{}", hex(96)),
    ] {
        assert_eq!(
            DigestValue::parse(&text).unwrap_err(),
            DigestError::Invalid,
            "{text}"
        );
        assert_digest_diag(&text, "digest-invalid", Error);
    }
}

// #digest-format: "the lowercase hexadecimal encoding"
#[test]
fn uppercase_hex_is_digest_invalid() {
    for text in [
        format!("sha256:{}", hex(64).to_uppercase()),
        format!("sha256:A{}", &hex(64)[1..]),
    ] {
        assert_eq!(
            DigestValue::parse(&text).unwrap_err(),
            DigestError::Invalid,
            "{text}"
        );
        assert_digest_diag(&text, "digest-invalid", Error);
    }
}

// #digest-format: `algorithm:hex-value`
#[test]
fn malformed_values_are_digest_invalid() {
    for text in [
        "abc".to_string(),
        "sha256".to_string(),
        ":abc".to_string(),
        "sha256:".to_string(),
        "sha256:abc".to_string(),
        format!("sha256:{}", hex(63)),
        "a b:abcd".to_string(),
        format!("sha256:{}", "zz".repeat(32)),
        format!("sha256:{}:{}", hex(32), hex(32)),
        format!("sha-:{}", hex(64)),
        format!("-sha:{}", hex(64)),
        format!("sha--256:{}", hex(64)),
        format!("sha_:{}", hex(64)),
    ] {
        assert_eq!(
            DigestValue::parse(&text).unwrap_err(),
            DigestError::Invalid,
            "{text}"
        );
        assert_digest_diag(&text, "digest-invalid", Error);
    }
}

// #digest-format: a name this crate cannot compute but whose output is long enough is not rejected
#[test]
fn unknown_long_algorithm_is_a_warning() {
    for text in [
        format!("sha3-256:{}", hex(64)),
        format!("blake2b:{}", hex(128)),
        format!("sha3.256+x_y:{}", hex(64)),
    ] {
        assert!(
            matches!(
                DigestValue::parse(&text).unwrap().algorithm(),
                DigestAlgorithm::Other(_)
            ),
            "{text}"
        );
        let r = report(&digest_doc(&text));
        assert_only(&r, "digest-algorithm-unknown", Warning, "/entries/0/digest");
        assert!(!r.has_errors());
    }
}

// #digest-format: an empty `digest` is present, and not a digest
#[test]
fn empty_digest_is_digest_invalid() {
    assert_eq!(DigestValue::parse("").unwrap_err(), DigestError::Invalid);
    assert_digest_diag("", "digest-invalid", Error);
}
