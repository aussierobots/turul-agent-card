//! ES256 signing of a manifest or a catalog ([#signed-payload-and-jws-construction](https://ai-catalog.io/specification/#signed-payload-and-jws-construction)).

use std::fmt;

use p256::SecretKey;
use p256::pkcs8::{DecodePrivateKey, EncodePrivateKey};
use rand_core::OsRng;
use turul_ai_catalog::json::{Json, read};
use turul_ai_catalog::model::{AiCatalog, CatalogEntry, Signature, Subject, TrustManifest};
use turul_ai_catalog::validate::{Code, DID_WEB_V1, Severity, validate, validate_manifest};
use turul_jwt_validator::detached::sign_detached_es256;

use crate::header::{Kid, TYP_CATALOG, TYP_TRUST_MANIFEST};
use crate::identity::DidWebRoot;
use crate::payload::{PayloadError, catalog_payload, manifest_payload};

/// A PKCS#8 DER ES256 private key. The validator parses it at sign time.
///
/// The bytes have no public accessor: the private key leaves the process only through a key
/// store.
pub struct SigningKey(Vec<u8>);

impl SigningKey {
    /// Wraps PKCS#8 DER bytes; the key is parsed when a signature is made, not here.
    pub fn from_pkcs8_der(der: &[u8]) -> Self {
        Self(der.to_vec())
    }

    /// Draws a fresh P-256 key from the operating system's random source.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog_trust::SigningKey;
    ///
    /// let key = SigningKey::generate();
    /// let jwk = key.public_jwk()?;
    /// # let _ = jwk;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn generate() -> Self {
        let der = SecretKey::random(&mut OsRng)
            .to_pkcs8_der()
            .expect("a P-256 key encodes as PKCS#8");
        Self(der.as_bytes().to_vec())
    }

    /// Returns the public key as a JWK object with exactly `kty`, `crv`, `x` and `y`, and never
    /// `d`.
    ///
    /// # Errors
    ///
    /// [`SignError::Key`] when the bytes are not a P-256 PKCS#8 key.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog::json::Json;
    /// use turul_ai_catalog_trust::SigningKey;
    ///
    /// let Json::Object(jwk) = SigningKey::generate().public_jwk()? else {
    ///     unreachable!("a JWK is an object");
    /// };
    /// assert_eq!(jwk.len(), 4);
    /// assert!(jwk.get("d").is_none());
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn public_jwk(&self) -> Result<Json, SignError> {
        let secret = SecretKey::from_pkcs8_der(&self.0).map_err(|_| SignError::Key)?;
        let text = secret.public_key().to_jwk_string();
        read(text.as_bytes()).map_err(|_| SignError::Key)
    }

    /// The PKCS#8 PEM of the key, for a key store.
    #[cfg(feature = "ssm")]
    pub(crate) fn to_pem(&self) -> Result<String, SignError> {
        let secret = SecretKey::from_pkcs8_der(&self.0).map_err(|_| SignError::Key)?;
        secret
            .to_pkcs8_pem(Default::default())
            .map(|pem| pem.to_string())
            .map_err(|_| SignError::Key)
    }

    /// Parses a P-256 PKCS#8 PEM.
    #[cfg(feature = "ssm")]
    pub(crate) fn from_pem(pem: &str) -> Result<Self, SignError> {
        let secret = SecretKey::from_pkcs8_pem(pem).map_err(|_| SignError::Key)?;
        let der = secret.to_pkcs8_der().map_err(|_| SignError::Key)?;
        Ok(Self(der.as_bytes().to_vec()))
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
    /// The `kid` fragment: a non-empty RFC 3986 fragment.
    pub key_fragment: String,
    /// The signature's `issuedAt`.
    pub issued_at: String,
    /// The signature's `expiresAt`, if any.
    pub expires_at: Option<String>,
}

/// Reports why signing was refused.
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
    /// The signed payload cannot be canonicalised.
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
            Self::Payload(_) => f.write_str("the payload cannot be canonicalised"),
        }
    }
}

impl std::error::Error for SignError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Payload(e) => Some(e),
            _ => None,
        }
    }
}

impl From<PayloadError> for SignError {
    fn from(e: PayloadError) -> Self {
        Self::Payload(e)
    }
}

pub(crate) fn kid(params: &SignerParams) -> Result<Kid, SignError> {
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
///
/// # Errors
///
/// - [`SignError::Kid`] when `params.key_fragment` is not a non-empty RFC 3986 fragment.
/// - [`SignError::DigestMissing`] when `entry` has no `digest`.
/// - [`SignError::SubjectMismatch`] when the manifest already names a different subject.
/// - [`SignError::ContributorMismatch`] when the manifest's `contributor` is not the signer.
/// - [`SignError::Invalid`] when the manifest, with its signature set, has an error diagnostic.
/// - [`SignError::Payload`] when the payload cannot be canonicalised.
/// - [`SignError::Key`] when `key` is not an ES256 PKCS#8 key.
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
///
/// # Errors
///
/// - [`SignError::Kid`] when `params.key_fragment` is not a non-empty RFC 3986 fragment.
/// - [`SignError::Invalid`] when the catalog, with its signature set, has an error diagnostic.
/// - [`SignError::Payload`] when the payload cannot be canonicalised.
/// - [`SignError::Key`] when `key` is not an ES256 PKCS#8 key.
///
/// # Examples
///
/// A key that is not PKCS#8 is refused; the crate-level example signs with a real one.
///
/// ```
/// use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
/// use turul_ai_catalog::model::{AiCatalog, ArtifactSource, CatalogEntry};
/// use turul_ai_catalog_trust::{DidWebRoot, SignError, SignerParams, SigningKey, sign_catalog};
///
/// let catalog = AiCatalog::new(vec![CatalogEntry::new(
///     EntryIdentifier::new("urn:air:example.com:a2a:research"),
///     ArtifactType::A2A_AGENT_CARD,
///     ArtifactSource::Url("https://agents.example.com/card.json".into()),
/// )]);
/// let params = SignerParams {
///     signer: DidWebRoot::parse("did:web:example.com").ok_or("not a root did:web DID")?,
///     key_fragment: "key-1".into(),
///     issued_at: "2026-09-12T10:00:00Z".into(),
///     expires_at: None,
/// };
/// let refused = sign_catalog(catalog, &params, &SigningKey::from_pkcs8_der(b"not a key"));
/// assert!(matches!(refused, Err(SignError::Key)));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
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
