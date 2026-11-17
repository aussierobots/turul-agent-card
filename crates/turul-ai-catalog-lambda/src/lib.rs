//! Serves one bundled AI Catalog at the well-known URI.

use std::fmt;

use lambda_http::http::{Method, StatusCode, header};
use lambda_http::{Body, Request, Response};
use turul_ai_catalog::identity::{MEDIA_TYPE, WELL_KNOWN_PATH};
use turul_ai_catalog::parse::{ParseError, parse};
use turul_ai_catalog::validate::{Report, Severity, validate};

/// The only catalog this crate serves, compiled in.
pub const BUNDLE: &[u8] = include_bytes!("../catalog.json");

/// Value of `Allow` on a 405 (RFC 9110 section 15.5.6).
pub const ALLOW: &str = "GET, HEAD";

/// Why a bundle cannot be served.
#[derive(Debug)]
#[non_exhaustive]
pub enum ColdStartError {
    Parse(ParseError),
    Invalid(Report),
}

impl fmt::Display for ColdStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(e) => write!(f, "bundle does not parse: {e} at '{}'", e.pointer),
            Self::Invalid(report) => {
                f.write_str("bundle is invalid:")?;
                for d in report
                    .diagnostics()
                    .iter()
                    .filter(|d| d.severity == Severity::Error)
                {
                    write!(f, " {} at '{}';", d.code, d.path)?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ColdStartError {}

/// Bytes that passed parse and validate with no error diagnostic.
#[derive(Debug)]
pub struct ServedCatalog {
    bytes: &'static [u8],
}

impl ServedCatalog {
    /// The one validation gate: a parse error or any error diagnostic fails.
    pub fn load(bytes: &'static [u8]) -> Result<Self, ColdStartError> {
        let catalog = parse(bytes).map_err(ColdStartError::Parse)?;
        let report = validate(&catalog);
        if report.has_errors() {
            return Err(ColdStartError::Invalid(report));
        }
        Ok(Self { bytes })
    }
}

fn empty(status: StatusCode) -> lambda_http::http::response::Builder {
    Response::builder().status(status)
}

/// Routes by path, then method. Pure: no I/O, clock or state.
pub fn handle(served: &ServedCatalog, request: &Request) -> Response<Body> {
    let built = if request.uri().path() != WELL_KNOWN_PATH {
        empty(StatusCode::NOT_FOUND).body(Body::Empty)
    } else if request.method() == Method::GET {
        empty(StatusCode::OK)
            .header(header::CONTENT_TYPE, MEDIA_TYPE)
            .body(Body::Binary(served.bytes.to_vec()))
    } else if request.method() == Method::HEAD {
        empty(StatusCode::OK)
            .header(header::CONTENT_TYPE, MEDIA_TYPE)
            .body(Body::Empty)
    } else {
        empty(StatusCode::METHOD_NOT_ALLOWED)
            .header(header::ALLOW, ALLOW)
            .body(Body::Empty)
    };
    built.expect("static response parts are valid")
}
