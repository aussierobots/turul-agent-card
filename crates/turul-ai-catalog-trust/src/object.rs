//! The two kinds of object that carry a signature.

use turul_ai_catalog::model::{AiCatalog, Signature, TrustManifest};

/// A trust manifest or a catalog root, the two objects that carry a signed payload.
#[derive(Clone, Copy, Debug)]
pub enum SignedObject<'a> {
    /// A trust manifest.
    Manifest(&'a TrustManifest),
    /// A catalog root.
    Catalog(&'a AiCatalog),
}

impl SignedObject<'_> {
    /// Returns the object's immediate `signature`, if it has one.
    pub fn signature(&self) -> Option<&Signature> {
        match self {
            Self::Manifest(m) => m.signature.as_ref(),
            Self::Catalog(c) => c.signature.as_ref(),
        }
    }
}
