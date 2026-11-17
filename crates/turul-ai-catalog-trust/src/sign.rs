//! ES256 signing of a manifest or a catalog (#signed-payload-and-jws-construction).

use std::fmt;

use turul_ai_catalog::model::{AiCatalog, CatalogEntry, Signature, Subject, TrustManifest};
use turul_ai_catalog::validate::{Code, DID_WEB_V1, Severity, validate, validate_manifest};
use turul_jwt_validator::detached::sign_detached_es256;

use crate::header::{Kid, TYP_CATALOG, TYP_TRUST_MANIFEST};
use crate::identity::DidWebRoot;
use crate::payload::{PayloadError, catalog_payload, manifest_payload};

/// A PKCS#8 DER ES256 private key. The validator parses it at sign time.
pub struct SigningKey(Vec<u8>);

impl SigningKey {
    pub fn from_pkcs8_der(der: &[u8]) -> Self {
        Self(der.to_vec())
    }
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SigningKey(..)")
    }
}

/// Who signs, with which key id, and the signature times (RFC 3339 text, judged by validation).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignerParams {
    /// The root `did:web` signer; `kid` is this, `#`, and `key_fragment`.
    pub signer: DidWebRoot,
    pub key_fragment: String,
    pub issued_at: String,
    pub expires_at: Option<String>,
}

/// A signing refusal.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SignError {
    /// The key bytes are not an ES256 PKCS#8 key.
    Key,
    /// `key_fragment` is not a non-empty RFC 3986 fragment.
    Kid,
    /// The entry has no `digest`, so no subject can bind it.
    DigestMissing,
    /// The manifest carries a subject that is not the entry's.
    SubjectMismatch,
    /// The manifest's `contributor` is not the signer.
    ContributorMismatch,
    /// The object to sign has an error diagnostic.
    Invalid(Code),
    Payload(PayloadError),
}

impl fmt::Display for SignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key => f.write_str("not an ES256 PKCS#8 signing key"),
            Self::Kid => f.write_str("kid is not the signer plus an RFC 3986 fragment"),
            Self::DigestMissing => f.write_str("the entry has no digest to bind"),
            Self::SubjectMismatch => f.write_str("the manifest subject is not the entry"),
            Self::ContributorMismatch => f.write_str("contributor is not the signer"),
            Self::Invalid(code) => write!(f, "the object to sign is invalid: {code}"),
            Self::Payload(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SignError {}

impl From<PayloadError> for SignError {
    fn from(e: PayloadError) -> Self {
        Self::Payload(e)
    }
}

fn kid(params: &SignerParams) -> Result<Kid, SignError> {
    let kid = format!("{}#{}", params.signer.as_str(), params.key_fragment);
    Kid::parse(&kid, &params.signer).ok_or(SignError::Kid)
}

fn signature(params: &SignerParams) -> Signature {
    let mut s = Signature::new(
        params.signer.as_str().to_string(),
        DID_WEB_V1.to_string(),
        params.issued_at.clone(),
        String::new(),
    );
    s.expires_at = params.expires_at.clone();
    s
}

fn first_error(report: &turul_ai_catalog::validate::Report) -> Result<(), SignError> {
    match report
        .diagnostics()
        .iter()
        .find(|d| d.severity == Severity::Error)
    {
        Some(d) => Err(SignError::Invalid(d.code)),
        None => Ok(()),
    }
}

fn jws(key: &SigningKey, typ: &str, kid: &Kid, payload: &[u8]) -> Result<String, SignError> {
    sign_detached_es256(&key.0, typ, kid.as_str(), payload).map_err(|_| SignError::Key)
}

/// Signs `manifest` as the manifest of `entry`, setting its `subject` and `signature`.
pub fn sign_manifest(
    entry: &CatalogEntry,
    mut manifest: TrustManifest,
    params: &SignerParams,
    key: &SigningKey,
) -> Result<TrustManifest, SignError> {
    let kid = kid(params)?;
    let subject = Subject::for_entry(entry).ok_or(SignError::DigestMissing)?;
    match &manifest.subject {
        Some(s) if *s != subject => return Err(SignError::SubjectMismatch),
        _ => manifest.subject = Some(subject),
    }
    if manifest.contributor != params.signer.as_str() {
        return Err(SignError::ContributorMismatch);
    }
    manifest.signature = Some(signature(params));
    first_error(&validate_manifest(&manifest))?;
    let jws = jws(key, TYP_TRUST_MANIFEST, &kid, &manifest_payload(&manifest)?)?;
    if let Some(s) = &mut manifest.signature {
        s.jws = jws;
    }
    Ok(manifest)
}

/// Signs `catalog` as a whole snapshot, setting its root `signature`.
pub fn sign_catalog(
    mut catalog: AiCatalog,
    params: &SignerParams,
    key: &SigningKey,
) -> Result<AiCatalog, SignError> {
    let kid = kid(params)?;
    catalog.signature = Some(signature(params));
    first_error(&validate(&catalog))?;
    let jws = jws(key, TYP_CATALOG, &kid, &catalog_payload(&catalog)?)?;
    if let Some(s) = &mut catalog.signature {
        s.jws = jws;
    }
    Ok(catalog)
}
