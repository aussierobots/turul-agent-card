//! Identity shapes: a root `did:web` DID and an `urn:air` identifier.

const PREFIX: &str = "did:web:";
const LABEL_SEP: char = '.';
const URN_AIR: &str = "urn:air:";

/// A root `did:web` DID whose domain is lowercase ASCII LDH labels and no IP address spelling
/// (#the-did-web-signer-profile). `xn--` A-labels pass as LDH text and are not IDNA-validated.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DidWebRoot(String);

impl DidWebRoot {
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

    /// The whole DID.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The domain after `did:web:`.
    pub fn domain(&self) -> &str {
        &self.0[PREFIX.len()..]
    }
}

/// An `urn:air` identifier split into publisher, namespace and name. The publisher is not
/// checked against the `did:web` domain rule here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrnAir {
    publisher: String,
    namespace: String,
    name: String,
}

impl UrnAir {
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

    pub fn publisher(&self) -> &str {
        &self.publisher
    }

    /// Every component between publisher and name, `:` kept.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}
