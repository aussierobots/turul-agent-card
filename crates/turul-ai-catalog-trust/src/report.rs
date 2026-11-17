//! Coverage, authority, endorsement, selection among updates and the per-entry trust report
//! ([#entry-release-coverage](https://ai-catalog.io/specification/#entry-release-coverage), [#the-did-web-publisher-profile](https://ai-catalog.io/specification/#the-did-web-publisher-profile),
//! [#catalog-authorization](https://ai-catalog.io/specification/#catalog-authorization), [#independent-contributions-and-updates](https://ai-catalog.io/specification/#independent-contributions-and-updates),
//! [#verification-result](https://ai-catalog.io/specification/#verification-result)). The report holds one outcome per signature and no catalog-wide
//! verdict.

use std::collections::HashMap;
use std::time::SystemTime;

use turul_ai_catalog::identity::{EntryIdentifier, UrnAir};
use turul_ai_catalog::model::{AiCatalog, CatalogEntry, TrustManifest};
use turul_ai_catalog_client::{Child, ResolvedCatalog};

use crate::accept::{AcceptedSignature, Reason, TrustPolicy, accept, precheck};
use crate::did_web::{DidDocument, ResolveError, resolve};
use crate::digest::{IntegrityError, verify_artifact};
use crate::identity::DidWebRoot;
use crate::object::SignedObject;

/// The DID document, or why there is none, for each signer.
pub type DidDocuments = HashMap<DidWebRoot, Result<DidDocument, ResolveError>>;

/// What one signature came to.
#[derive(Debug)]
#[non_exhaustive]
pub enum SignatureOutcome<'a> {
    /// The object has no signature.
    Unsigned,
    /// The signature was refused.
    Rejected(Reason),
    /// The signature passed every acceptance step.
    Accepted(AcceptedSignature<'a>),
}

/// Reports why an accepted manifest signature does not cover an entry release.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum CoverageGap {
    /// The accepted signature is a catalog's.
    NotAManifest,
    /// The manifest `subject` is not the entry.
    SubjectMismatch,
    /// The artifact bytes could not be shown to match `entry.digest`.
    Integrity(IntegrityError),
}

/// Entry Release Coverage of one accepted signature.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Coverage {
    /// The signature covers the entry release.
    Covered,
    /// The signature does not cover it, for the given reason.
    Uncovered(CoverageGap),
}

/// What an accepted manifest signature entitles a consumer to rely on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Endorsement {
    /// Nothing may be relied on.
    None,
    /// The contributor's signature covers the entry release.
    Contributor,
    /// `Contributor`, and the signer is the entry's `urn:air` publisher.
    Publisher,
}

/// Returns Entry Release Coverage: an accepted manifest signature, a `subject` that binds
/// `entry`, and artifact bytes that match `entry.digest` (`integrity`). The signer equals the
/// contributor already, because acceptance rejects a manifest where they differ.
pub fn entry_release_coverage(
    accepted: &AcceptedSignature<'_>,
    entry: &CatalogEntry,
    integrity: &Result<(), IntegrityError>,
) -> Coverage {
    let SignedObject::Manifest(manifest) = accepted.object else {
        return Coverage::Uncovered(CoverageGap::NotAManifest);
    };
    if !binds(manifest, entry) {
        return Coverage::Uncovered(CoverageGap::SubjectMismatch);
    }
    match integrity {
        Ok(()) => Coverage::Covered,
        Err(e) => Coverage::Uncovered(CoverageGap::Integrity(e.clone())),
    }
}

fn binds(manifest: &TrustManifest, entry: &CatalogEntry) -> bool {
    manifest.subject.as_ref().is_some_and(|s| s.binds(entry))
}

