//! The one digest parser, for the `algorithm:hex-value` form of
//! [#digest-format](https://ai-catalog.io/specification/#digest-format).

/// A hash algorithm named by a digest.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DigestAlgorithm {
    /// SHA-256 (64 hex digits).
    Sha256,
    /// SHA-384 (96 hex digits).
    Sha384,
    /// SHA-512 (128 hex digits).
    Sha512,
    /// A well-formed name this crate cannot compute.
    Other(String),
}

/// Why [`DigestValue::parse`] refuses a digest value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigestError {
    /// Not `algorithm:lowercase-hex`, or not the output length of a known algorithm.
    Invalid,
    /// The output is shorter than SHA-256 ([#digest-format](https://ai-catalog.io/specification/#digest-format): "MUST reject").
    Weak,
}

/// A parsed `algorithm:hex-value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigestValue {
    algorithm: DigestAlgorithm,
    hex: String,
}

impl DigestError {
    fn message(self) -> &'static str {
        match self {
            Self::Invalid => "digest is not algorithm:lowercase-hex of a known length",
            Self::Weak => "digest output is shorter than SHA-256",
        }
    }
}

impl std::fmt::Display for DigestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for DigestError {}

impl DigestValue {
    /// Parses an `algorithm:hex-value` digest.
    ///
    /// The algorithm name is matched case-insensitively; the hex value must be lowercase and,
    /// for `sha256`, `sha384` and `sha512`, exactly the output length of that algorithm.
    ///
    /// # Errors
    ///
    /// Returns [`DigestError::Invalid`] when the text is not `algorithm:lowercase-hex` or the
    /// length does not match a known algorithm, and [`DigestError::Weak`] when the output is
    /// shorter than SHA-256.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog::digest::{DigestAlgorithm, DigestError, DigestValue};
    ///
    /// let digest = DigestValue::parse(&format!("sha256:{}", "ab".repeat(32)))?;
    /// assert_eq!(digest.algorithm(), DigestAlgorithm::Sha256);
    /// assert_eq!(digest.hex().len(), 64);
    ///
    /// assert_eq!(DigestValue::parse("md5:d41d8cd98f00b204e9800998ecf8427e"), Err(DigestError::Weak));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(s: &str) -> Result<Self, DigestError> {
        let (name, hex) = s.split_once(':').ok_or(DigestError::Invalid)?;
        if !is_algorithm_name(name)
            || hex.is_empty()
            || hex.len() % 2 != 0
            || !hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(DigestError::Invalid);
        }
        if hex.len() < 64 {
            return Err(DigestError::Weak);
        }
        let (algorithm, len) = if name.eq_ignore_ascii_case("sha256") {
            (DigestAlgorithm::Sha256, 64)
        } else if name.eq_ignore_ascii_case("sha384") {
            (DigestAlgorithm::Sha384, 96)
        } else if name.eq_ignore_ascii_case("sha512") {
            (DigestAlgorithm::Sha512, 128)
        } else {
            (DigestAlgorithm::Other(name.to_string()), hex.len())
        };
        if hex.len() != len {
            return Err(DigestError::Invalid);
        }
        Ok(Self {
            algorithm,
            hex: hex.to_string(),
        })
    }

    /// Returns the algorithm the digest names.
    pub fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm.clone()
    }

    /// Returns the lowercase hex output, without the algorithm prefix.
    pub fn hex(&self) -> &str {
        &self.hex
    }
}

/// `component (separator component)*`, `component = [A-Za-z0-9]+`, `separator = [+._-]`.
fn is_algorithm_name(name: &str) -> bool {
    name.split(['+', '.', '_', '-'])
        .all(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_alphanumeric()))
}
