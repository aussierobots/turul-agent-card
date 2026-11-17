//! Rendering an error together with its causes.

use std::error::Error;
use std::fmt::Write as _;

/// Formats `error` followed by each cause in its [`Error::source`] chain, joined by `": "`.
///
/// Error types in these crates follow the Rust API Guidelines: a `Display` message describes only
/// its own failure and the underlying cause is returned by `source()`. Front ends call this to show
/// the whole chain in one line.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog::error::chain;
///
/// let inner = std::io::Error::other("disk full");
/// let outer = std::io::Error::new(std::io::ErrorKind::Other, inner);
/// assert_eq!(chain(&outer), "disk full");
/// ```
pub fn chain(error: &dyn Error) -> String {
    let mut out = error.to_string();
    let mut cause = error.source();
    while let Some(e) = cause {
        let text = e.to_string();
        // Skip a cause whose text the previous message already ends with.
        if !out.ends_with(&text) {
            let _ = write!(out, ": {text}");
        }
        cause = e.source();
    }
    out
}
