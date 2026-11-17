//! Signer identity: a root `did:web` DID.

const PREFIX: &str = "did:web:";
const LABEL_SEP: char = '.';

/// A root `did:web` DID whose domain is lowercase ASCII LDH labels and no IP address spelling
/// ([#the-did-web-signer-profile](https://ai-catalog.io/specification/#the-did-web-signer-profile)). `xn--` A-labels pass as LDH text and are not IDNA-validated.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DidWebRoot(String);

impl DidWebRoot {
    /// Returns the DID when `s` is a root `did:web` DID, else `None`.
    ///
    /// Paths, ports, uppercase and IP address spellings are refused.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog_trust::DidWebRoot;
    ///
    /// let did = DidWebRoot::parse("did:web:example.com").ok_or("not a root did:web DID")?;
    /// assert_eq!(did.domain(), "example.com");
    /// assert!(DidWebRoot::parse("did:web:example.com:user").is_none());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(s: &str) -> Option<Self> {
        let domain = s.strip_prefix(PREFIX)?;
        if domain.is_empty() || domain.len() > 253 {
            return None;
        }
        let mut last = "";
        for label in domain.split(LABEL_SEP) {
            let b = label.as_bytes();
            let edge_ok = b.first().is_some_and(|c| *c != b'-') && b.last() != Some(&b'-');
            if b.len() > 63
                || !edge_ok
                || !b
                    .iter()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
            {
                return None;
            }
            last = label;
        }
        // The WHATWG host parser's "ends in a number": such a host is an IPv4 spelling.
        let hex = last.strip_prefix("0x");
        if last.bytes().all(|c| c.is_ascii_digit())
            || hex.is_some_and(|h| h.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')))
        {
            return None;
        }
        Some(Self(s.to_string()))
    }

    /// Returns the whole DID.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the domain after `did:web:`.
    pub fn domain(&self) -> &str {
        &self.0[PREFIX.len()..]
    }
}
