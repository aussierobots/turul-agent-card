//! [`produce`]: the creation API for catalog files.
//!
//! One fixed pipeline: start document, apply the change, [`validate`], canonical bytes, atomic
//! write. Every refusal happens before any byte is written.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use turul_ai_catalog::canonical::{CanonicalError, to_canonical_vec};
use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier, EntryKey, HostIdentifier};
use turul_ai_catalog::json;
use turul_ai_catalog::model::{AiCatalog, ArtifactSource, CatalogEntry, HostInfo};
use turul_ai_catalog::parse::{ParseError, parse};
use turul_ai_catalog::validate::{Report, validate};

/// What a producer command asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// Creates a new catalog file holding no entries.
    Init {
        /// The `host` object of the new catalog, if any.
        host: Option<HostInput>,
    },
    /// Appends an entry to an existing catalog file.
    Add(EntryInput),
    /// Removes the entry with this identifier and version from an existing catalog file.
    Remove(EntryKey),
}

/// Unchecked host flags; `display_name` is required because `host.displayName` MUST be present.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HostInput {
    /// The `host.displayName`.
    pub display_name: String,
    /// The `host.identifier`, such as a DID.
    pub identifier: Option<String>,
    /// The `host.documentationUrl`.
    pub documentation_url: Option<String>,
    /// The `host.logoUrl`.
    pub logo_url: Option<String>,
}

impl HostInput {
    /// Builds host input with the required display name and nothing else.
    pub fn new(display_name: String) -> Self {
        Self {
            display_name,
            identifier: None,
            documentation_url: None,
            logo_url: None,
        }
    }
}

/// Unchecked `add` flags.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EntryInput {
    /// The entry `identifier`.
    pub identifier: String,
    /// The entry `type`, a media type.
    pub artifact_type: String,
    /// Where the artifact content comes from.
    pub content: Content,
    /// The entry `displayName`.
    pub display_name: Option<String>,
    /// The entry `description`.
    pub description: Option<String>,
    /// The entry `version`.
    pub version: Option<String>,
    /// The entry `tags`; none are written when empty.
    pub tags: Vec<String>,
}

impl EntryInput {
    /// Builds entry input with the required members and no optional ones.
    pub fn new(identifier: String, artifact_type: String, content: Content) -> Self {
        Self {
            identifier,
            artifact_type,
            content,
            display_name: None,
            description: None,
            version: None,
            tags: Vec::new(),
        }
    }
}

/// Where an entry's artifact content comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Content {
    /// A reference to the artifact, written as the entry `url`.
    Url(String),
    /// A file holding one JSON text, embedded as `data`.
    DataFile(PathBuf),
}

/// Why [`produce`] refused a change.
#[derive(Debug)]
#[non_exhaustive]
pub enum ProduceError {
    /// `init` on a path that exists.
    Exists,
    /// A file could not be read, or the catalog could not be written.
    Io(std::io::Error),
    /// The existing catalog file does not parse.
    Parse(ParseError),
    /// The `--data` file is not a JSON text.
    Data(ParseError),
    /// The catalog carries a top-level `signature`.
    Signed,
    /// `remove` matched no entry.
    NoMatch,
    /// The document the change would write has an error diagnostic.
    Invalid(Report),
    /// The document cannot be written as canonical JSON.
    NotIJson(CanonicalError),
}

