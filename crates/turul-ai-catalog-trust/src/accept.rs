//! Acceptance of one signature (#verification-procedures, #profile-selection,
//! #signature-acceptance-and-time): every outcome is a per-signature [`Reason`], never a
//! catalog-wide verdict.

use std::fmt;
use std::time::{Duration, SystemTime};

use turul_ai_catalog::timestamp::Timestamp;
use turul_ai_catalog::validate::{Code, DID_WEB_V1, Report, Severity, validate, validate_manifest};
use turul_ai_catalog_client::FetchPolicy;
use turul_jwt_validator::detached::{DetachedJwsError, verify_detached};

use crate::did_web::{DidDocument, KeySelectionError, ResolveError, select_key};
use crate::header::{self, HeaderError, Kid, inspect};
use crate::identity::DidWebRoot;
use crate::object::SignedObject;
use crate::payload::{PayloadError, catalog_payload, manifest_payload};

/// Local acceptance policy. Stricter acceptance is expressed here, never by another profile
/// name (#profile-selection).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TrustPolicy {
    /// Profiles this consumer permits.
    pub profiles: Vec<String>,
    /// Tolerance applied to both ends of the signature's validity window.
    pub clock_skew: Duration,
    /// Signers whose catalog signature counts as a catalog endorsement.
    pub operators: Vec<DidWebRoot>,
    /// Limits for the DID document and artifact fetches.
    pub fetch: FetchPolicy,
}

impl Default for TrustPolicy {
    fn default() -> Self {
        Self {
            profiles: vec![DID_WEB_V1.to_string()],
            clock_skew: Duration::ZERO,
            operators: Vec::new(),
            fetch: FetchPolicy::default(),
        }
    }
}

/// Why one signature is not a verified endorsement.
#[derive(Debug)]
#[non_exhaustive]
pub enum Reason {
    /// The object has no `signature`.
    Unsigned,
    /// The signed object has an error diagnostic.
    Invalid(Code),
    /// A `profile` other than `did-web-v1`.
    Unsupported(String),
    /// `did-web-v1` is not in the local policy.
    Disallowed,
    /// `signer` is not a root `did:web` DID.
    SignerShape,
    Header(HeaderError),
    Payload(PayloadError),
    /// No acceptable DID document for the signer; `None` when none was supplied.
    Resolve(Option<ResolveError>),
    Key(KeySelectionError),
    Signature(DetachedJwsError),
    NotYetValid,
    Expired,
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unsigned => f.write_str("no signature"),
            Self::Invalid(code) => write!(f, "the signed object is invalid: {code}"),
            Self::Unsupported(p) => write!(f, "profile {p:?} is not supported"),
            Self::Disallowed => f.write_str("profile did-web-v1 is not permitted by policy"),
            Self::SignerShape => f.write_str("signer is not a root did:web DID"),
            Self::Header(e) => e.fmt(f),
            Self::Payload(e) => e.fmt(f),
            Self::Resolve(Some(e)) => e.fmt(f),
            Self::Resolve(None) => f.write_str("no DID document for the signer"),
            Self::Key(e) => e.fmt(f),
            Self::Signature(e) => e.fmt(f),
            Self::NotYetValid => f.write_str("signature is not yet valid"),
            Self::Expired => f.write_str("signature has expired"),
        }
    }
}

impl std::error::Error for Reason {}

/// A signature that passed every acceptance step. The profile is always `did-web-v1`; the
/// object it authenticates travels with it, so its fields are never combined with another
/// object's.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AcceptedSignature<'a> {
    pub object: SignedObject<'a>,
    pub signer: DidWebRoot,
    pub kid: Kid,
    pub issued_at: Timestamp,
    pub expires_at: Option<Timestamp>,
}

fn first_error(report: &Report) -> Result<(), Reason> {
    match report
        .diagnostics()
        .iter()
        .find(|d| d.severity == Severity::Error)
    {
        Some(d) => Err(Reason::Invalid(d.code)),
        None => Ok(()),
    }
}

/// Steps 0 to 4: everything decidable without a key, a document or the network.
pub(crate) fn precheck(
    object: SignedObject<'_>,
    policy: &TrustPolicy,
) -> Result<(DidWebRoot, Kid), Reason> {
    let signature = object.signature().ok_or(Reason::Unsigned)?;
    first_error(&match object {
        SignedObject::Manifest(m) => validate_manifest(m),
        SignedObject::Catalog(c) => validate(c),
    })?;
    if signature.profile != DID_WEB_V1 {
        return Err(Reason::Unsupported(signature.profile.clone()));
    }
    if !policy.profiles.iter().any(|p| p == DID_WEB_V1) {
        return Err(Reason::Disallowed);
    }
    let signer = DidWebRoot::parse(&signature.signer).ok_or(Reason::SignerShape)?;
    let kid = inspect(object, signature).map_err(Reason::Header)?;
    Ok((signer, kid))
}

/// Steps 5 to 9 for a signature that passed [`precheck`].
pub(crate) fn accept<'a>(
    object: SignedObject<'a>,
    signer: DidWebRoot,
    kid: Kid,
    doc: &DidDocument,
    policy: &TrustPolicy,
    now: SystemTime,
) -> Result<AcceptedSignature<'a>, Reason> {
    let signature = object.signature().ok_or(Reason::Unsigned)?;
    let payload = match object {
        SignedObject::Manifest(m) => manifest_payload(m),
        SignedObject::Catalog(c) => catalog_payload(c),
    }
    .map_err(Reason::Payload)?;
    if doc.id() != &signer {
        return Err(Reason::Resolve(Some(ResolveError::IdMismatch)));
    }
    let key = select_key(doc, &kid).map_err(Reason::Key)?;
    verify_detached(&signature.jws, &payload, key.key(), &header::policy(object))
        .map_err(Reason::Signature)?;

    let issued_at = Timestamp::parse(&signature.issued_at)
        .map_err(|_| Reason::Invalid(Code::TimestampInvalid))?;
    let expires_at = signature
        .expires_at
        .as_deref()
        .map(Timestamp::parse)
        .transpose()
        .map_err(|_| Reason::Invalid(Code::TimestampInvalid))?;
    // A shifted instant that cannot be represented falls back to `now`, which is stricter.
    let late = Timestamp::from_system_time(now.checked_add(policy.clock_skew).unwrap_or(now));
    let early = Timestamp::from_system_time(now.checked_sub(policy.clock_skew).unwrap_or(now));
    if issued_at > late {
        return Err(Reason::NotYetValid);
    }
    if expires_at.is_some_and(|e| early >= e) {
        return Err(Reason::Expired);
    }
    Ok(AcceptedSignature {
        object,
        signer,
        kid,
        issued_at,
        expires_at,
    })
}

/// Accepts the signature of `object` against the signer's DID document `doc`, running the
/// checks in order and returning the first failure. No key, document or network work happens
/// for a signature that fails the key-free checks.
pub fn verify_signature<'a>(
    object: SignedObject<'a>,
    doc: &DidDocument,
    policy: &TrustPolicy,
    now: SystemTime,
) -> Result<AcceptedSignature<'a>, Reason> {
    let (signer, kid) = precheck(object, policy)?;
    accept(object, signer, kid, doc, policy, now)
}
