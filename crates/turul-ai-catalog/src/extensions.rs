//! Extension keys and the policy-urls extension
//! ([#format-and-key-naming](https://ai-catalog.io/specification/#format-and-key-naming),
//! [#policy-urls-extension](https://ai-catalog.io/specification/#policy-urls-extension)).

use crate::json::Json;
use crate::model::CatalogEntry;
use crate::uri;

/// The policy-urls extension key; recognised only in `entry.extensions`.
pub const POLICY_URLS: &str = "https://ai-catalog.org/extensions/policy-urls";

/// The one extension-key rule: a URL with an authority, or a reverse-DNS string.
pub(crate) fn is_valid_key(key: &str) -> bool {
    if key.contains("://") {
        return uri::is_uri_with_authority(key);
    }
    let mut labels = key.split('.');
    let first_ok = labels
        .next()
        .is_some_and(|l| l.as_bytes().first().is_some_and(u8::is_ascii_alphabetic));
    first_ok
        && key.split('.').count() >= 2
        && key.split('.').all(|l| {
            !l.is_empty()
                && l.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        })
}

/// The policy-urls extension value, borrowed from the entry that carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyUrls<'a> {
    /// The `privacyPolicyUrl` member.
    pub privacy_policy_url: Option<&'a str>,
    /// The `termsOfServiceUrl` member.
    pub terms_of_service_url: Option<&'a str>,
}

/// Why [`CatalogEntry::policy_urls`] refuses the extension value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyUrlsError {
    /// The value is not a JSON object.
    NotObject,
    /// The named member is present and not a string.
    MemberNotString(String),
}

impl std::fmt::Display for PolicyUrlsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotObject => f.write_str("policy-urls value is not an object"),
            Self::MemberNotString(name) => write!(f, "policy-urls member {name} is not a string"),
        }
    }
}

impl std::error::Error for PolicyUrlsError {}

impl CatalogEntry {
    /// Returns the policy-urls extension of this entry; `Ok(None)` when absent.
    ///
    /// # Errors
    ///
    /// Returns [`PolicyUrlsError::NotObject`] when the extension value is not a JSON object and
    /// [`PolicyUrlsError::MemberNotString`] when `privacyPolicyUrl` or `termsOfServiceUrl` is
    /// present and not a string.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog::parse::parse_entry;
    ///
    /// let entry = parse_entry(br#"{
    ///   "identifier":"urn:air:example.com:mcp:weather",
    ///   "type":"application/mcp-server-card+json",
    ///   "url":"https://mcp.example.com/.well-known/mcp/server-card.json",
    ///   "extensions":{"https://ai-catalog.org/extensions/policy-urls":
    ///     {"privacyPolicyUrl":"https://example.com/privacy"}}}"#)?;
    /// let policy = entry.policy_urls()?.ok_or("policy-urls present")?;
    /// assert_eq!(policy.privacy_policy_url, Some("https://example.com/privacy"));
    /// assert_eq!(policy.terms_of_service_url, None);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn policy_urls(&self) -> Result<Option<PolicyUrls<'_>>, PolicyUrlsError> {
        let Some(v) = self.extensions.as_ref().and_then(|m| m.get(POLICY_URLS)) else {
            return Ok(None);
        };
        let Json::Object(m) = v else {
            return Err(PolicyUrlsError::NotObject);
        };
        let member = |name: &str| match m.get(name) {
            None => Ok(None),
            Some(Json::String(s)) => Ok(Some(s.as_str())),
            Some(_) => Err(PolicyUrlsError::MemberNotString(name.to_string())),
        };
        Ok(Some(PolicyUrls {
            privacy_policy_url: member("privacyPolicyUrl")?,
            terms_of_service_url: member("termsOfServiceUrl")?,
        }))
    }
}
