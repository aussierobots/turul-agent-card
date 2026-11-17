//! The JWS protected header of a `did-web-v1` signature (#signed-payload-and-jws-construction,
//! #jws-algorithm-and-key-requirements). Parsing and every header rule but `kid` belong to the
//! validator; this module only builds its policy and checks `kid`.

use std::fmt;

use turul_ai_catalog::model::Signature;
use turul_ai_catalog::uri::is_fragment;
use turul_jwt_validator::detached::{
    DetachedJwsError, DetachedJwsPolicy, JwsAlg, inspect_detached,
};

use crate::identity::DidWebRoot;
use crate::object::SignedObject;

/// `typ` of a signed trust manifest.
pub const TYP_TRUST_MANIFEST: &str = "ai-catalog-trust-manifest+jws";
/// `typ` of a signed catalog root.
pub const TYP_CATALOG: &str = "ai-catalog+jws";
/// The one algorithm `did-web-v1` allows.
pub const ES256: JwsAlg = JwsAlg::Es256;
/// Header members `did-web-v1` forbids.
pub const DID_WEB_V1_FORBIDDEN_HEADERS: [&str; 4] = ["jku", "jwk", "x5u", "x5c"];

const ALLOWED_ALGS: [JwsAlg; 1] = [ES256];

/// The one header policy: ES256, the object kind's `typ`, no forbidden member.
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Kid(String);

impl Kid {
    /// `kid` when it is `signer`, `#` and a non-empty fragment.
    pub fn parse(kid: &str, signer: &DidWebRoot) -> Option<Kid> {
        kid_matches_signer(kid, signer).then(|| Kid(kid.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The part after `#`.
    pub fn fragment(&self) -> &str {
        self.0.split_once('#').map_or("", |(_, f)| f)
    }
}

/// A header the `did-web-v1` profile refuses.
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
            Self::Jws(e) => e.fmt(f),
            Self::KidMismatch => f.write_str("kid is not the signer plus a fragment"),
        }
    }
}

impl std::error::Error for HeaderError {}

/// `kid` is exactly the signer, `#`, then a non-empty RFC 3986 fragment.
pub fn kid_matches_signer(kid: &str, signer: &DidWebRoot) -> bool {
    kid.strip_prefix(signer.as_str())
        .and_then(|rest| rest.strip_prefix('#'))
        .is_some_and(|fragment| !fragment.is_empty() && is_fragment(fragment))
}

/// Checks `signature.jws`'s protected header for `object` without any key. `Ok` says nothing
/// about the signature segment.
pub fn inspect(object: SignedObject<'_>, signature: &Signature) -> Result<Kid, HeaderError> {
    let header = inspect_detached(&signature.jws, &policy(object)).map_err(HeaderError::Jws)?;
    let signer = DidWebRoot::parse(&signature.signer).ok_or(HeaderError::KidMismatch)?;
    header
        .get("kid")
        .and_then(|k| k.as_str())
        .and_then(|k| Kid::parse(k, &signer))
        .ok_or(HeaderError::KidMismatch)
}
