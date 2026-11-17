//! `did:web` resolution, the DID document the profile reads, and verification-key selection
//! (#did-document-resolution-and-key-selection, #current-state-verification).

use std::fmt;

use turul_ai_catalog::canonical::CanonicalError;
use turul_ai_catalog::json::{Json, JsonPointer, read};
use turul_ai_catalog::parse::ParseError;
use turul_ai_catalog_client::{FetchError, FetchPolicy, Fetched, fetch, media_type_essence};
use turul_jwt_validator::detached::{DetachedJwsError, Es256PublicKey, check_public_jwk};

use crate::header::Kid;
use crate::identity::DidWebRoot;

const DID_JSON: &str = "application/did+json";
const DID_LD_JSON: &str = "application/did+ld+json";
const JSON: &str = "application/json";

/// The URL of the DID document of a root `did:web` DID.
pub fn document_url(root: &DidWebRoot) -> String {
    format!("https://{}/.well-known/did.json", root.domain())
}

/// A DID document that passed [`accept_document`] for its signer.
#[derive(Clone, Debug, PartialEq)]
pub struct DidDocument {
    id: DidWebRoot,
    json: Json,
}

impl DidDocument {
    /// The DID the document is for, equal to its `id`.
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

/// A DID document that cannot be retrieved or validated.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ResolveError {
    Fetch(FetchError),
    Status(u16),
    MediaType(Option<String>),
    Json(ParseError),
    /// A member the profile reads has the wrong shape.
    Invalid {
        pointer: JsonPointer,
    },
    /// `id` is not exactly the signer.
    IdMismatch,
}

impl fmt::Display for ResolveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Fetch(e) => write!(f, "{e}"),
            Self::Status(s) => write!(f, "unexpected status {s}"),
            Self::MediaType(Some(m)) => write!(f, "unsupported media type {m:?}"),
            Self::MediaType(None) => f.write_str("missing Content-Type"),
            Self::Json(e) => write!(f, "{e}"),
            Self::Invalid { pointer } => write!(f, "invalid DID document at {pointer}"),
            Self::IdMismatch => f.write_str("DID document id is not the signer"),
        }
    }
}

impl std::error::Error for ResolveError {}

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

/// Fetches the DID document current now and accepts it. Blocking: call it through
/// `spawn_blocking` from async code.
pub fn resolve(root: &DidWebRoot, policy: &FetchPolicy) -> Result<DidDocument, ResolveError> {
    let fetched = fetch(&document_url(root), policy).map_err(ResolveError::Fetch)?;
    accept_document(&fetched, root)
}

/// A P-256 public key selected from a DID document and checked by the validator.
pub struct PublicJwk(Es256PublicKey);

impl PublicJwk {
    pub fn key(&self) -> &Es256PublicKey {
        &self.0
    }
}

impl fmt::Debug for PublicJwk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("PublicJwk")
    }
}

/// No verification key for a `kid`.
#[derive(Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum KeySelectionError {
    NotFound,
    Ambiguous,
    NotAuthorized,
    ControllerMismatch,
    NoPublicKeyJwk,
    JwkNotIJson { pointer: JsonPointer },
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
            Self::Jwk(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for KeySelectionError {}

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

/// The one verification method for `kid`, authorised by `assertionMethod`, controlled by the
/// document's DID, with a valid P-256 `publicKeyJwk`.
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