/// Returns whether the `urn:air` publisher component of `identifier`, under the root `did:web`
/// domain rule, is exactly `signer`. No subdomain, suffix or ownership heuristic applies.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog::identity::EntryIdentifier;
/// use turul_ai_catalog_trust::{DidWebRoot, publisher_authorized};
///
/// let signer = DidWebRoot::parse("did:web:example.com").ok_or("not a root did:web DID")?;
/// let id = EntryIdentifier::new("urn:air:example.com:a2a:research");
/// assert!(publisher_authorized(&id, &signer));
/// let other = EntryIdentifier::new("urn:air:other.example:a2a:research");
/// assert!(!publisher_authorized(&other, &signer));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn publisher_authorized(identifier: &EntryIdentifier, signer: &DidWebRoot) -> bool {
    UrnAir::parse(identifier.as_str())
        .and_then(|urn| DidWebRoot::parse(&format!("did:web:{}", urn.publisher())))
        .is_some_and(|publisher| &publisher == signer)
}

/// Returns whether the accepted signature is a catalog's and its signer is a configured
/// operator.
pub fn catalog_endorsed(accepted: &AcceptedSignature<'_>, policy: &TrustPolicy) -> bool {
    matches!(accepted.object, SignedObject::Catalog(_))
        && policy.operators.contains(&accepted.signer)
}

/// One manifest's outcome.
#[derive(Debug)]
#[non_exhaustive]
pub struct ManifestReport<'a> {
    /// The manifest.
    pub manifest: &'a TrustManifest,
    /// The outcome of its signature.
    pub signature: SignatureOutcome<'a>,
    /// Present when the signature was accepted.
    pub coverage: Option<Coverage>,
    /// Whether the signer is the entry's `urn:air` publisher; `false` unless the signature was
    /// accepted.
    pub publisher_authorized: bool,
}

impl ManifestReport<'_> {
    /// Returns what the signature entitles a consumer to rely on.
    pub fn endorsement(&self) -> Endorsement {
        match (&self.signature, &self.coverage) {
            (SignatureOutcome::Accepted(_), Some(Coverage::Covered))
                if self.publisher_authorized =>
            {
                Endorsement::Publisher
            }
            (SignatureOutcome::Accepted(_), Some(Coverage::Covered)) => Endorsement::Contributor,
            _ => Endorsement::None,
        }
    }

    fn issued_at(&self) -> Option<turul_ai_catalog::timestamp::Timestamp> {
        match &self.signature {
            SignatureOutcome::Accepted(a) => Some(a.issued_at),
            _ => None,
        }
    }
}

/// One entry's outcomes.
#[derive(Debug)]
#[non_exhaustive]
pub struct EntryReport<'a> {
    /// The entry.
    pub entry: &'a CatalogEntry,
    /// Present when some accepted manifest bound to the entry, so its artifact was checked.
    pub integrity: Option<Result<(), IntegrityError>>,
    /// One report per trust manifest of the entry.
    pub manifests: Vec<ManifestReport<'a>>,
}

/// One catalog node's outcomes.
#[derive(Debug)]
#[non_exhaustive]
pub struct CatalogReport<'a> {
    /// The catalog document.
    pub catalog: &'a AiCatalog,
    /// The URL the catalog was fetched from, if any.
    pub base: Option<&'a str>,
    /// The outcome of the catalog's own signature.
    pub signature: SignatureOutcome<'a>,
    /// An accepted catalog signature by a configured operator.
    pub endorsed: bool,
    /// One report per entry, in order.
    pub entries: Vec<EntryReport<'a>>,
}

/// The outcome of every signature of a resolved tree: the root first, then each nested
/// catalog in entry order.
#[derive(Debug)]
#[non_exhaustive]
pub struct TrustReport<'a> {
    /// Depth-first: the root first, then each loaded nested catalog in entry order, each
    /// followed by its own nested catalogs.
    pub catalogs: Vec<CatalogReport<'a>>,
}

/// The manifest chosen among one contributor's endorsed manifests of an entry.
#[derive(Debug)]
#[non_exhaustive]
pub enum Selection<'r, 'a> {
    /// The one manifest with the latest `issuedAt`.
    Current(&'r ManifestReport<'a>),
    /// Two or more share the latest `issuedAt`; the choice is local policy.
    Tie(Vec<&'r ManifestReport<'a>>),
}

