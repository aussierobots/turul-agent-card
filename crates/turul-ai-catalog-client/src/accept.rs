//! The one catalog response gate.

use std::fmt;

use turul_ai_catalog::identity::MEDIA_TYPE;
use turul_ai_catalog::model::AiCatalog;
use turul_ai_catalog::parse::{ParseError, parse};

use crate::fetch::Fetched;

const GENERIC_JSON: &str = "application/json";

/// A retrieval outcome, not a document diagnostic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadWarning {
    MediaTypeGeneric,
    /// `skipped` nested catalogs were not fetched because the `limit` requests were spent.
    RequestBudget {
        limit: u32,
        skipped: usize,
    },
}

impl LoadWarning {
    pub fn code(&self) -> &'static str {
        match self {
            LoadWarning::MediaTypeGeneric => "media-type-generic",
            LoadWarning::RequestBudget { .. } => "request-budget",
        }
    }
}

impl fmt::Display for LoadWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadWarning::MediaTypeGeneric => {
                write!(f, "served as {GENERIC_JSON}, not {MEDIA_TYPE}")
            }
            LoadWarning::RequestBudget { limit, skipped } => write!(
                f,
                "request budget of {limit} spent; {skipped} nested catalog(s) not fetched"
            ),
        }
    }
}

#[derive(Debug)]
pub struct Accepted {
    pub catalog: AiCatalog,
    pub warnings: Vec<LoadWarning>,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum AcceptError {
    Status(u16),
    MediaType(Option<String>),
    Parse(ParseError),
}

impl fmt::Display for AcceptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AcceptError::Status(s) => write!(f, "unexpected status {s}"),
            AcceptError::MediaType(Some(m)) => write!(f, "unsupported media type {m:?}"),
            AcceptError::MediaType(None) => f.write_str("missing Content-Type"),
            AcceptError::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for AcceptError {}

/// The media type of a raw `Content-Type` value: everything before the first `;`, trimmed.
pub fn media_type_essence(content_type: &str) -> &str {
    content_type.split(';').next().unwrap_or("").trim()
}

pub fn accept_response(fetched: &Fetched) -> Result<Accepted, AcceptError> {
    if fetched.status != 200 {
        return Err(AcceptError::Status(fetched.status));
    }
    let raw = fetched.content_type.as_deref();
    let essence = raw.map(media_type_essence);
    let mut warnings = Vec::new();
    match essence {
        Some(e) if e.eq_ignore_ascii_case(MEDIA_TYPE) => {}
        Some(e) if e.eq_ignore_ascii_case(GENERIC_JSON) => {
            warnings.push(LoadWarning::MediaTypeGeneric)
        }
        _ => return Err(AcceptError::MediaType(raw.map(String::from))),
    }
    let catalog = parse(&fetched.body).map_err(AcceptError::Parse)?;
    Ok(Accepted { catalog, warnings })
}
