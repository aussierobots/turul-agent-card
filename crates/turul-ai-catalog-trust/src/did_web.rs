//! `did:web` resolution, the DID document the profile reads, and verification-key selection
//! ([#did-document-resolution-and-key-selection](https://ai-catalog.io/specification/#did-document-resolution-and-key-selection), [#current-state-verification](https://ai-catalog.io/specification/#current-state-verification)).

use std::fmt;

use turul_ai_catalog::canonical::CanonicalError;
use turul_ai_catalog::json::{Json, JsonPointer, read};
use turul_ai_catalog::model::Members;
use turul_ai_catalog::parse::ParseError;
use turul_ai_catalog_client::{FetchError, FetchPolicy, Fetched, fetch, media_type_essence};
use turul_jwt_validator::detached::{DetachedJwsError, Es256PublicKey, check_public_jwk};

use crate::header::Kid;
use crate::identity::DidWebRoot;
use crate::sign::{SignError, SignerParams, kid};

const DID_JSON: &str = "application/did+json";
const DID_LD_JSON: &str = "application/did+ld+json";
const JSON: &str = "application/json";

/// Returns the URL of the DID document of a root `did:web` DID.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog_trust::{DidWebRoot, document_url};
///
/// let root = DidWebRoot::parse("did:web:example.com").ok_or("not a root did:web DID")?;
/// assert_eq!(document_url(&root), "https://example.com/.well-known/did.json");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn document_url(root: &DidWebRoot) -> String {
    format!("https://{}/.well-known/did.json", root.domain())
}

/// Returns the DID document of `signer` that carries `public_jwk` as the verification method
/// `<signer>#<key_fragment>`, controlled by the signer and authorised by `assertionMethod`.
/// Serve its canonical bytes as `application/did+json` at [`document_url`].
///
/// # Errors
///
/// - [`SignError::Kid`] when `key_fragment` is not a non-empty RFC 3986 fragment.
/// - [`SignError::Key`] when `public_jwk` is not a P-256 public JWK.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog_trust::{DidWebRoot, SigningKey, did_document};
///
/// let signer = DidWebRoot::parse("did:web:example.com").ok_or("not a root did:web DID")?;
/// let jwk = SigningKey::generate().public_jwk()?;
/// let document = did_document(&signer, "key-1", &jwk)?;
/// let bytes = document.to_canonical_vec()?;
/// assert!(bytes.starts_with(b"{"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn did_document(
    signer: &DidWebRoot,
    key_fragment: &str,
    public_jwk: &Json,
) -> Result<Json, SignError> {
    let params = SignerParams {
        signer: signer.clone(),
        key_fragment: key_fragment.to_string(),
        issued_at: String::new(),
        expires_at: None,
    };
    let method_id = kid(&params)?.as_str().to_string();
    let jwk_bytes = public_jwk.to_canonical_vec().map_err(|_| SignError::Key)?;
    check_public_jwk(&jwk_bytes).map_err(|_| SignError::Key)?;

    let text = |s: &str| Json::String(s.to_string());
    let mut method = Members::default();
    method.insert("id".into(), text(&method_id));
    method.insert("type".into(), text("JsonWebKey2020"));
    method.insert("controller".into(), text(signer.as_str()));
    method.insert("publicKeyJwk".into(), public_jwk.clone());
    let mut document = Members::default();
    document.insert(
        "@context".into(),
        Json::Array(vec![
            text("https://www.w3.org/ns/did/v1"),
            text("https://w3id.org/security/suites/jws-2020/v1"),
        ]),
    );
    document.insert("id".into(), text(signer.as_str()));
    document.insert(
        "verificationMethod".into(),
        Json::Array(vec![Json::Object(method)]),
    );
    document.insert(
        "assertionMethod".into(),
        Json::Array(vec![text(&method_id)]),
    );
    Ok(Json::Object(document))
}

/// A DID document that passed [`accept_document`] for its signer.
///
/// Only [`accept_document`] and [`resolve`] construct one.
#[derive(Clone, Debug, PartialEq)]
pub struct DidDocument {
    id: DidWebRoot,
    json: Json,
}

impl DidDocument {
    /// Returns the DID the document is for, equal to its `id`.
    pub fn id(&self) -> &DidWebRoot {
        &self.id
    }

    fn members(&self, name: &str) -> Option<&Json> {
        match &self.json {
            Json::Object(m) => m.get(name),
            _ => None,
        }
    }
}

