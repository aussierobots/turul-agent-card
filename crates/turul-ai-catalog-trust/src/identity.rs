//! Signer identity: a root `did:web` DID.

use turul_ai_catalog::identity::is_root_did_web_domain;

const PREFIX: &str = "did:web:";

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
        if !is_root_did_web_domain(domain) {
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
