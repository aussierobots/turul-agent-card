//! Endorsing the entries of a catalog: the artifact digest and the signer's own trust manifest
//! ([#entry-release-coverage](https://ai-catalog.io/specification/#entry-release-coverage), [#level-3-trusted-catalog](https://ai-catalog.io/specification/#level-3-trusted-catalog)).

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use turul_ai_catalog::model::{AiCatalog, CatalogEntry, TrustManifest};
use turul_ai_catalog_client::{FetchPolicy, Fetched};

use crate::accept::{TrustPolicy, verify_signature};
use crate::did_web::{DidDocument, accept_document, did_document};
use crate::digest::{IntegrityError, artifact_digest, verify_artifact};
use crate::object::SignedObject;
use crate::report::publisher_authorized;
use crate::sign::{SignError, SignerParams, SigningKey, sign_manifest};

/// Returns `entry` carrying a manifest of the signer that binds it.
///
/// A manifest of the signer that already binds the entry (identifier, type, digest and
/// version), was signed under the same key id and carries no `expiresAt` is kept as it is,
/// `issuedAt` included, and the entry is returned unchanged.
/// Otherwise every manifest of the signer is removed and one newly signed manifest is
/// appended. Manifests of other contributors are never touched.
///
/// # Errors
///
/// - [`SignError::DigestMissing`] when the entry has no `digest`.
/// - Any other [`SignError`] of [`sign_manifest`].
///
/// # Examples
///
/// ```
/// use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
/// use turul_ai_catalog::model::{ArtifactSource, CatalogEntry};
/// use turul_ai_catalog_trust::{
///     DidWebRoot, SignError, SignerParams, SigningKey, endorse_entry,
/// };
///
/// let entry = CatalogEntry::new(
///     EntryIdentifier::new("urn:air:example.com:a2a:research"),
///     ArtifactType::A2A_AGENT_CARD,
///     ArtifactSource::Url("https://agents.example.com/card.json".into()),
/// );
/// let params = SignerParams {
///     signer: DidWebRoot::parse("did:web:example.com").ok_or("not a root did:web DID")?,
///     key_fragment: "key-1".into(),
///     issued_at: "2026-09-12T10:00:00Z".into(),
///     expires_at: None,
/// };
/// let refused = endorse_entry(entry, &params, &SigningKey::generate());
/// assert!(matches!(refused, Err(SignError::DigestMissing)));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn endorse_entry(
    mut entry: CatalogEntry,
    params: &SignerParams,
    key: &SigningKey,
) -> Result<CatalogEntry, SignError> {
    if entry.digest.is_none() {
        return Err(SignError::DigestMissing);
    }
    let signer = params.signer.as_str();
    let own = own_document(params, key)?;
    // A manifest is kept only when re-signing could not change what a verifier accepts: a
    // subject that still binds the entry, a signature this key made that verifies under the
    // signer's DID document for this key, and no expiry on either side to run out.
    let keeps = |m: &TrustManifest| {
        m.contributor == signer
            && params.expires_at.is_none()
            && m.subject.as_ref().is_some_and(|s| s.binds(&entry))
            && m.signature
                .as_ref()
                .is_some_and(|sig| sig.expires_at.is_none())
            && verify_signature(
                SignedObject::Manifest(m),
                &own,
                &TrustPolicy::default(),
                judged_at(),
            )
            .is_ok()
    };
    let kept = entry.trust_manifests.iter().flatten().position(keeps);
    let mut manifests = entry.trust_manifests.take().unwrap_or_default();
    match kept {
        Some(i) => {
            // The kept manifest is the signer's only one: any other is stale.
            let keep = manifests[i].clone();
            manifests.retain(|m| m.contributor != signer);
            manifests.insert(i.min(manifests.len()), keep);
        }
        None => {
            let signed =
                sign_manifest(&entry, TrustManifest::new(signer.to_string()), params, key)?;
            manifests.retain(|m| m.contributor != signer);
            manifests.push(signed);
        }
    }
    entry.trust_manifests = Some(manifests);
    Ok(entry)
}

/// Returns the instant a kept manifest's signature is judged at. Kept manifests have no `expiresAt`, so
/// only `issuedAt` is compared with it; a fixed late instant keeps the decision independent of
/// the clock.
fn judged_at() -> SystemTime {
    // 9999-12-31T23:59:59Z, the last instant an RFC 3339 timestamp can name.
    UNIX_EPOCH + Duration::from_secs(253_402_300_799)
}

