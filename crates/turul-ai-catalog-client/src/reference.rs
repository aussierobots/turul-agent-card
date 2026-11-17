//! The URL-reference resolver: WHATWG resolution via the `url` crate.

use std::fmt;

use url::Url;

/// A URL reference that cannot be resolved to an absolute URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unresolvable;

impl fmt::Display for Unresolvable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("URL reference cannot be resolved")
    }
}

impl std::error::Error for Unresolvable {}

/// Resolves `reference` against the optional `base` URL.
///
/// An absolute `reference` is returned serialised; otherwise it is joined to `base`.
///
/// # Errors
///
/// Returns [`Unresolvable`] when `reference` is relative and `base` is absent, is not an
/// absolute URL, or cannot be joined with it.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog_client::resolve_reference;
///
/// let url = resolve_reference(Some("https://example.com/a/ai-catalog.json"), "../b.json")?;
/// assert_eq!(url, "https://example.com/b.json");
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn resolve_reference(base: Option<&str>, reference: &str) -> Result<String, Unresolvable> {
    if let Ok(absolute) = Url::parse(reference) {
        return Ok(absolute.into());
    }
    let base = Url::parse(base.ok_or(Unresolvable)?).map_err(|_| Unresolvable)?;
    base.join(reference)
        .map(String::from)
        .map_err(|_| Unresolvable)
}