/// Reports why a DID document cannot be retrieved or validated.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ResolveError {
    /// Retrieval failed.
    Fetch(FetchError),
    /// The response status was not 200.
    Status(u16),
    /// The `Content-Type` is not a DID or JSON media type; `None` when it is missing.
    MediaType(Option<String>),
    /// The body is not strict JSON.
    Json(ParseError),
    /// A member the profile reads has the wrong shape.
    Invalid {
        /// Location of the member within the document.
        pointer: JsonPointer,
    },
    /// `id` is not exactly the signer.
    IdMismatch,
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fetch(_) => f.write_str("DID document fetch failed"),
            Self::Status(s) => write!(f, "unexpected status {s}"),
            Self::MediaType(Some(m)) => write!(f, "unsupported media type {m:?}"),
            Self::MediaType(None) => f.write_str("missing Content-Type"),
            Self::Json(_) => f.write_str("DID document is not valid JSON"),
            Self::Invalid { pointer } => write!(f, "invalid DID document at {pointer}"),
            Self::IdMismatch => f.write_str("DID document id is not the signer"),
        }
    }
}

impl std::error::Error for ResolveError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Fetch(e) => Some(e),
            Self::Json(e) => Some(e),
            _ => None,
        }
    }
}

fn invalid(pointer: JsonPointer) -> ResolveError {
    ResolveError::Invalid { pointer }
}

fn string_member<'a>(
    members: &'a turul_ai_catalog::model::Members,
    name: &str,
    at: &JsonPointer,
) -> Result<&'a str, ResolveError> {
    match members.get(name) {
        Some(Json::String(s)) => Ok(s),
        _ => Err(invalid(at.child(name))),
    }
}

fn check_method(item: &Json, at: &JsonPointer) -> Result<(), ResolveError> {
    let Json::Object(m) = item else {
        return Err(invalid(at.clone()));
    };
    for name in ["id", "type", "controller"] {
        string_member(m, name, at)?;
    }
    Ok(())
}

fn check_shape(json: &Json) -> Result<(), ResolveError> {
    let root = JsonPointer::root();
    let Json::Object(m) = json else {
        return Err(invalid(root));
    };
    string_member(m, "id", &root)?;
    if let Some(v) = m.get("verificationMethod") {
        let at = root.child("verificationMethod");
        let Json::Array(items) = v else {
            return Err(invalid(at));
        };
        for (i, item) in items.iter().enumerate() {
            check_method(item, &at.index(i))?;
        }
    }
    if let Some(v) = m.get("assertionMethod") {
        let at = root.child("assertionMethod");
        let Json::Array(items) = v else {
            return Err(invalid(at));
        };
        for (i, item) in items.iter().enumerate() {
            if !matches!(item, Json::String(_)) {
                check_method(item, &at.index(i))?;
            }
        }
    }
    Ok(())
}

/// The one DID-document gate: status 200, a DID or JSON media type, strict JSON, the shape of
/// what the profile reads, and `id` exactly equal to `root`.
///
/// # Errors
///
/// - [`ResolveError::Status`] when the status is not 200.
/// - [`ResolveError::MediaType`] when `Content-Type` is missing or not `application/did+json`,
///   `application/did+ld+json` or `application/json`.
/// - [`ResolveError::Json`] when the body is not strict JSON.
/// - [`ResolveError::Invalid`] when a member the profile reads has the wrong shape.
/// - [`ResolveError::IdMismatch`] when `id` is not exactly `root`.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog_client::Fetched;
/// use turul_ai_catalog_trust::{DidWebRoot, accept_document};
///
/// let root = DidWebRoot::parse("did:web:example.com").ok_or("not a root did:web DID")?;
/// let body = br#"{"id":"did:web:example.com"}"#.to_vec();
/// let fetched = Fetched::new(200, Some("application/did+json".into()), body);
/// let doc = accept_document(&fetched, &root)?;
/// assert_eq!(doc.id(), &root);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn accept_document(fetched: &Fetched, root: &DidWebRoot) -> Result<DidDocument, ResolveError> {
    if fetched.status != 200 {
        return Err(ResolveError::Status(fetched.status));
    }
    let raw = fetched.content_type.as_deref();
    match raw.map(media_type_essence) {
        Some(e)
            if [DID_JSON, DID_LD_JSON, JSON]
                .iter()
                .any(|t| e.eq_ignore_ascii_case(t)) => {}
        _ => return Err(ResolveError::MediaType(raw.map(String::from))),
    }
    let json = read(&fetched.body).map_err(ResolveError::Json)?;
    check_shape(&json)?;
    match &json {
        Json::Object(m) if matches!(m.get("id"), Some(Json::String(id)) if id == root.as_str()) => {
        }
        _ => return Err(ResolveError::IdMismatch),
    }
    Ok(DidDocument {
        id: root.clone(),
        json,
    })
}

