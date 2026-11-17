//! The one digest parser (#digest-format).

/// A hash algorithm named by a digest.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DigestAlgorithm {
    Sha256,
    Sha384,
    Sha512,
    /// A well-formed name this crate cannot compute.
    Other(String),
}

/// Why a digest value is refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DigestError {
    /// Not `algorithm:lowercase-hex`, or not the output length of a known algorithm.
    Invalid,
    /// The output is shorter than SHA-256 (#digest-format: "MUST reject").
    Weak,
}

/// A parsed `algorithm:hex-value`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DigestValue {
    algorithm: DigestAlgorithm,
    hex: String,
}

impl DigestValue {
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

    pub fn algorithm(&self) -> DigestAlgorithm {
        self.algorithm.clone()
    }

    pub fn hex(&self) -> &str {
        &self.hex
    }
}

/// `component (separator component)*`, `component = [A-Za-z0-9]+`, `separator = [+._-]`.
fn is_algorithm_name(name: &str) -> bool {
    name.split(['+', '.', '_', '-'])
        .all(|c| !c.is_empty() && c.bytes().all(|b| b.is_ascii_alphanumeric()))
}
