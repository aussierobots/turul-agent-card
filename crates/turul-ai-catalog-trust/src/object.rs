//! The kind of object a signature is contained in.

use turul_ai_catalog::model::{AiCatalog, Signature, TrustManifest};

/// A trust manifest or a catalog root, the two things that carry a signed payload.
#[derive(Clone, Copy, Debug)]
pub enum SignedObject<'a> {
    Manifest(&'a TrustManifest),
    Catalog(&'a AiCatalog),
}

impl SignedObject<'_> {
    /// The object's immediate `signature`.
    pub fn signature(&self) -> Option<&Signature> {
        match self {
            Self::Manifest(m) => m.signature.as_ref(),
            Self::Catalog(c) => c.signature.as_ref(),
        }
    }
}
