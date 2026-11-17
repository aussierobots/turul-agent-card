//! The JWS protected header of a `did-web-v1` signature ([#signed-payload-and-jws-construction](https://ai-catalog.io/specification/#signed-payload-and-jws-construction),
//! [#jws-algorithm-and-key-requirements](https://ai-catalog.io/specification/#jws-algorithm-and-key-requirements)). Parsing and every header rule but `kid` belong to
//! the JWS validator; this module builds its policy and checks `kid`.

use std::fmt;

use turul_ai_catalog::model::Signature;
use turul_ai_catalog::uri::is_fragment;
use turul_jwt_validator::detached::{
    DetachedJwsError, DetachedJwsPolicy, JwsAlg, inspect_detached,
};

use crate::identity::DidWebRoot;
use crate::object::SignedObject;

/// The `typ` header of a signed trust manifest.
pub const TYP_TRUST_MANIFEST: &str = "ai-catalog-trust-manifest+jws";
/// The `typ` header of a signed catalog root.
pub const TYP_CATALOG: &str = "ai-catalog+jws";
/// The only JWS algorithm `did-web-v1` allows.
pub const ES256: JwsAlg = JwsAlg::Es256;
/// The protected-header members `did-web-v1` forbids.
pub const DID_WEB_V1_FORBIDDEN_HEADERS: [&str; 4] = ["jku", "jwk", "x5u", "x5c"];

const ALLOWED_ALGS: [JwsAlg; 1] = [ES256];

/// Returns the header policy for `object`: ES256, the object kind's `typ` and no forbidden
/// member.
pub fn policy(object: SignedObject<'_>) -> DetachedJwsPolicy<'static> {
    DetachedJwsPolicy {
        expected_typ: match object {
            SignedObject::Manifest(_) => TYP_TRUST_MANIFEST,
            SignedObject::Catalog(_) => TYP_CATALOG,
        },
        allowed_algs: &ALLOWED_ALGS,
        forbidden_params: &DID_WEB_V1_FORBIDDEN_HEADERS,
    }
}

/// A `kid` that is the signer, `#` and a non-empty RFC 3986 fragment.
///
/// Construct it with [`Kid::parse`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kid(String);

impl Kid {
    /// Returns the `kid` when it is `signer`, `#` and a non-empty fragment, else `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog_trust::{DidWebRoot, Kid};
    ///
    /// let signer = DidWebRoot::parse("did:web:example.com").ok_or("not a root did:web DID")?;
    /// let kid = Kid::parse("did:web:example.com#key-1", &signer).ok_or("not a kid")?;
    /// assert_eq!(kid.fragment(), "key-1");
    /// assert!(Kid::parse("did:web:example.com", &signer).is_none());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(kid: &str, signer: &DidWebRoot) -> Option<Kid> {
        kid_matches_signer(kid, signer).then(|| Kid(kid.to_string()))
    }

    /// Returns the whole `kid`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns the part after `#`.
    pub fn fragment(&self) -> &str {
        self.0.split_once('#').map_or("", |(_, f)| f)
    }
}

/// Reports why a protected header is refused by the `did-web-v1` profile.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum HeaderError {
    /// The validator refused the structure or a header member.
    Jws(DetachedJwsError),
    /// `kid` is missing, not a string, or not the signer plus a fragment.
    KidMismatch,
}

impl fmt::Display for HeaderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Jws(_) => f.write_str("protected header refused"),
            Self::KidMismatch => f.write_str("kid is not the signer plus a fragment"),
        }
    }
}

impl std::error::Error for HeaderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Jws(e) => Some(e),
            Self::KidMismatch => None,
        }
    }
}

/// Returns whether `kid` is exactly the signer, `#`, then a non-empty RFC 3986 fragment.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog_trust::{DidWebRoot, kid_matches_signer};
///
/// let signer = DidWebRoot::parse("did:web:example.com").ok_or("not a root did:web DID")?;
/// assert!(kid_matches_signer("did:web:example.com#key-1", &signer));
/// assert!(!kid_matches_signer("did:web:example.com#", &signer));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn kid_matches_signer(kid: &str, signer: &DidWebRoot) -> bool {
    kid.strip_prefix(signer.as_str())
        .and_then(|rest| rest.strip_prefix('#'))
        .is_some_and(|fragment| !fragment.is_empty() && is_fragment(fragment))
}

/// Checks `signature.jws`'s protected header for `object` without any key and returns its
/// `kid`. `Ok` says nothing about the signature segment.
///
/// # Errors
///
/// [`HeaderError::Jws`] when the validator refuses the structure or a header member;
/// [`HeaderError::KidMismatch`] when `signature.signer` is not a root `did:web` DID, or `kid`
/// is missing, not a string, or not the signer plus a fragment.
pub fn inspect(object: SignedObject<'_>, signature: &Signature) -> Result<Kid, HeaderError> {
    let header = inspect_detached(&signature.jws, &policy(object)).map_err(HeaderError::Jws)?;
    let signer = DidWebRoot::parse(&signature.signer).ok_or(HeaderError::KidMismatch)?;
    header
        .get("kid")
        .and_then(|k| k.as_str())
        .and_then(|k| Kid::parse(k, &signer))
        .ok_or(HeaderError::KidMismatch)
}
