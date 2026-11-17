//! The publisher verifier for runtime registration: a stored entry is accepted when its
//! `urn:air` publisher signed a trust manifest that covers it
//! ([#the-did-web-publisher-profile](https://ai-catalog.io/specification/#the-did-web-publisher-profile)).

use std::time::SystemTime;

use async_trait::async_trait;
use turul_ai_catalog::model::CatalogEntry;
use turul_ai_catalog_storage::{PublisherVerifier, VerifierRefusal};

use crate::accept::TrustPolicy;
use crate::report::{Endorsement, EntryReport, verify_entry};

/// Accepts an entry whose trust manifest has an accepted `did-web-v1` signature in its time
/// window, covers the entry release including its artifact bytes, and is signed by the entry's
/// `urn:air` publisher. An assessor's signature never passes.
#[derive(Clone, Debug)]
pub struct DidWebPublisherVerifier {
    policy: TrustPolicy,
}

impl DidWebPublisherVerifier {
    /// Returns a verifier that decides under `policy`.
    pub fn new(policy: TrustPolicy) -> Self {
        Self { policy }
    }
}

/// Returns `Ok` when some manifest of `report` is endorsed by the publisher, else a refusal that
/// names each manifest's outcome.
pub fn publisher_verdict(report: &EntryReport<'_>) -> Result<(), VerifierRefusal> {
    if report
        .manifests
        .iter()
        .any(|m| m.endorsement() == Endorsement::Publisher)
    {
        return Ok(());
    }
    if report.manifests.is_empty() {
        return Err(VerifierRefusal("the entry has no trust manifest".into()));
    }
    let outcomes: Vec<String> = report
        .manifests
        .iter()
        .enumerate()
        .map(|(i, m)| {
            format!(
                "manifest {i}: signature {:?}, coverage {:?}, publisher authorized {}",
                m.signature, m.coverage, m.publisher_authorized
            )
        })
        .collect();
    Err(VerifierRefusal(outcomes.join("; ")))
}

#[async_trait]
impl PublisherVerifier for DidWebPublisherVerifier {
    async fn verify_publisher(
        &self,
        entry: &CatalogEntry,
        now: SystemTime,
    ) -> Result<(), VerifierRefusal> {
        let entry = entry.clone();
        let policy = self.policy.clone();
        tokio::task::spawn_blocking(move || {
            publisher_verdict(&verify_entry(&entry, None, &policy, now))
        })
        .await
        .map_err(|e| VerifierRefusal(format!("verification did not complete: {e}")))?
    }
}
