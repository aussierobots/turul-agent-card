#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lambda_http::http::{Method, StatusCode, header};
use lambda_http::{Body, Request, Response};
use turul_ai_catalog::identity::{MEDIA_TYPE, WELL_KNOWN_PATH};
use turul_ai_catalog::model::AiCatalog;
use turul_ai_catalog::validate::Severity;
use turul_ai_catalog_storage::{
    AssembleError, CatalogId, CatalogStore, ImportError, InMemoryCatalogStore, assemble, import,
};

/// Value of the `Allow` header on a 405 response (RFC 9110 section 15.5.6).
pub const ALLOW: &str = "GET, HEAD";

/// Environment variable naming the catalog in the store.
pub const ID_VAR: &str = "TURUL_AI_CATALOG_ID";
/// Environment variable naming the DynamoDB table.
pub const TABLE_VAR: &str = "TURUL_AI_CATALOG_TABLE";
/// Environment variable holding the snapshot refresh interval in seconds.
pub const MAX_AGE_VAR: &str = "TURUL_AI_CATALOG_MAX_AGE_SECS";

const DEFAULT_MAX_AGE: Duration = Duration::from_secs(60);

/// Why the function cannot start or refresh its snapshot.
#[derive(Debug)]
#[non_exhaustive]
pub enum ColdStartError {
    /// The store could not be read, or the assembled catalog is invalid or not canonicalisable.
    Assemble(AssembleError),
    /// A carried document cannot be stored, e.g. it is signed or has catalog-level extensions.
    Import(ImportError),
    /// An environment variable is missing or malformed; the message names it.
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
            Self::Import(e) => write!(f, "cannot import the catalog: {e}"),
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

/// Parses the catalog id from the value of [`ID_VAR`]; `raw` is `None` when the variable is unset.
///
/// # Errors
///
/// Returns [`ColdStartError::Config`] when `raw` is `None` or is not a valid [`CatalogId`].
///
/// # Examples
///
/// ```
/// use turul_ai_catalog_lambda::catalog_id;
///
/// let id = catalog_id(Some("default"))?;
/// assert_eq!(id.as_str(), "default");
/// assert!(catalog_id(None).is_err());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn catalog_id(raw: Option<&str>) -> Result<CatalogId, ColdStartError> {
    let raw = raw.ok_or(ColdStartError::Config("TURUL_AI_CATALOG_ID is not set"))?;
    CatalogId::new(raw)
        .map_err(|_| ColdStartError::Config("TURUL_AI_CATALOG_ID is not a valid catalog id"))
}

/// Parses the DynamoDB table name from the value of [`TABLE_VAR`].
///
/// # Errors
///
/// Returns [`ColdStartError::Config`] when `raw` is `None` or empty.
#[cfg(feature = "dynamodb")]
pub fn table_name(raw: Option<&str>) -> Result<String, ColdStartError> {
    match raw {
        Some(name) if !name.is_empty() => Ok(name.to_string()),
        _ => Err(ColdStartError::Config("TURUL_AI_CATALOG_TABLE is not set")),
    }
}

/// Parses the snapshot refresh interval from the value of [`MAX_AGE_VAR`]: whole seconds, 60 when
/// unset.
///
/// # Errors
///
/// Returns [`ColdStartError::Config`] when `raw` is not a whole number of seconds.
///
/// # Examples
///
/// ```
/// use std::time::Duration;
/// use turul_ai_catalog_lambda::max_age;
///
/// assert_eq!(max_age(None)?, Duration::from_secs(60));
/// assert_eq!(max_age(Some("5"))?, Duration::from_secs(5));
/// assert!(max_age(Some("soon")).is_err());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn max_age(raw: Option<&str>) -> Result<Duration, ColdStartError> {
    match raw {
        None => Ok(DEFAULT_MAX_AGE),
        Some(text) => text.parse().map(Duration::from_secs).map_err(|_| {
            ColdStartError::Config("TURUL_AI_CATALOG_MAX_AGE_SECS is not a whole number of seconds")
        }),
    }
}

/// Canonical bytes of an assembled catalog that passed [`validate`](turul_ai_catalog::validate::validate)
/// with no error diagnostic.
#[derive(Debug)]
pub struct ServedCatalog {
    bytes: Vec<u8>,
}

