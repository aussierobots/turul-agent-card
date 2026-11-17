//! Extension keys and the policy-urls extension (#format-and-key-naming, #policy-urls-extension).

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

/// The policy-urls extension value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PolicyUrls<'a> {
    pub privacy_policy_url: Option<&'a str>,
    pub terms_of_service_url: Option<&'a str>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PolicyUrlsError {
    /// The value is not a JSON object.
    NotObject,
    /// The named member is present and not a string.
    MemberNotString(String),
}

impl CatalogEntry {
    /// The policy-urls extension of this entry; `Ok(None)` when absent.
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
