#![doc = include_str!("../README.md")]
#![warn(missing_docs)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod assembly;
mod ids;
mod import;
mod records;
mod store;

#[cfg(feature = "dynamodb")]
mod dynamodb;
#[cfg(feature = "in-memory")]
mod in_memory;
#[cfg(feature = "postgres")]
mod postgres;
#[cfg(feature = "sqlite")]
mod sqlite;

pub use assembly::{AssembleError, Assembled, assemble};
pub use ids::{CatalogId, Etag, RecordKey};
pub use import::{ImportError, import};
pub use records::{EntryRecord, HostRecord, RecordError};
pub use store::{CatalogStore, Condition, StoreError, Stored};

#[cfg(feature = "in-memory")]
pub use in_memory::InMemoryCatalogStore;

#[cfg(feature = "sqlite")]
pub use sqlite::SqliteCatalogStore;

#[cfg(feature = "postgres")]
pub use postgres::PostgresCatalogStore;

#[cfg(feature = "dynamodb")]
pub use dynamodb::DynamoDbCatalogStore;
