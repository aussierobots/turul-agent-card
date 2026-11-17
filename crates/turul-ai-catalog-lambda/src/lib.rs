//! Serves the AI Catalog assembled from a catalog store at the well-known URI.

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lambda_http::http::{Method, StatusCode, header};
use lambda_http::{Body, Request, Response};
use turul_ai_catalog::identity::{MEDIA_TYPE, WELL_KNOWN_PATH};
use turul_ai_catalog::validate::Severity;
use turul_ai_catalog_storage::{AssembleError, CatalogId, CatalogStore, assemble};

/// Value of `Allow` on a 405 (RFC 9110 section 15.5.6).
pub const ALLOW: &str = "GET, HEAD";

/// Environment variable naming the catalog in the store.
pub const ID_VAR: &str = "TURUL_AI_CATALOG_ID";
/// Environment variable naming the DynamoDB table.
pub const TABLE_VAR: &str = "TURUL_AI_CATALOG_TABLE";
/// Environment variable holding the snapshot refresh interval in seconds.
pub const MAX_AGE_VAR: &str = "TURUL_AI_CATALOG_MAX_AGE_SECS";

const DEFAULT_MAX_AGE: Duration = Duration::from_secs(60);

/// Why the function cannot start or refresh.
#[derive(Debug)]
#[non_exhaustive]
pub enum ColdStartError {
    Assemble(AssembleError),
    Config(&'static str),
}

impl fmt::Display for ColdStartError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Assemble(AssembleError::Invalid(report)) => {
                f.write_str("assembled catalog is invalid:")?;
                for d in report
                    .diagnostics()
                    .iter()
                    .filter(|d| d.severity == Severity::Error)
                {
                    write!(f, " {} at '{}';", d.code, d.path)?;
                }
                Ok(())
            }
            Self::Assemble(e) => write!(f, "cannot assemble the catalog: {e}"),
            Self::Config(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ColdStartError {}

impl From<AssembleError> for ColdStartError {
    fn from(e: AssembleError) -> Self {
        Self::Assemble(e)
    }
}

/// The catalog id from `TURUL_AI_CATALOG_ID`; `raw` is `None` when the variable is unset.
pub fn catalog_id(raw: Option<&str>) -> Result<CatalogId, ColdStartError> {
    let raw = raw.ok_or(ColdStartError::Config("TURUL_AI_CATALOG_ID is not set"))?;
    CatalogId::new(raw)
        .map_err(|_| ColdStartError::Config("TURUL_AI_CATALOG_ID is not a valid catalog id"))
}

/// The table name from `TURUL_AI_CATALOG_TABLE`.
#[cfg(feature = "dynamodb")]
pub fn table_name(raw: Option<&str>) -> Result<String, ColdStartError> {
    match raw {
        Some(name) if !name.is_empty() => Ok(name.to_string()),
        _ => Err(ColdStartError::Config("TURUL_AI_CATALOG_TABLE is not set")),
    }
}

/// The refresh interval from `TURUL_AI_CATALOG_MAX_AGE_SECS`: whole seconds, 60 when unset.
pub fn max_age(raw: Option<&str>) -> Result<Duration, ColdStartError> {
    match raw {
        None => Ok(DEFAULT_MAX_AGE),
        Some(text) => text.parse().map(Duration::from_secs).map_err(|_| {
            ColdStartError::Config("TURUL_AI_CATALOG_MAX_AGE_SECS is not a whole number of seconds")
        }),
    }
}

/// Canonical bytes of an assembled catalog that passed `validate` with no error diagnostic.
#[derive(Debug)]
pub struct ServedCatalog {
    bytes: Vec<u8>,
}

impl ServedCatalog {
    /// Assembles the catalog from the store; any `AssembleError` fails.
    pub async fn assemble(
        store: &dyn CatalogStore,
        catalog: &CatalogId,
    ) -> Result<Self, ColdStartError> {
        let assembled = assemble(store, catalog).await?;
        Ok(Self {
            bytes: assembled.bytes,
        })
    }
}

/// The last good snapshot and when it was assembled. The clock lives here, never in `handle`.
pub struct SnapshotCache {
    max_age: Duration,
    state: Mutex<(Arc<ServedCatalog>, Instant)>,
}

impl SnapshotCache {
    pub fn new(served: ServedCatalog, max_age: Duration) -> Self {
        Self {
            max_age,
            state: Mutex::new((Arc::new(served), Instant::now())),
        }
    }

    /// The snapshot to serve: re-assembled when its age is at least `max_age`, and kept as it is
    /// when re-assembly fails, in which case the failure is returned beside it so the caller can
    /// report it, and the next attempt waits another `max_age`.
    pub async fn current(
        &self,
        store: &dyn CatalogStore,
        catalog: &CatalogId,
    ) -> (Arc<ServedCatalog>, Option<ColdStartError>) {
        let (served, assembled_at) = self.read();
        if assembled_at.elapsed() < self.max_age {
            return (served, None);
        }
        match ServedCatalog::assemble(store, catalog).await {
            Ok(fresh) => {
                let fresh = Arc::new(fresh);
                *self.state.lock().unwrap_or_else(|p| p.into_inner()) =
                    (Arc::clone(&fresh), Instant::now());
                (fresh, None)
            }
            Err(e) => {
                // Keep the last good snapshot but restart its age, so a lasting failure is retried
                // and reported once per `max_age`, not on every request.
                self.state.lock().unwrap_or_else(|p| p.into_inner()).1 = Instant::now();
                (served, Some(e))
            }
        }
    }

    fn read(&self) -> (Arc<ServedCatalog>, Instant) {
        let state = self.state.lock().unwrap_or_else(|p| p.into_inner());
        (Arc::clone(&state.0), state.1)
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
            .body(Body::Binary(served.bytes.clone()))
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
