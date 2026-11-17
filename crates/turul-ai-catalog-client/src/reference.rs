//! The one URL-reference resolver: WHATWG resolution via the `url` crate.

use std::fmt;

use url::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unresolvable;

impl fmt::Display for Unresolvable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("URL reference cannot be resolved")
    }
}

impl std::error::Error for Unresolvable {}

/// An absolute `reference` is returned serialised; otherwise it is joined to `base`.
pub fn resolve_reference(base: Option<&str>, reference: &str) -> Result<String, Unresolvable> {
    if let Ok(absolute) = Url::parse(reference) {
        return Ok(absolute.into());
    }
    let base = Url::parse(base.ok_or(Unresolvable)?).map_err(|_| Unresolvable)?;
    base.join(reference)
        .map(String::from)
        .map_err(|_| Unresolvable)
}
