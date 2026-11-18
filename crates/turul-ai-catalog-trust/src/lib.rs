#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub mod accept;
pub mod did_web;
pub mod digest;
pub mod endorse;
pub mod header;
pub mod identity;
pub mod object;
#[cfg(feature = "ssm")]
pub mod parameter_store;
pub mod payload;
#[cfg(feature = "registration")]
pub mod registration;
pub mod report;
pub mod sign;

pub use accept::{AcceptedSignature, Reason, TrustPolicy, verify_signature};
pub use did_web::{
    DidDocument, KeySelectionError, PublicJwk, ResolveError, accept_document, did_document,
    document_url, resolve, select_key,
};
pub use digest::{IntegrityError, Unsupported, artifact_digest, compute, verify_artifact};
pub use endorse::{EndorseError, EndorseErrorKind, endorse_catalog, endorse_entry};
pub use header::{
    DID_WEB_V1_FORBIDDEN_HEADERS, ES256, HeaderError, Kid, TYP_CATALOG, TYP_TRUST_MANIFEST,
    inspect, kid_matches_signer, policy,
};
pub use identity::DidWebRoot;
pub use object::SignedObject;
#[cfg(feature = "ssm")]
pub use parameter_store::{KeyStoreError, ParameterKeyStore};
pub use payload::{PayloadError, catalog_payload, manifest_payload};
#[cfg(feature = "registration")]
pub use registration::{DidWebPublisherVerifier, publisher_verdict};
pub use report::{
    CatalogReport, Coverage, CoverageGap, DidDocuments, Endorsement, EntryReport, ManifestReport,
    Selection, SignatureOutcome, TrustReport, catalog_endorsed, entry_release_coverage,
    publisher_authorized, select_current, verify, verify_entry, verify_entry_resolved,
    verify_resolved,
};
pub use sign::{SignError, SignerParams, SigningKey, sign_catalog, sign_manifest};