impl fmt::Display for ProduceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProduceError::Exists => f.write_str("the file already exists"),
            ProduceError::Io(e) => write!(f, "{e}"),
            ProduceError::Parse(e) => write!(f, "the catalog file does not parse: {e}"),
            ProduceError::Data(e) => write!(f, "the data file is not JSON: {e}"),
            ProduceError::Signed => f.write_str(
                "the catalog carries a signature; a changed catalog needs a new signature",
            ),
            ProduceError::NoMatch => f.write_str("no entry matches the identifier and version"),
            ProduceError::Invalid(_) => f.write_str("the catalog would have an error diagnostic"),
            ProduceError::NotIJson(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ProduceError {}

/// Applies `change` to the catalog file at `path` and writes the canonical bytes back.
///
/// The write is atomic: a temporary file in the same directory is synced and renamed over
/// `path`. Returns the [`Report`] of the written document, which holds warnings only.
///
/// # Errors
///
/// Returns [`ProduceError::Exists`] for [`Change::Init`] on an existing path,
/// [`ProduceError::Io`] when a file cannot be read or written, [`ProduceError::Parse`] when the
/// existing catalog does not parse, [`ProduceError::Data`] when a [`Content::DataFile`] is not
/// JSON, [`ProduceError::Signed`] when the existing catalog carries a `signature`,
/// [`ProduceError::NoMatch`] when [`Change::Remove`] matches no entry,
/// [`ProduceError::Invalid`] when the result would have an error diagnostic, and
/// [`ProduceError::NotIJson`] when it cannot be canonicalised. Nothing is written on error.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog_client::{Change, Content, EntryInput, HostInput, produce};
///
/// let dir = std::env::temp_dir().join(format!("ai-catalog-produce-doc-{}", std::process::id()));
/// std::fs::create_dir_all(&dir)?;
/// let file = dir.join("ai-catalog.json");
/// # let _ = std::fs::remove_file(&file);
///
/// let host = HostInput::new("Example Corp".into());
/// produce(&file, &Change::Init { host: Some(host) })?;
/// let entry = EntryInput::new(
///     "urn:air:example.com:mcp:weather".into(),
///     "application/mcp-server-card+json".into(),
///     Content::Url("https://mcp.example.com/.well-known/mcp/server-card.json".into()),
/// );
/// let report = produce(&file, &Change::Add(entry))?;
/// assert!(!report.has_errors());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn produce(path: &Path, change: &Change) -> Result<Report, ProduceError> {
    let mut catalog = match change {
        Change::Init { .. } => {
            if path.try_exists().map_err(ProduceError::Io)? {
                return Err(ProduceError::Exists);
            }
            AiCatalog::new(Vec::new())
        }
        Change::Add(_) | Change::Remove(_) => {
            let bytes = fs::read(path).map_err(ProduceError::Io)?;
            let catalog = parse(&bytes).map_err(ProduceError::Parse)?;
            if catalog.signature.is_some() {
                return Err(ProduceError::Signed);
            }
            catalog
        }
    };
    apply(&mut catalog, change)?;
    let report = validate(&catalog);
    if report.has_errors() {
        return Err(ProduceError::Invalid(report));
    }
    let bytes = to_canonical_vec(&catalog).map_err(ProduceError::NotIJson)?;
    write_atomically(path, &bytes).map_err(ProduceError::Io)?;
    Ok(report)
}

fn apply(catalog: &mut AiCatalog, change: &Change) -> Result<(), ProduceError> {
    match change {
        Change::Init { host } => {
            catalog.host = host.as_ref().map(|h| {
                let mut info = HostInfo::new(h.display_name.clone());
                info.identifier = h.identifier.clone().map(HostIdentifier::new);
                info.documentation_url = h.documentation_url.clone();
                info.logo_url = h.logo_url.clone();
                info
            });
        }
        Change::Add(input) => {
            let source = match &input.content {
                Content::Url(url) => ArtifactSource::Url(url.clone()),
                Content::DataFile(file) => {
                    let bytes = fs::read(file).map_err(ProduceError::Io)?;
                    ArtifactSource::Data(json::read(&bytes).map_err(ProduceError::Data)?)
                }
            };
            let mut entry = CatalogEntry::new(
                EntryIdentifier::new(input.identifier.clone()),
                ArtifactType::new(input.artifact_type.clone()),
                source,
            );
            entry.display_name = input.display_name.clone();
            entry.description = input.description.clone();
            entry.version = input.version.clone();
            entry.tags = (!input.tags.is_empty()).then(|| input.tags.clone());
            catalog.entries.push(entry);
        }
        Change::Remove(key) => {
            let before = catalog.entries.len();
            catalog.entries.retain(|e| e.key() != *key);
            if catalog.entries.len() == before {
                return Err(ProduceError::NoMatch);
            }
        }
    }
    Ok(())
}

/// Temporary file in the same directory, `sync_all`, then `rename` over `path`.
fn write_atomically(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut name = OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(format!(".tmp-{}", std::process::id()));
    let temp = path.with_file_name(name);
    let result = (|| {
        let mut file = File::create(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