/// The signer's DID document holding only this key, as a verifier would accept it.
fn own_document(params: &SignerParams, key: &SigningKey) -> Result<DidDocument, SignError> {
    let document = did_document(&params.signer, &params.key_fragment, &key.public_jwk()?)?;
    let bytes = document.to_canonical_vec().map_err(|_| SignError::Key)?;
    accept_document(
        &Fetched::new(200, Some("application/did+json".to_string()), bytes),
        &params.signer,
    )
    .map_err(|_| SignError::Key)
}

/// What stopped the endorsement of an entry.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum EndorseErrorKind {
    /// The artifact digest could not be computed.
    Digest(IntegrityError),
    /// The entry's manifest could not be signed.
    Sign(SignError),
}

/// Reports which entry of a catalog could not be endorsed, and why.
#[derive(Clone, Debug)]
pub struct EndorseError {
    /// The index of the entry within the catalog's `entries`.
    pub entry: usize,
    /// Why it failed.
    pub kind: EndorseErrorKind,
}

impl fmt::Display for EndorseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            EndorseErrorKind::Digest(_) => {
                write!(f, "entry {}: the artifact digest failed", self.entry)
            }
            EndorseErrorKind::Sign(_) => write!(f, "entry {}: signing failed", self.entry),
        }
    }
}

impl std::error::Error for EndorseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            EndorseErrorKind::Digest(e) => Some(e),
            EndorseErrorKind::Sign(e) => Some(e),
        }
    }
}

/// Endorses, in order, every entry whose `urn:air` publisher is the signer: sets its `digest`
/// to [`artifact_digest`] (a relative `url` resolves against `base`), then [`endorse_entry`].
/// Other entries are left as they are. The first failure stops the run and returns nothing.
/// The catalog's own signature is not touched; call [`sign_catalog`](crate::sign_catalog)
/// afterwards when one is wanted.
///
/// Running again over unchanged artifacts returns identical bytes.
///
/// Blocking: a `url` entry performs an HTTP request. Call through
/// `tokio::task::spawn_blocking` from async code.
///
/// # Errors
///
/// [`EndorseError`] naming the first entry that failed, with [`EndorseErrorKind::Digest`] or
/// [`EndorseErrorKind::Sign`].
///
/// # Examples
///
/// ```
/// use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
/// use turul_ai_catalog::json::read;
/// use turul_ai_catalog::model::{AiCatalog, ArtifactSource, CatalogEntry};
/// use turul_ai_catalog_client::FetchPolicy;
/// use turul_ai_catalog_trust::{DidWebRoot, SignerParams, SigningKey, endorse_catalog};
///
/// let catalog = AiCatalog::new(vec![CatalogEntry::new(
///     EntryIdentifier::new("urn:air:example.com:a2a:research"),
///     ArtifactType::A2A_AGENT_CARD,
///     ArtifactSource::Data(read(br#"{"name":"research"}"#)?),
/// )]);
/// let params = SignerParams {
///     signer: DidWebRoot::parse("did:web:example.com").ok_or("not a root did:web DID")?,
///     key_fragment: "key-1".into(),
///     issued_at: "2026-09-12T10:00:00Z".into(),
///     expires_at: None,
/// };
/// let endorsed = endorse_catalog(
///     catalog,
///     None,
///     &params,
///     &SigningKey::generate(),
///     &FetchPolicy::default(),
/// )?;
/// assert!(endorsed.entries[0].digest.is_some());
/// assert_eq!(endorsed.entries[0].trust_manifests.as_ref().map(Vec::len), Some(1));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn endorse_catalog(
    mut catalog: AiCatalog,
    base: Option<&str>,
    params: &SignerParams,
    key: &SigningKey,
    policy: &FetchPolicy,
) -> Result<AiCatalog, EndorseError> {
    let mut changed = false;
    for (index, slot) in catalog.entries.iter_mut().enumerate() {
        if !publisher_authorized(&slot.identifier, &params.signer) {
            continue;
        }
        let at = |kind| EndorseError { entry: index, kind };
        let mut entry = slot.clone();
        // A digest the artifact still matches is kept as written, so manifests of other
        // contributors that bind it keep binding.
        if verify_artifact(&entry, base, policy).is_err() {
            entry.digest = Some(
                artifact_digest(&entry, base, policy)
                    .map_err(|e| at(EndorseErrorKind::Digest(e)))?,
            );
        }
        let entry = endorse_entry(entry, params, key).map_err(|e| at(EndorseErrorKind::Sign(e)))?;
        changed |= entry != *slot;
        *slot = entry;
    }
    // A catalog signature covers every entry, so a changed entry leaves it invalid.
    if changed {
        catalog.signature = None;
        catalog.additional_signatures = None;
    }
    Ok(catalog)
}
