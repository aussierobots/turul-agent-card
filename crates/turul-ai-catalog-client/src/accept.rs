//! Acceptance of a fetched catalog response: status, media type and parse.

use std::fmt;

use turul_ai_catalog::identity::MEDIA_TYPE;
use turul_ai_catalog::model::AiCatalog;
use turul_ai_catalog::parse::{ParseError, parse};

#[cfg(doc)]
use crate::fetch::FetchPolicy;
use crate::fetch::Fetched;

const GENERIC_JSON: &str = "application/json";

/// A non-fatal retrieval outcome; it describes how a catalog was obtained, not what it contains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LoadWarning {
    /// The catalog was served as `application/json` rather than `application/ai-catalog+json`.
    MediaTypeGeneric,
    /// `skipped` nested catalogs were not fetched because the `limit` requests were spent.
    RequestBudget {
        /// The request limit, [`FetchPolicy::max_requests`].
        limit: u32,
        /// How many nested catalogs were left unfetched.
        skipped: usize,
    },
}

impl LoadWarning {
    /// Returns the stable kebab-case code of this warning, as used in JSON output.
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

/// A response that passed [`accept_response`].
#[derive(Debug)]
pub struct Accepted {
    /// The parsed catalog.
    pub catalog: AiCatalog,
    /// Retrieval warnings; empty when the response was fully conformant.
    pub warnings: Vec<LoadWarning>,
}

/// Why [`accept_response`] refused a response.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum AcceptError {
    /// The status was not 200.
    Status(u16),
    /// The media type was neither `application/ai-catalog+json` nor `application/json`;
    /// holds the raw `Content-Type` value, or `None` when the header was absent.
    MediaType(Option<String>),
    /// The body is not a parseable catalog.
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

/// Returns the media type of a raw `Content-Type` value: everything before the first `;`,
/// trimmed.
pub fn media_type_essence(content_type: &str) -> &str {
    content_type.split(';').next().unwrap_or("").trim()
}

/// Accepts a fetched response as a catalog.
///
/// The status must be 200 and the media type `application/ai-catalog+json`, or the generic
/// `application/json`, which is accepted with a [`LoadWarning::MediaTypeGeneric`]. The media
/// type is compared case-insensitively and ignores parameters.
///
/// # Errors
///
/// Returns [`AcceptError::Status`] for any status other than 200, [`AcceptError::MediaType`]
/// for a missing or other media type, and [`AcceptError::Parse`] when the body does not parse.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog_client::{Fetched, LoadWarning, accept_response};
///
/// let body = br#"{"specVersion":"1.0","entries":[]}"#.to_vec();
/// let fetched = Fetched::new(200, Some("application/json; charset=utf-8".into()), body);
/// let accepted = accept_response(&fetched)?;
/// assert_eq!(accepted.warnings, [LoadWarning::MediaTypeGeneric]);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
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
