#![doc = include_str!("../README.md")]
//!
//! # Modules
//!
//! - [`parse`]: strict bytes-to-model reading ([`parse::parse`]).
//! - [`model`]: the document types ([`model::AiCatalog`], [`model::CatalogEntry`], ...).
//! - [`validate`]: content rules the schema cannot express ([`validate::validate`]).
//! - [`canonical`]: RFC 8785 serialisation ([`canonical::to_canonical_vec`]).
//! - [`identity`]: identifiers, artifact types and entry keys.
//! - [`resolve`]: display-name, description, version and latest-entry resolution.
//! - [`error`]: rendering an error with its cause chain ([`error::chain`]).
//! - [`digest`], [`timestamp`], [`uri`], [`extensions`] and [`json`]: the value parsers the
//!   other modules share.
#![warn(missing_docs)]

pub mod canonical;
pub mod digest;
pub mod error;
pub mod extensions;
pub mod identity;
pub mod json;
pub mod model;
mod names;
mod number;
pub mod parse;
pub mod resolve;
mod semver;
pub mod timestamp;
mod to_json;
pub mod uri;
pub mod validate;
