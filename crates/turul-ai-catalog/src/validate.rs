//! The one document validator: content rules the CDDL cannot express.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::digest::{DigestAlgorithm, DigestError, DigestValue};
use crate::extensions::{self, PolicyUrlsError};
use crate::identity::ArtifactType;
use crate::json::{Json, JsonPointer, is_noncharacter};
use crate::model::{AiCatalog, ArtifactSource, CatalogEntry, Members, Signature, TrustManifest};
use crate::number::{self, Defect};
use crate::parse;
use crate::timestamp::Timestamp;
use crate::uri;

/// The deepest nesting of catalogs this crate validates; the root is level 1. #nested-catalog-entries
/// says clients SHOULD impose a limit and that 4 is RECOMMENDED; deeper nesting is not validated.
pub const MAX_NESTING_DEPTH: u8 = 4;

/// The level of a catalog in a tree of nested catalogs: the root is 1 and the limit is
/// [`MAX_NESTING_DEPTH`] (4, the RECOMMENDED limit of #nested-catalog-entries).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NestingDepth(u8);

impl NestingDepth {
    pub const ROOT: Self = Self(1);

    pub fn get(self) -> u8 {
        self.0
    }

    /// The depth of a catalog nested in one at this depth; `None` at [`MAX_NESTING_DEPTH`].
    pub fn child(self) -> Option<Self> {
        (self.0 < MAX_NESTING_DEPTH).then(|| Self(self.0 + 1))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Code {
    SpecVersionNoncanonical,
    IdentifierEmpty,
    TypeEmpty,
    UrlEmpty,
    EntryDuplicate,
    VersionMissingInGroup,
    TimestampInvalid,
    DigestInvalid,
    DigestWeak,
    DigestAlgorithmUnknown,
    ExtensionKeyInvalid,
    PolicyUrlsInvalid,
    LogoUrlNotDataUri,
    NestedCatalogInvalid,
    NestedDepthLimit,
    NumberNotIJson,
    NumberInexact,
    StringNotIJson,
    ContributorNotAbsoluteUri,
    ManifestNotSubstantive,
    ManifestSubjectMissing,
    SignerContributorMismatch,
    SignerNotAbsoluteUri,
    ProfileEmpty,
    ProfileUnrecognised,
    SignatureExpiryOrder,
    SubjectMismatch,
    EntryDigestMissing,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SpecVersionNoncanonical => "spec-version-noncanonical",
            Self::IdentifierEmpty => "identifier-empty",
            Self::TypeEmpty => "type-empty",
            Self::UrlEmpty => "url-empty",
            Self::EntryDuplicate => "entry-duplicate",
            Self::VersionMissingInGroup => "version-missing-in-group",
            Self::TimestampInvalid => "timestamp-invalid",
            Self::DigestInvalid => "digest-invalid",
            Self::DigestWeak => "digest-weak",
            Self::DigestAlgorithmUnknown => "digest-algorithm-unknown",
            Self::ExtensionKeyInvalid => "extension-key-invalid",
            Self::PolicyUrlsInvalid => "policy-urls-invalid",
            Self::LogoUrlNotDataUri => "logo-url-not-data-uri",
            Self::NestedCatalogInvalid => "nested-catalog-invalid",
            Self::NestedDepthLimit => "nested-depth-limit",
            Self::NumberNotIJson => "number-not-i-json",
            Self::NumberInexact => "number-inexact",
            Self::StringNotIJson => "string-not-i-json",
            Self::ContributorNotAbsoluteUri => "contributor-not-absolute-uri",
            Self::ManifestNotSubstantive => "manifest-not-substantive",
            Self::ManifestSubjectMissing => "manifest-subject-missing",
            Self::SignerContributorMismatch => "signer-contributor-mismatch",
            Self::SignerNotAbsoluteUri => "signer-not-absolute-uri",
            Self::ProfileEmpty => "profile-empty",
            Self::ProfileUnrecognised => "profile-unrecognised",
            Self::SignatureExpiryOrder => "signature-expiry-order",
            Self::SubjectMismatch => "subject-mismatch",
            Self::EntryDigestMissing => "entry-digest-missing",
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Code,
    /// RFC 6901 pointer of the offending value in the validated document.
    pub path: JsonPointer,
}

/// The conformance level a document meets (never the implementation's level).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ConformanceLevel {
    Minimal,
    Discoverable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Conformance {
    /// No level is judged: the report is of a standalone manifest.
    None,
    Met(ConformanceLevel),
    NotMet,
    /// Content was left unvalidated at the nesting limit.
    Undetermined,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report {
    diagnostics: Vec<Diagnostic>,
    conformance: Conformance,
}

impl Report {
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    pub fn conformance(&self) -> Conformance {
        self.conformance
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }
}

/// The profile identifier the did:web publisher profile reserves (#profile-selection).
pub const DID_WEB_V1: &str = "did-web-v1";

/// The profile identifiers matched exactly; any other must be an absolute URI.
pub const RESERVED_PROFILES: [&str; 1] = [DID_WEB_V1];

/// The one owner of manifest-scope and signature-scope diagnostics. Pointers are
/// relative to the manifest; entry-scope binding needs the entry, so only [`validate`] reports it.
pub fn validate_manifest(manifest: &TrustManifest) -> Report {
    let mut diagnostics = Vec::new();
    check_manifest(manifest, &JsonPointer::root(), &mut diagnostics);
    if let Json::Object(m) = manifest.to_json() {
        let mut w = Walk {
            out: &mut diagnostics,
        };
        w.object(
            &m,
            &JsonPointer::root(),
            manifest.signature.is_some(),
            |_, _, _| None,
        );
    }
    Report {
        diagnostics,
        conformance: Conformance::None,
    }
}

pub fn validate(catalog: &AiCatalog) -> Report {
    validate_at(catalog, NestingDepth::ROOT)
}

/// Validates one document that sits at `depth`.
pub fn validate_at(catalog: &AiCatalog, depth: NestingDepth) -> Report {
    let mut diagnostics = Vec::new();
    check(catalog, depth, true, &mut diagnostics);
    let conformance = if diagnostics.iter().any(|d| d.severity == Severity::Error) {
        Conformance::NotMet
    } else if diagnostics.iter().any(|d| d.code == Code::NestedDepthLimit) {
        Conformance::Undetermined
    } else if catalog.host.is_some() {
        Conformance::Met(ConformanceLevel::Discoverable)
    } else {
        Conformance::Met(ConformanceLevel::Minimal)
    };
    Report {
        diagnostics,
        conformance,
    }
}

fn push(out: &mut Vec<Diagnostic>, severity: Severity, code: Code, path: JsonPointer) {
    out.push(Diagnostic {
        severity,
        code,
        path,
    });
}

fn check_keys(ext: &Option<Members>, base: &JsonPointer, out: &mut Vec<Diagnostic>) {
    for (k, _) in ext.iter().flat_map(Members::iter) {
        if !extensions::is_valid_key(k) {
            push(
                out,
                Severity::Error,
                Code::ExtensionKeyInvalid,
                base.child(k),
            );
        }
    }
}

/// Document checks, entries, uniqueness, then (when `walk`) the I-JSON walk.
fn check(c: &AiCatalog, depth: NestingDepth, walk: bool, out: &mut Vec<Diagnostic>) {
    let root = JsonPointer::root();
    let canonical = |part: &str| part.len() == 1 || !part.starts_with('0');
    if c.spec_version.split('.').any(|p| !canonical(p)) {
        push(
            out,
            Severity::Warning,
            Code::SpecVersionNoncanonical,
            root.child("specVersion"),
        );
    }
    if let Some(u) = c.host.as_ref().and_then(|h| h.logo_url.as_ref())
        && !u
            .as_bytes()
            .get(..5)
            .is_some_and(|s| s.eq_ignore_ascii_case(b"data:"))
    {
        push(
            out,
            Severity::Warning,
            Code::LogoUrlNotDataUri,
            root.child("host").child("logoUrl"),
        );
    }
    check_keys(&c.extensions, &root.child("extensions"), out);
    if let Some(sig) = &c.signature {
        check_signature(sig, &root.child("signature"), out);
    }

    let entries = root.child("entries");
    for (i, e) in c.entries.iter().enumerate() {
        check_entry(e, &entries.index(i), depth, out);
    }
    check_uniqueness(&c.entries, &entries, out);
    if walk {
        walk_document(c, out);
    }
}

fn check_entry(e: &CatalogEntry, p: &JsonPointer, depth: NestingDepth, out: &mut Vec<Diagnostic>) {
    use Severity::{Error, Warning};
    if e.identifier.as_str().is_empty() {
        push(out, Error, Code::IdentifierEmpty, p.child("identifier"));
    }
    if e.artifact_type.as_str().is_empty() {
        push(out, Error, Code::TypeEmpty, p.child("type"));
    }
    if matches!(&e.source, ArtifactSource::Url(u) if u.is_empty()) {
        push(out, Error, Code::UrlEmpty, p.child("url"));
    }
    if let Some(t) = &e.updated_at
        && Timestamp::parse(t).is_err()
    {
        push(out, Error, Code::TimestampInvalid, p.child("updatedAt"));
    }
    if let Some(d) = &e.digest {
        check_digest(d, p.child("digest"), out);
    }
    let ext = p.child("extensions");
    check_keys(&e.extensions, &ext, out);
    let manifests = p.child("trustManifests");
    for (j, t) in e.trust_manifests.iter().flatten().enumerate() {
        let tp = manifests.index(j);
        check_manifest(t, &tp, out);
        if let Some(s) = &t.subject {
            for member in s.mismatches(e) {
                push(
                    out,
                    Error,
                    Code::SubjectMismatch,
                    tp.child("subject").child(member),
                );
            }
        }
    }
    if e.digest.is_none()
        && e.trust_manifests
            .iter()
            .flatten()
            .any(|t| t.signature.is_some())
    {
        push(out, Error, Code::EntryDigestMissing, p.child("digest"));
    }
    match e.policy_urls() {
        Err(PolicyUrlsError::NotObject) => push(
            out,
            Error,
            Code::PolicyUrlsInvalid,
            ext.child(extensions::POLICY_URLS),
        ),
        Err(PolicyUrlsError::MemberNotString(m)) => push(
            out,
            Error,
            Code::PolicyUrlsInvalid,
            ext.child(extensions::POLICY_URLS).child(&m),
        ),
        Ok(_) => {}
    }
    if e.artifact_type == ArtifactType::AI_CATALOG
        && let ArtifactSource::Data(data) = &e.source
    {
        let dp = p.child("data");
        let Some(child) = depth.child() else {
            push(out, Warning, Code::NestedDepthLimit, dp);
            return;
        };
        match parse::catalog(data, &dp) {
            Err(err) => push(out, Error, Code::NestedCatalogInvalid, err.pointer),
            Ok(nested) => {
                let mut inner = Vec::new();
                check(&nested, child, false, &mut inner);
                for d in inner {
                    push(out, d.severity, d.code, dp.join(&d.path));
                }
            }
        }
    }
}

fn check_digest(d: &str, p: JsonPointer, out: &mut Vec<Diagnostic>) {
    match DigestValue::parse(d) {
        Err(DigestError::Invalid) => push(out, Severity::Error, Code::DigestInvalid, p),
        Err(DigestError::Weak) => push(out, Severity::Error, Code::DigestWeak, p),
        Ok(v) if matches!(v.algorithm(), DigestAlgorithm::Other(_)) => {
            push(out, Severity::Warning, Code::DigestAlgorithmUnknown, p)
        }
        Ok(_) => {}
    }
}

/// Manifest-scope checks at `p`.
fn check_manifest(m: &TrustManifest, p: &JsonPointer, out: &mut Vec<Diagnostic>) {
    use Severity::Error;
    if !uri::is_absolute_uri(&m.contributor) {
        push(
            out,
            Error,
            Code::ContributorNotAbsoluteUri,
            p.child("contributor"),
        );
    }
    let substantive = m.signature.is_some()
        || m.trust_schema.is_some()
        || m.attestations.as_ref().is_some_and(|a| !a.is_empty())
        || m.provenance.as_ref().is_some_and(|a| !a.is_empty())
        || m.extensions.as_ref().is_some_and(|x| !x.is_empty());
    if !substantive {
        push(out, Error, Code::ManifestNotSubstantive, p.clone());
    }
    if m.signature.is_some() && m.subject.is_none() {
        push(out, Error, Code::ManifestSubjectMissing, p.child("subject"));
    }
    check_keys(&m.extensions, &p.child("extensions"), out);
    if let Some(s) = &m.subject {
        check_digest(&s.digest, p.child("subject").child("digest"), out);
    }
    for (i, a) in m.attestations.iter().flatten().enumerate() {
        if let Some(d) = &a.digest {
            check_digest(d, p.child("attestations").index(i).child("digest"), out);
        }
    }
    for (i, l) in m.provenance.iter().flatten().enumerate() {
        if let Some(d) = &l.source_digest {
            check_digest(d, p.child("provenance").index(i).child("sourceDigest"), out);
        }
    }
    if let Some(s) = &m.signature {
        let sp = p.child("signature");
        if s.signer != m.contributor {
            push(
                out,
                Error,
                Code::SignerContributorMismatch,
                sp.child("signer"),
            );
        }
        check_signature(s, &sp, out);
    }
}

/// Signature own-member checks at `p`; never reads `jws`.
fn check_signature(s: &Signature, p: &JsonPointer, out: &mut Vec<Diagnostic>) {
    use Severity::{Error, Warning};
    if !uri::is_absolute_uri(&s.signer) {
        push(out, Error, Code::SignerNotAbsoluteUri, p.child("signer"));
    }
    if s.profile.is_empty() {
        push(out, Error, Code::ProfileEmpty, p.child("profile"));
    } else if !RESERVED_PROFILES.contains(&s.profile.as_str()) && !uri::is_absolute_uri(&s.profile)
    {
        push(out, Warning, Code::ProfileUnrecognised, p.child("profile"));
    }
    let issued = Timestamp::parse(&s.issued_at);
    if issued.is_err() {
        push(out, Error, Code::TimestampInvalid, p.child("issuedAt"));
    }
    if let Some(e) = &s.expires_at {
        match (issued, Timestamp::parse(e)) {
            (_, Err(_)) => push(out, Error, Code::TimestampInvalid, p.child("expiresAt")),
            (Ok(i), Ok(x)) if x <= i => {
                push(out, Error, Code::SignatureExpiryOrder, p.child("expiresAt"))
            }
            _ => {}
        }
    }
}

fn check_uniqueness(entries: &[CatalogEntry], base: &JsonPointer, out: &mut Vec<Diagnostic>) {
    let mut seen = HashSet::new();
    for (j, e) in entries.iter().enumerate() {
        if !seen.insert(e.key()) {
            push(out, Severity::Error, Code::EntryDuplicate, base.index(j));
        }
    }
    // identifier -> (has a versioned entry, has an unversioned entry)
    let mut groups = HashMap::new();
    for e in entries {
        let g: &mut (bool, bool) = groups.entry(&e.identifier).or_default();
        if e.version.is_some() {
            g.0 = true;
        } else {
            g.1 = true;
        }
    }
    for (i, e) in entries.iter().enumerate() {
        if e.version.is_none() && groups[&e.identifier].0 {
            push(
                out,
                Severity::Error,
                Code::VersionMissingInGroup,
                base.index(i),
            );
        }
    }
}

/// One walk over `to_json()`; `signed` marks values a signature or digest covers.
struct Walk<'a> {
    out: &'a mut Vec<Diagnostic>,
}

impl Walk<'_> {
    fn report(&mut self, code: Code, signed: bool, p: JsonPointer) {
        let severity = if signed {
            Severity::Error
        } else {
            Severity::Warning
        };
        push(self.out, severity, code, p);
    }

    fn value(&mut self, v: &Json, p: &JsonPointer, signed: bool) {
        match v {
            Json::Number(n) => match number::defect(n.as_str()) {
                Some(Defect::NotIJson) => self.report(Code::NumberNotIJson, signed, p.clone()),
                Some(Defect::Inexact) => {
                    push(self.out, Severity::Warning, Code::NumberInexact, p.clone())
                }
                None => {}
            },
            Json::String(s) if s.chars().any(is_noncharacter) => {
                self.report(Code::StringNotIJson, signed, p.clone())
            }
            Json::Array(items) => {
                for (i, x) in items.iter().enumerate() {
                    self.value(x, &p.index(i), signed);
                }
            }
            Json::Object(m) => self.object(m, p, signed, |_, _, _| None),
            _ => {}
        }
    }

    /// Members in order; `scope` may replace the walk of one member by returning its context.
    fn object(
        &mut self,
        m: &Members,
        p: &JsonPointer,
        signed: bool,
        scope: impl Fn(&str, &Json, bool) -> Option<bool>,
    ) {
        for (k, v) in m.iter() {
            let cp = p.child(k);
            if k.chars().any(is_noncharacter) {
                self.report(Code::StringNotIJson, signed, cp.clone());
            }
            self.value(v, &cp, scope(k, v, signed).unwrap_or(signed));
        }
    }
}

fn walk_document(c: &AiCatalog, out: &mut Vec<Diagnostic>) {
    let Json::Object(root) = c.to_json() else {
        return;
    };
    let mut w = Walk { out };
    let signed = c.signature.is_some();
    let p = JsonPointer::root();
    for (k, v) in root.iter() {
        let cp = p.child(k);
        if k.chars().any(is_noncharacter) {
            w.report(Code::StringNotIJson, signed, cp.clone());
        }
        match (k.as_str(), v) {
            ("entries", Json::Array(items)) => {
                for (i, (e, ej)) in c.entries.iter().zip(items).enumerate() {
                    walk_entry(&mut w, e, ej, &cp.index(i), signed);
                }
            }
            _ => w.value(v, &cp, signed),
        }
    }
}

fn walk_entry(w: &mut Walk, e: &CatalogEntry, ej: &Json, p: &JsonPointer, signed: bool) {
    let Json::Object(m) = ej else {
        return;
    };
    for (k, v) in m.iter() {
        let cp = p.child(k);
        if k.chars().any(is_noncharacter) {
            w.report(Code::StringNotIJson, signed, cp.clone());
        }
        match (k.as_str(), v) {
            ("data", _) => w.value(v, &cp, signed || e.digest.is_some()),
            ("trustManifests", Json::Array(items)) => {
                let manifests = e.trust_manifests.iter().flatten();
                for (j, (t, tj)) in manifests.zip(items).enumerate() {
                    let tp = cp.index(j);
                    let ts = signed || t.signature.is_some();
                    match tj {
                        Json::Object(tm) => w.object(tm, &tp, ts, |_, _, _| None),
                        other => w.value(other, &tp, ts),
                    }
                }
            }
            _ => w.value(v, &cp, signed),
        }
    }
}