/// Fetches the DID document current now and accepts it with [`accept_document`].
///
/// Blocking: call it through `tokio::task::spawn_blocking` from async code.
///
/// # Errors
///
/// [`ResolveError::Fetch`] when retrieval fails, otherwise the errors of [`accept_document`].
pub fn resolve(root: &DidWebRoot, policy: &FetchPolicy) -> Result<DidDocument, ResolveError> {
    let fetched = fetch(&document_url(root), None, policy).map_err(ResolveError::Fetch)?;
    accept_document(&fetched, root)
}

/// A P-256 public key selected from a DID document and checked by the validator.
pub struct PublicJwk(Es256PublicKey);

impl PublicJwk {
    /// Returns the validated public key.
    pub fn key(&self) -> &Es256PublicKey {
        &self.0
    }
}

impl fmt::Debug for PublicJwk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PublicJwk")
    }
}

/// Reports why no verification key could be selected for a `kid`.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeySelectionError {
    /// No verification method has the `kid`.
    NotFound,
    /// More than one verification method has the `kid`.
    Ambiguous,
    /// `assertionMethod` does not authorise the `kid`.
    NotAuthorized,
    /// The method's `controller` is not the document's DID.
    ControllerMismatch,
    /// The method has no `publicKeyJwk`.
    NoPublicKeyJwk,
    /// The `publicKeyJwk` is not I-JSON.
    JwkNotIJson {
        /// Location of the offending value within the JWK.
        pointer: JsonPointer,
    },
    /// The validator refused the `publicKeyJwk` as a P-256 key.
    Jwk(DetachedJwsError),
}

impl fmt::Display for KeySelectionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => f.write_str("no verification method with the kid"),
            Self::Ambiguous => f.write_str("more than one verification method with the kid"),
            Self::NotAuthorized => f.write_str("kid is not authorized by assertionMethod"),
            Self::ControllerMismatch => f.write_str("controller is not the signer"),
            Self::NoPublicKeyJwk => f.write_str("verification method has no publicKeyJwk"),
            Self::JwkNotIJson { pointer } => write!(f, "publicKeyJwk is not I-JSON at {pointer}"),
            Self::Jwk(_) => f.write_str("publicKeyJwk is not a valid P-256 key"),
        }
    }
}

impl std::error::Error for KeySelectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Jwk(e) => Some(e),
            _ => None,
        }
    }
}

/// A `#fragment` resolves against the document `id`; any other value is compared as given.
fn resolved(value: &str, id: &str) -> String {
    if value.starts_with('#') {
        format!("{id}{value}")
    } else {
        value.to_string()
    }
}

fn array<'a>(doc: &'a DidDocument, name: &str) -> &'a [Json] {
    match doc.members(name) {
        Some(Json::Array(items)) => items,
        _ => &[],
    }
}

fn id_of(method: &Json) -> &str {
    match method {
        Json::Object(m) => match m.get("id") {
            Some(Json::String(s)) => s,
            _ => "",
        },
        _ => "",
    }
}

/// Selects the one verification method for `kid`, authorised by `assertionMethod`, controlled
/// by the document's DID, with a valid P-256 `publicKeyJwk`.
///
/// # Errors
///
/// The [`KeySelectionError`] variant that names the first requirement the document fails.
pub fn select_key(doc: &DidDocument, kid: &Kid) -> Result<PublicJwk, KeySelectionError> {
    let id = doc.id.as_str();
    let want = kid.as_str();
    let listed = array(doc, "verificationMethod")
        .iter()
        .filter(|m| resolved(id_of(m), id) == want);
    let embedded = array(doc, "assertionMethod")
        .iter()
        .filter(|m| matches!(m, Json::Object(_)) && resolved(id_of(m), id) == want);
    let referenced = array(doc, "assertionMethod")
        .iter()
        .any(|m| matches!(m, Json::String(s) if resolved(s, id) == want));

    let embedded_count = embedded.clone().count();
    let mut candidates = listed.chain(embedded);
    let (Some(method), second) = (candidates.next(), candidates.next()) else {
        return Err(KeySelectionError::NotFound);
    };
    if second.is_some() {
        return Err(KeySelectionError::Ambiguous);
    }
    if embedded_count == 0 && !referenced {
        return Err(KeySelectionError::NotAuthorized);
    }
    let Json::Object(m) = method else {
        unreachable!("candidates are objects");
    };
    match m.get("controller") {
        Some(Json::String(c)) if c == id => {}
        _ => return Err(KeySelectionError::ControllerMismatch),
    }
    let jwk = m
        .get("publicKeyJwk")
        .ok_or(KeySelectionError::NoPublicKeyJwk)?;
    let bytes = jwk.to_canonical_vec().map_err(|e| match e {
        CanonicalError::NotIJson { pointer } => KeySelectionError::JwkNotIJson { pointer },
        _ => unreachable!("NotIJson is the only canonical refusal"),
    })?;
    check_public_jwk(&bytes)
        .map(PublicJwk)
        .map_err(KeySelectionError::Jwk)
}