/// Selects, per contributor (exact string), the manifest with the latest authenticated
/// `issuedAt` among those endorsed. The chosen manifest is returned unchanged; array order
/// carries no meaning.
pub fn select_current<'r, 'a>(entry: &'r EntryReport<'a>) -> Vec<(&'a str, Selection<'r, 'a>)> {
    let mut groups: Vec<(&'a str, Vec<&'r ManifestReport<'a>>)> = Vec::new();
    for m in entry
        .manifests
        .iter()
        .filter(|m| m.endorsement() != Endorsement::None)
    {
        let contributor = m.manifest.contributor.as_str();
        match groups.iter_mut().find(|(c, _)| *c == contributor) {
            Some((_, g)) => g.push(m),
            None => groups.push((contributor, vec![m])),
        }
    }
    groups
        .into_iter()
        .map(|(contributor, group)| {
            let latest = group.iter().filter_map(|m| m.issued_at()).max();
            let mut top: Vec<_> = group
                .into_iter()
                .filter(|m| m.issued_at() == latest)
                .collect();
            let selection = if top.len() == 1 {
                Selection::Current(top.remove(0))
            } else {
                Selection::Tie(top)
            };
            (contributor, selection)
        })
        .collect()
}

fn outcome<'a>(
    object: SignedObject<'a>,
    docs: &DidDocuments,
    policy: &TrustPolicy,
    now: SystemTime,
) -> SignatureOutcome<'a> {
    let (signer, kid) = match precheck(object, policy) {
        Ok(p) => p,
        Err(Reason::Unsigned) => return SignatureOutcome::Unsigned,
        Err(r) => return SignatureOutcome::Rejected(r),
    };
    let doc = match docs.get(&signer) {
        Some(Ok(doc)) => doc,
        Some(Err(e)) => return SignatureOutcome::Rejected(Reason::Resolve(Some(e.clone()))),
        None => return SignatureOutcome::Rejected(Reason::Resolve(None)),
    };
    match accept(object, signer, kid, doc, policy, now) {
        Ok(a) => SignatureOutcome::Accepted(a),
        Err(r) => SignatureOutcome::Rejected(r),
    }
}

/// Returns one entry's outcomes from already resolved DID documents, as [`verify_resolved`]
/// decides them for each entry of a tree. A signer absent from `docs` gives [`Reason::Resolve`].
/// Fetches no DID documents; artifact retrieval follows `policy.fetch`, and a relative `url`
/// resolves against `base`.
pub fn verify_entry_resolved<'a>(
    entry: &'a CatalogEntry,
    base: Option<&str>,
    docs: &DidDocuments,
    policy: &TrustPolicy,
    now: SystemTime,
) -> EntryReport<'a> {
    let outcomes: Vec<(&'a TrustManifest, SignatureOutcome<'a>)> = entry
        .trust_manifests
        .iter()
        .flatten()
        .map(|m| (m, outcome(SignedObject::Manifest(m), docs, policy, now)))
        .collect();
    // Retrieval follows authentication and the subject check, runs once per entry, and only
    // when some accepted manifest relies on it.
    let relied_on = outcomes
        .iter()
        .any(|(m, o)| matches!(o, SignatureOutcome::Accepted(_)) && binds(m, entry));
    let integrity = relied_on.then(|| verify_artifact(entry, base, &policy.fetch));
    let unused = Ok(());
    let manifests = outcomes
        .into_iter()
        .map(|(manifest, signature)| {
            let (coverage, authorized) = match &signature {
                SignatureOutcome::Accepted(a) => (
                    Some(entry_release_coverage(
                        a,
                        entry,
                        integrity.as_ref().unwrap_or(&unused),
                    )),
                    publisher_authorized(&entry.identifier, &a.signer),
                ),
                _ => (None, false),
            };
            ManifestReport {
                manifest,
                signature,
                coverage,
                publisher_authorized: authorized,
            }
        })
        .collect();
    EntryReport {
        entry,
        integrity,
        manifests,
    }
}

