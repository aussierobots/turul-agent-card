//! Bytes to model: version gate, then CDDL shape mapping.

use std::fmt;

use crate::identity::{ArtifactType, EntryIdentifier, HostIdentifier, SPEC_VERSION};
use crate::json::{self, Json, JsonPointer};
use crate::model::*;
use crate::names::*;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ParseError {
    pub kind: ParseErrorKind,
    pub pointer: JsonPointer,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParseErrorKind {
    Syntax,
    InvalidUtf8,
    DepthLimit,
    LoneSurrogate,
    DuplicateMember,
    SpecVersionInvalid,
    UnsupportedMajor { found: String },
    MissingMember,
    WrongType,
    ArtifactSourceConflict,
    ArtifactSourceMissing,
}

impl fmt::Display for ParseErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax => f.write_str("invalid JSON syntax"),
            Self::InvalidUtf8 => f.write_str("invalid UTF-8"),
            Self::DepthLimit => f.write_str("JSON nesting exceeds 128 levels"),
            Self::LoneSurrogate => f.write_str("unpaired surrogate escape"),
            Self::DuplicateMember => f.write_str("duplicate member name"),
            Self::SpecVersionInvalid => {
                f.write_str("specVersion is missing or not \"<major>.<minor>\"")
            }
            Self::UnsupportedMajor { found } => {
                let supported = SPEC_VERSION.split('.').next().unwrap_or(SPEC_VERSION);
                write!(
                    f,
                    "unsupported specVersion \"{found}\"; this consumer supports major version {supported}"
                )
            }
            Self::MissingMember => f.write_str("required member is missing"),
            Self::WrongType => f.write_str("member has the wrong JSON type"),
            Self::ArtifactSourceConflict => f.write_str("entry has both url and data"),
            Self::ArtifactSourceMissing => f.write_str("entry has neither url nor data"),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.kind)
    }
}

impl std::error::Error for ParseError {}

fn fail<T>(kind: ParseErrorKind, p: &JsonPointer) -> Result<T, ParseError> {
    Err(ParseError {
        kind,
        pointer: p.clone(),
    })
}

/// Parses an AI Catalog document.
pub fn parse(input: &[u8]) -> Result<AiCatalog, ParseError> {
    catalog(&json::read(input)?, &JsonPointer::root())
}

/// Parses one catalog entry: the same member reader `parse` applies to `entries[i]`.
pub fn parse_entry(input: &[u8]) -> Result<CatalogEntry, ParseError> {
    entry(&json::read(input)?, &JsonPointer::root())
}

/// Parses Host Info: the same member reader `parse` applies to `host`.
pub fn parse_host(input: &[u8]) -> Result<HostInfo, ParseError> {
    host(&json::read(input)?, &JsonPointer::root())
}

/// A read value to the model: object check, version gate, then `document`. `p` is the
/// pointer of `v`, so every error carries its full pointer.
pub(crate) fn catalog(v: &Json, p: &JsonPointer) -> Result<AiCatalog, ParseError> {
    gate(v, p)?;
    document(v, p)
}

/// #consumer-behavior: the version is parsed before any other member is mapped.
fn gate(root: &Json, p: &JsonPointer) -> Result<(), ParseError> {
    let Json::Object(m) = root else {
        return fail(ParseErrorKind::WrongType, p);
    };
    let vp = p.child("specVersion");
    let Some(Json::String(v)) = m.get("specVersion") else {
        return fail(ParseErrorKind::SpecVersionInvalid, &vp);
    };
    let well_formed = v.split_once('.').is_some_and(|(a, b)| {
        [a, b]
            .iter()
            .all(|c| !c.is_empty() && c.bytes().all(|x| x.is_ascii_digit()))
    });
    if !well_formed {
        return fail(ParseErrorKind::SpecVersionInvalid, &vp);
    }
    let major = v.split('.').next().unwrap_or("").trim_start_matches('0');
    let supported = SPEC_VERSION.split('.').next().unwrap_or("");
    if major != supported {
        return fail(ParseErrorKind::UnsupportedMajor { found: v.clone() }, &vp);
    }
    Ok(())
}

type Res<T> = Result<T, ParseError>;

struct Obj<'a> {
    m: &'a Members,
    p: &'a JsonPointer,
}

