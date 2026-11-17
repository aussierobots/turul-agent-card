//! Identity shapes: a root `did:web` DID and a `urn:air` identifier.

const PREFIX: &str = "did:web:";
const LABEL_SEP: char = '.';
const URN_AIR: &str = "urn:air:";

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

/// A `urn:air` identifier split into publisher, namespace and name. The publisher is not
/// checked against the `did:web` domain rule here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrnAir {
    publisher: String,
    namespace: String,
    name: String,
}

impl UrnAir {
    /// Returns the identifier when `s` is `urn:air:` followed by at least a publisher, a
    /// namespace and a name, all non-empty, else `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog_trust::UrnAir;
    ///
    /// let urn = UrnAir::parse("urn:air:example.com:a2a:research").ok_or("not an urn:air")?;
    /// assert_eq!(urn.publisher(), "example.com");
    /// assert_eq!(urn.namespace(), "a2a");
    /// assert_eq!(urn.name(), "research");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.strip_prefix(URN_AIR)?.split(':').collect();
        if parts.len() < 3 || parts.iter().any(|p| p.is_empty()) {
            return None;
        }
        Some(Self {
            publisher: parts[0].to_string(),
            namespace: parts[1..parts.len() - 1].join(":"),
            name: parts[parts.len() - 1].to_string(),
        })
    }

    /// Returns the publisher component.
    pub fn publisher(&self) -> &str {
        &self.publisher
    }

    /// Returns every component between publisher and name, `:` kept.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the name component.
    pub fn name(&self) -> &str {
        &self.name
    }
}
