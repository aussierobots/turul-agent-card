//! Identity values of the format: spec constants, identifiers, artifact types and entry keys.

use std::borrow::Cow;

use crate::model::CatalogEntry;

/// The AI Catalog specification version this crate produces and the only major it accepts.
pub const SPEC_VERSION: &str = "1.0";

/// The AI Catalog media type ([#media-type](https://ai-catalog.io/specification/#media-type)).
pub const MEDIA_TYPE: &str = "application/ai-catalog+json";

/// The well-known path ([#well-known-uri](https://ai-catalog.io/specification/#well-known-uri)).
pub const WELL_KNOWN_PATH: &str = "/.well-known/ai-catalog.json";

/// An entry or subject `identifier`: opaque text, compared exactly ([#catalog-entry](https://ai-catalog.io/specification/#catalog-entry)).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryIdentifier(String);

impl EntryIdentifier {
    /// Wraps `text` as an entry identifier without interpreting it.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// Returns the identifier text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The host `identifier`: opaque text, never treated as verified ([#host-info](https://ai-catalog.io/specification/#host-info)).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct HostIdentifier(String);

impl HostIdentifier {
    /// Wraps `text` as a host identifier without interpreting it.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// Returns the identifier text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// An entry or subject `type`: open text, compared exactly ([#catalog-entry](https://ai-catalog.io/specification/#catalog-entry)).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ArtifactType(Cow<'static, str>);

impl ArtifactType {
    /// `application/ai-catalog+json`: a nested catalog.
    pub const AI_CATALOG: Self = Self(Cow::Borrowed(MEDIA_TYPE));
    /// `application/agent-card+json`.
    pub const AGENT_CARD: Self = Self(Cow::Borrowed("application/agent-card+json"));
    /// `application/a2a-agent-card+json`: an A2A Agent Card.
    pub const A2A_AGENT_CARD: Self = Self(Cow::Borrowed("application/a2a-agent-card+json"));
    /// `application/mcp-server-card+json`: an MCP Server Card.
    pub const MCP_SERVER_CARD: Self = Self(Cow::Borrowed("application/mcp-server-card+json"));
    /// `application/agent-skills+json`.
    pub const AGENT_SKILLS_JSON: Self = Self(Cow::Borrowed("application/agent-skills+json"));
    /// `application/agent-skills+md`.
    pub const AGENT_SKILLS_MD: Self = Self(Cow::Borrowed("application/agent-skills+md"));
    /// `application/agent-skills+zip`.
    pub const AGENT_SKILLS_ZIP: Self = Self(Cow::Borrowed("application/agent-skills+zip"));
    /// `application/agent-skills+gzip`.
    pub const AGENT_SKILLS_GZIP: Self = Self(Cow::Borrowed("application/agent-skills+gzip"));
    /// `application/agent-plugins+zip`.
    pub const AGENT_PLUGINS_ZIP: Self = Self(Cow::Borrowed("application/agent-plugins+zip"));
    /// `application/agent-plugins+gzip`.
    pub const AGENT_PLUGINS_GZIP: Self = Self(Cow::Borrowed("application/agent-plugins+gzip"));

    /// Wraps `text` as an artifact type; any text is accepted, known or not.
    pub fn new(text: impl Into<String>) -> Self {
        Self(Cow::Owned(text.into()))
    }

    /// Returns the type text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The identity of an entry: `identifier` and `version` as written ([#multi-version-entries](https://ai-catalog.io/specification/#multi-version-entries)).
///
/// Built only by [`EntryKey::new`], which [`CatalogEntry::key`] calls.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EntryKey {
    identifier: EntryIdentifier,
    version: Option<String>,
}

impl EntryKey {
    /// Builds the key of the entry with this `identifier` and `version`.
    pub fn new(identifier: EntryIdentifier, version: Option<String>) -> Self {
        Self {
            identifier,
            version,
        }
    }

    pub(crate) fn of(entry: &CatalogEntry) -> Self {
        Self::new(entry.identifier.clone(), entry.version.clone())
    }

    /// Returns the entry `identifier`.
    pub fn identifier(&self) -> &EntryIdentifier {
        &self.identifier
    }

    /// Returns the entry `version`, when it has one.
    pub fn version(&self) -> Option<&str> {
        self.version.as_deref()
    }
}

const URN_AIR_PREFIX: &str = "urn:air:";

/// Whether `domain` satisfies the root `did:web` domain restrictions: lowercase ASCII LDH labels,
/// no port, IP address spelling, trailing root dot or Unicode U-label
/// ([#the-did-web-signer-profile](https://ai-catalog.io/specification/#the-did-web-signer-profile)).
/// `xn--` A-labels pass as LDH text and are not IDNA-validated.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog::identity::is_root_did_web_domain;
///
/// assert!(is_root_did_web_domain("example.com"));
/// assert!(!is_root_did_web_domain("Example.com"));
/// assert!(!is_root_did_web_domain("example.com:8443"));
/// assert!(!is_root_did_web_domain("10.0.0.1"));
/// ```
pub fn is_root_did_web_domain(domain: &str) -> bool {
    if domain.is_empty() || domain.len() > 253 {
        return false;
    }
    let mut last = "";
    for label in domain.split('.') {
        let b = label.as_bytes();
        let edge_ok = b.first().is_some_and(|c| *c != b'-') && b.last() != Some(&b'-');
        if b.len() > 63
            || !edge_ok
            || !b
                .iter()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
        {
            return false;
        }
        last = label;
    }
    // The WHATWG host parser's "ends in a number": such a host is an IPv4 spelling.
    let hex = last.strip_prefix("0x");
    !(last.bytes().all(|c| c.is_ascii_digit())
        || hex.is_some_and(|h| h.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))))
}

/// A `urn:air` identifier split into publisher, namespace and name. Parsing is grammar only:
/// [`validate`](crate::validate::validate) does not require entry identifiers to be `urn:air`, and
/// the publisher is not checked against any domain rule here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrnAir {
    publisher: String,
    namespace: String,
    name: String,
}

impl UrnAir {
    /// Returns the identifier when `s` is `urn:air:` followed by at least a publisher, a
    /// namespace and a name, all non-empty, else `None`.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog::identity::UrnAir;
    ///
    /// let urn = UrnAir::parse("urn:air:example.com:a2a:research").ok_or("not an urn:air")?;
    /// assert_eq!(urn.publisher(), "example.com");
    /// assert_eq!(urn.namespace(), "a2a");
    /// assert_eq!(urn.name(), "research");
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.strip_prefix(URN_AIR_PREFIX)?.split(':').collect();
        if parts.len() < 3 || parts.iter().any(|p| p.is_empty()) {
            return None;
        }
        Some(Self {
            publisher: parts[0].to_string(),
            namespace: parts[1..parts.len() - 1].join(":"),
            name: parts[parts.len() - 1].to_string(),
        })
    }

    /// Returns the publisher component.
    pub fn publisher(&self) -> &str {
        &self.publisher
    }

    /// Returns every component between publisher and name, `:` kept.
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Returns the name component.
    pub fn name(&self) -> &str {
        &self.name
    }
}