impl<'a> Obj<'a> {
    fn of(v: &'a Json, p: &'a JsonPointer) -> Res<Self> {
        match v {
            Json::Object(m) => Ok(Self { m, p }),
            _ => fail(ParseErrorKind::WrongType, p),
        }
    }

    fn req<T>(&self, k: &str, f: impl FnOnce(&Json, &JsonPointer) -> Res<T>) -> Res<T> {
        let cp = self.p.child(k);
        match self.m.get(k) {
            Some(v) => f(v, &cp),
            None => fail(ParseErrorKind::MissingMember, &cp),
        }
    }

    fn opt<T>(&self, k: &str, f: impl FnOnce(&Json, &JsonPointer) -> Res<T>) -> Res<Option<T>> {
        match self.m.get(k) {
            Some(v) => f(v, &self.p.child(k)).map(Some),
            None => Ok(None),
        }
    }

    fn rest(&self, known: &[&str]) -> Members {
        let mut out = Members::default();
        for (k, v) in self.m.iter() {
            if !known.contains(&k.as_str()) {
                out.insert(k.clone(), v.clone());
            }
        }
        out
    }
}

fn text(v: &Json, p: &JsonPointer) -> Res<String> {
    match v {
        Json::String(s) => Ok(s.clone()),
        _ => fail(ParseErrorKind::WrongType, p),
    }
}

fn any(v: &Json, _: &JsonPointer) -> Res<Json> {
    Ok(v.clone())
}

fn members(v: &Json, p: &JsonPointer) -> Res<Members> {
    match v {
        Json::Object(m) => Ok(m.clone()),
        _ => fail(ParseErrorKind::WrongType, p),
    }
}

fn array<T>(v: &Json, p: &JsonPointer, f: impl Fn(&Json, &JsonPointer) -> Res<T>) -> Res<Vec<T>> {
    let Json::Array(items) = v else {
        return fail(ParseErrorKind::WrongType, p);
    };
    items
        .iter()
        .enumerate()
        .map(|(i, x)| f(x, &p.index(i)))
        .collect()
}

fn texts(v: &Json, p: &JsonPointer) -> Res<Vec<String>> {
    array(v, p, text)
}

/// CDDL `uint`: lexeme `0` or `[1-9][0-9]*`, at most 2^64-1 (RFC 8610 Appendix D).
fn uint(v: &Json, p: &JsonPointer) -> Res<u64> {
    if let Json::Number(n) = v {
        let s = n.as_str();
        let canonical =
            s == "0" || (s.as_bytes()[0] != b'0' && s.bytes().all(|b| b.is_ascii_digit()));
        if canonical && let Ok(x) = s.parse::<u64>() {
            return Ok(x);
        }
    }
    fail(ParseErrorKind::WrongType, p)
}

/// The one mapping from a read value to the model; stops at the first defect.
pub(crate) fn document(v: &Json, p: &JsonPointer) -> Res<AiCatalog> {
    let o = Obj::of(v, p)?;
    let spec_version = o.req("specVersion", text)?;
    let host = o.opt("host", host)?;
    let entries = o.req("entries", |v, p| array(v, p, entry))?;
    let mut c = AiCatalog::new(entries);
    c.spec_version = spec_version;
    c.host = host;
    c.signature = o.opt("signature", signature)?;
    c.additional_signatures = o.opt("additionalSignatures", any)?;
    c.extensions = o.opt("extensions", members)?;
    c.unrecognized = o.rest(DOCUMENT);
    Ok(c)
}

fn host(v: &Json, p: &JsonPointer) -> Res<HostInfo> {
    let o = Obj::of(v, p)?;
    let mut h = HostInfo::new(o.req("displayName", text)?);
    h.identifier = o.opt("identifier", text)?.map(HostIdentifier::new);
    h.documentation_url = o.opt("documentationUrl", text)?;
    h.logo_url = o.opt("logoUrl", text)?;
    h.unrecognized = o.rest(HOST);
    Ok(h)
}