impl ServedCatalog {
    /// Assembles the catalog from the store with [`assemble`].
    ///
    /// # Errors
    ///
    /// Returns [`ColdStartError::Assemble`] for any [`AssembleError`]: a store failure, an error
    /// diagnostic in the assembled document, or a document that is not canonicalisable.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog_lambda::ServedCatalog;
    /// use turul_ai_catalog_storage::{CatalogId, InMemoryCatalogStore};
    ///
    /// # futures::executor::block_on(async {
    /// let store = InMemoryCatalogStore::new();
    /// let catalog = CatalogId::new("default")?;
    /// let served = ServedCatalog::assemble(&store, &catalog).await?;
    /// # let _ = served;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// # })?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub async fn assemble(
        store: &dyn CatalogStore,
        catalog: &CatalogId,
    ) -> Result<Self, ColdStartError> {
        let assembled = assemble(store, catalog).await?;
        Ok(Self {
            bytes: assembled.bytes,
        })
    }

    /// Builds the served catalog from a caller-supplied document: [`import`] into an in-memory
    /// store, then [`ServedCatalog::assemble`]. Reads no file and adds nothing to the document, so
    /// a function that compiles its catalog in and the local server share this one path.
    ///
    /// # Errors
    ///
    /// Returns [`ColdStartError::Import`] when the document cannot be stored (for example it
    /// carries a catalog `signature`), and [`ColdStartError::Assemble`] when the assembled
    /// document has an error diagnostic.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog::parse::parse;
    /// use turul_ai_catalog_lambda::ServedCatalog;
    ///
    /// # futures::executor::block_on(async {
    /// let document = parse(br#"{"specVersion":"1.0","entries":[{"identifier":"urn:air:example.com:mcp:weather","type":"application/mcp-server-card+json","url":"https://mcp.example.com/server-card.json"}]}"#)?;
    /// let served = ServedCatalog::from_document(&document).await?;
    /// # let _ = served;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// # })?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub async fn from_document(document: &AiCatalog) -> Result<Self, ColdStartError> {
        let store = InMemoryCatalogStore::new();
        // A fixed, valid id: the store is private to this call, so the id is never observed.
        let catalog =
            CatalogId::new("default").map_err(|_| ColdStartError::Config("invalid catalog id"))?;
        import(&store, &catalog, document)
            .await
            .map_err(ColdStartError::Import)?;
        Self::assemble(&store, &catalog).await
    }
}

/// The last good [`ServedCatalog`] and when it was assembled. The clock lives here, never in
/// [`handle`].
pub struct SnapshotCache {
    max_age: Duration,
    state: Mutex<(Arc<ServedCatalog>, Instant)>,
}

impl SnapshotCache {
    /// Creates a cache holding `served`, assembled now, that refreshes once it is `max_age` old.
    pub fn new(served: ServedCatalog, max_age: Duration) -> Self {
        Self {
            max_age,
            state: Mutex::new((Arc::new(served), Instant::now())),
        }
    }

    /// Returns the snapshot to serve, re-assembled when its age is at least `max_age`.
    ///
    /// When re-assembly fails the last good snapshot is kept and the failure is returned beside
    /// it so the caller can report it; the next attempt waits another `max_age`.
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

/// Answers a request for the well-known path, routing by path, then method. Pure: no I/O, clock
/// or state.
///
/// `GET` returns the catalog bytes and `HEAD` the same headers with an empty body, both with the
/// catalog media type; other methods on the path return 405 with `Allow`, and other paths 404.
///
/// # Examples
///
/// ```
/// use lambda_http::{Body, http::Request};
/// use turul_ai_catalog_lambda::{ServedCatalog, handle};
/// use turul_ai_catalog_storage::{CatalogId, InMemoryCatalogStore};
///
/// # futures::executor::block_on(async {
/// let store = InMemoryCatalogStore::new();
/// let served = ServedCatalog::assemble(&store, &CatalogId::new("default")?).await?;
///
/// let found = Request::builder().uri("/.well-known/ai-catalog.json").body(Body::Empty)?;
/// assert_eq!(handle(&served, &found).status(), 200);
///
/// let missing = Request::builder().uri("/other").body(Body::Empty)?;
/// assert_eq!(handle(&served, &missing).status(), 404);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// # })?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
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
