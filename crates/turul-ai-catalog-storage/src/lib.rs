#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod assembly;
mod ids;
mod import;
mod operator;
mod owned;
mod provenance;
mod records;
mod registration;
mod store;

#[cfg(feature = "dynamodb")]
mod dynamodb;
#[cfg(feature = "in-memory")]
mod in_memory;
#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "sqlite")]
mod sqlite;

pub use assembly::{AssembleError, Assembled, Omission, OmissionReason, assemble};
pub use ids::{CatalogId, Etag, RecordKey, WriterId};
pub use import::{ImportError, ImportMode, import};
pub use operator::{Expiry, Registration, RemoveError, list_registrations, remove_registration};
pub use provenance::{ModeKind, Provenance, ProvenanceError};
pub use records::{EntryRecord, HostRecord, RecordError};
pub use registration::{
    ModeRequest, PolicyError, PublisherVerifier, Registered, Registrar, RegistrationError,
    RegistrationPolicy, VerifierRefusal, WriterGrant,
};
pub use store::{
    CatalogStore, Condition, ItemCounts, PurgeReport, ScannedEntry, StoreError, Stored, StoredEntry,
};

#[cfg(feature = "in-memory")]
pub use in_memory::InMemoryCatalogStore;

#[cfg(feature = "sqlite")]
pub use sqlite::SqliteCatalogStore;

#[cfg(feature = "postgres")]
pub use postgres::PostgresCatalogStore;

#[cfg(feature = "dynamodb")]
pub use dynamodb::DynamoDbCatalogStore;