fn catalog_reports<'a>(
    node: &'a ResolvedCatalog,
    docs: &DidDocuments,
    policy: &TrustPolicy,
    now: SystemTime,
    out: &mut Vec<CatalogReport<'a>>,
) {
    let signature = outcome(SignedObject::Catalog(&node.document), docs, policy, now);
    let endorsed =
        matches!(&signature, SignatureOutcome::Accepted(a) if catalog_endorsed(a, policy));
    out.push(CatalogReport {
        catalog: &node.document,
        base: node.base.as_deref(),
        signature,
        endorsed,
        entries: node
            .document
            .entries
            .iter()
            .map(|e| verify_entry_resolved(e, node.base.as_deref(), docs, policy, now))
            .collect(),
    });
    for nested in &node.children {
        if let Child::Catalog(child) = &nested.child {
            catalog_reports(child, docs, policy, now, out);
        }
    }
}

/// Returns one outcome per signature of every catalog node of `tree`, each decided from its own
/// signature, its own document and its own entry. A signer absent from `docs`, or whose
/// document failed, gives [`Reason::Resolve`]. Fetches no DID documents;
/// artifact retrieval follows `policy.fetch`.
pub fn verify_resolved<'a>(
    tree: &'a ResolvedCatalog,
    docs: &DidDocuments,
    policy: &TrustPolicy,
    now: SystemTime,
) -> TrustReport<'a> {
    let mut catalogs = Vec::new();
    catalog_reports(tree, docs, policy, now, &mut catalogs);
    TrustReport { catalogs }
}

fn entry_signers(entry: &CatalogEntry, policy: &TrustPolicy, out: &mut Vec<DidWebRoot>) {
    for m in entry.trust_manifests.iter().flatten() {
        if let Ok((signer, _)) = precheck(SignedObject::Manifest(m), policy)
            && !out.contains(&signer)
        {
            out.push(signer);
        }
    }
}

fn collect_signers(node: &ResolvedCatalog, policy: &TrustPolicy, out: &mut Vec<DidWebRoot>) {
    let mut add = |object: SignedObject<'_>| {
        if let Ok((signer, _)) = precheck(object, policy)
            && !out.contains(&signer)
        {
            out.push(signer);
        }
    };
    add(SignedObject::Catalog(&node.document));
    for entry in &node.document.entries {
        for m in entry.trust_manifests.iter().flatten() {
            add(SignedObject::Manifest(m));
        }
    }
    for nested in &node.children {
        if let Child::Catalog(child) = &nested.child {
            collect_signers(child, policy, out);
        }
    }
}

/// Fetches each distinct prechecked signer's DID document once, then runs
/// [`verify_resolved`].
///
/// Blocking: call through `tokio::task::spawn_blocking` from async code.
pub fn verify<'a>(
    tree: &'a ResolvedCatalog,
    policy: &TrustPolicy,
    now: SystemTime,
) -> TrustReport<'a> {
    let mut signers = Vec::new();
    collect_signers(tree, policy, &mut signers);
    let docs: DidDocuments = signers
        .into_iter()
        .map(|s| {
            let doc = resolve(&s, &policy.fetch);
            (s, doc)
        })
        .collect();
    verify_resolved(tree, &docs, policy, now)
}

/// Fetches the DID document of each distinct prechecked signer of `entry`'s manifests once, then
/// runs [`verify_entry_resolved`].
///
/// Blocking: call through `tokio::task::spawn_blocking` from async code.
pub fn verify_entry<'a>(
    entry: &'a CatalogEntry,
    base: Option<&str>,
    policy: &TrustPolicy,
    now: SystemTime,
) -> EntryReport<'a> {
    let mut signers = Vec::new();
    entry_signers(entry, policy, &mut signers);
    let docs: DidDocuments = signers
        .into_iter()
        .map(|s| {
            let doc = resolve(&s, &policy.fetch);
            (s, doc)
        })
        .collect();
    verify_entry_resolved(entry, base, &docs, policy, now)
}
