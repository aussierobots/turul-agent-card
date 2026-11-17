//! Identity values of the format.

use std::borrow::Cow;

use crate::model::CatalogEntry;

/// The AI Catalog specification version this crate produces and the only major it accepts.
pub const SPEC_VERSION: &str = "1.0";

/// The AI Catalog media type (#media-type).
pub const MEDIA_TYPE: &str = "application/ai-catalog+json";

/// The well-known path (#well-known-uri).
pub const WELL_KNOWN_PATH: &str = "/.well-known/ai-catalog.json";

/// An entry or subject `identifier`: opaque text, compared exactly (#catalog-entry).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryIdentifier(String);

impl EntryIdentifier {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The host `identifier`: opaque text, never treated as verified (#host-info).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HostIdentifier(String);

impl HostIdentifier {
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An entry or subject `type`: open text, compared exactly (#catalog-entry).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ArtifactType(Cow<'static, str>);

impl ArtifactType {
    pub const AI_CATALOG: Self = Self(Cow::Borrowed(MEDIA_TYPE));
    pub const AGENT_CARD: Self = Self(Cow::Borrowed("application/agent-card+json"));
    pub const A2A_AGENT_CARD: Self = Self(Cow::Borrowed("application/a2a-agent-card+json"));
    pub const MCP_SERVER_CARD: Self = Self(Cow::Borrowed("application/mcp-server-card+json"));
    pub const AGENT_SKILLS_JSON: Self = Self(Cow::Borrowed("application/agent-skills+json"));
    pub const AGENT_SKILLS_MD: Self = Self(Cow::Borrowed("application/agent-skills+md"));
    pub const AGENT_SKILLS_ZIP: Self = Self(Cow::Borrowed("application/agent-skills+zip"));
    pub const AGENT_SKILLS_GZIP: Self = Self(Cow::Borrowed("application/agent-skills+gzip"));
    pub const AGENT_PLUGINS_ZIP: Self = Self(Cow::Borrowed("application/agent-plugins+zip"));
    pub const AGENT_PLUGINS_GZIP: Self = Self(Cow::Borrowed("application/agent-plugins+gzip"));

    pub fn new(text: impl Into<String>) -> Self {
        Self(Cow::Owned(text.into()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The identity of an entry: `identifier` and `version` as written (#multi-version-entries).
/// Built only by [`EntryKey::new`], which [`CatalogEntry::key`] calls.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryKey {
    identifier: EntryIdentifier,
    version: Option<String>,
}

impl EntryKey {
    pub fn new(identifier: EntryIdentifier, version: Option<String>) -> Self {
        Self {
            identifier,
            version,
        }
    }

    pub(crate) fn of(entry: &CatalogEntry) -> Self {
        Self::new(entry.identifier.clone(), entry.version.clone())
    }

    pub fn identifier(&self) -> &EntryIdentifier {
        &self.identifier
    }

    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}
