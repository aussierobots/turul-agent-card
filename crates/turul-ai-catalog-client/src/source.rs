//! Catalog sources and the one well-known URL constructor.

use std::fmt;
use std::path::PathBuf;

use turul_ai_catalog::identity::WELL_KNOWN_PATH;
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Source {
    File(PathBuf),
    Url(String),
    WellKnown(WellKnownHost),
}

impl Source {
    /// The URL a request for this source goes to; `None` for a file.
    pub fn request_url(&self) -> Option<String> {
        match self {
            Source::File(_) => None,
            Source::Url(u) => Some(u.clone()),
            Source::WellKnown(h) => Some(h.url.clone()),
        }
    }
}

/// A host with an optional port and nothing else.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WellKnownHost {
    url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidHost;

impl fmt::Display for InvalidHost {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("well-known host must be a host with an optional port")
    }
}

impl std::error::Error for InvalidHost {}

impl WellKnownHost {
    pub fn new(host: &str) -> Result<Self, InvalidHost> {
        let u = Url::parse(&format!("https://{host}{WELL_KNOWN_PATH}")).map_err(|_| InvalidHost)?;
        let plain = u.host_str().is_some()
            && u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none()
            && u.path() == WELL_KNOWN_PATH;
        if plain {
            Ok(WellKnownHost { url: u.into() })
        } else {
            Err(InvalidHost)
        }
    }
}