fn entry(v: &Json, p: &JsonPointer) -> Res<CatalogEntry> {
    let o = Obj::of(v, p)?;
    let identifier = EntryIdentifier::new(o.req("identifier", text)?);
    let artifact_type = ArtifactType::new(o.req("type", text)?);
    let source = match (o.m.get("url"), o.m.get("data")) {
        (Some(_), Some(_)) => return fail(ParseErrorKind::ArtifactSourceConflict, p),
        (None, None) => return fail(ParseErrorKind::ArtifactSourceMissing, p),
        (Some(_), None) => ArtifactSource::Url(o.req("url", text)?),
        (None, Some(_)) => ArtifactSource::Data(o.req("data", any)?),
    };
    let mut e = CatalogEntry::new(identifier, artifact_type, source);
    e.display_name = o.opt("displayName", text)?;
    e.version = o.opt("version", text)?;
    e.description = o.opt("description", text)?;
    e.tags = o.opt("tags", texts)?;
    e.publisher = o.opt("publisher", publisher)?;
    e.digest = o.opt("digest", text)?;
    e.trust_manifests = o.opt("trustManifests", |v, p| array(v, p, manifest))?;
    e.updated_at = o.opt("updatedAt", text)?;
    e.extensions = o.opt("extensions", members)?;
    e.unrecognized = o.rest(ENTRY);
    Ok(e)
}

fn publisher(v: &Json, p: &JsonPointer) -> Res<Publisher> {
    let o = Obj::of(v, p)?;
    let mut x = Publisher::new(o.req("identifier", text)?, o.req("displayName", text)?);
    x.identity_type = o.opt("identityType", text)?;
    x.unrecognized = o.rest(PUBLISHER);
    Ok(x)
}

fn manifest(v: &Json, p: &JsonPointer) -> Res<TrustManifest> {
    let o = Obj::of(v, p)?;
    let mut x = TrustManifest::new(o.req("contributor", text)?);
    x.subject = o.opt("subject", subject)?;
    x.signature = o.opt("signature", signature)?;
    x.additional_signatures = o.opt("additionalSignatures", any)?;
    x.trust_schema = o.opt("trustSchema", trust_schema)?;
    x.attestations = o.opt("attestations", |v, p| array(v, p, attestation))?;
    x.provenance = o.opt("provenance", |v, p| array(v, p, provenance))?;
    x.extensions = o.opt("extensions", members)?;
    x.unrecognized = o.rest(MANIFEST);
    Ok(x)
}

fn subject(v: &Json, p: &JsonPointer) -> Res<Subject> {
    let o = Obj::of(v, p)?;
    let mut x = Subject::new(
        EntryIdentifier::new(o.req("identifier", text)?),
        ArtifactType::new(o.req("type", text)?),
        o.req("digest", text)?,
    );
    x.version = o.opt("version", text)?;
    x.unrecognized = o.rest(SUBJECT);
    Ok(x)
}

fn signature(v: &Json, p: &JsonPointer) -> Res<Signature> {
    let o = Obj::of(v, p)?;
    let mut x = Signature::new(
        o.req("signer", text)?,
        o.req("profile", text)?,
        o.req("issuedAt", text)?,
        o.req("jws", text)?,
    );
    x.expires_at = o.opt("expiresAt", text)?;
    x.unrecognized = o.rest(SIGNATURE);
    Ok(x)
}

fn trust_schema(v: &Json, p: &JsonPointer) -> Res<TrustSchema> {
    let o = Obj::of(v, p)?;
    let mut x = TrustSchema::new(o.req("identifier", text)?, o.req("version", text)?);
    x.governance_uri = o.opt("governanceUri", text)?;
    x.verification_methods = o.opt("verificationMethods", texts)?;
    x.unrecognized = o.rest(TRUST_SCHEMA);
    Ok(x)
}

fn attestation(v: &Json, p: &JsonPointer) -> Res<Attestation> {
    let o = Obj::of(v, p)?;
    let mut x = Attestation::new(o.req("type", text)?, o.req("uri", text)?);
    x.digest = o.opt("digest", text)?;
    x.size = o.opt("size", uint)?;
    x.description = o.opt("description", text)?;
    x.unrecognized = o.rest(ATTESTATION);
    Ok(x)
}

fn provenance(v: &Json, p: &JsonPointer) -> Res<ProvenanceLink> {
    let o = Obj::of(v, p)?;
    let mut x = ProvenanceLink::new(o.req("relation", text)?, o.req("sourceId", text)?);
    x.source_digest = o.opt("sourceDigest", text)?;
    x.registry_uri = o.opt("registryUri", text)?;
    x.statement_uri = o.opt("statementUri", text)?;
    x.signature_ref = o.opt("signatureRef", text)?;
    x.unrecognized = o.rest(PROVENANCE);
    Ok(x)
}
