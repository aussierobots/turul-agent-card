//! The sanitiser for displayed catalog text, per
//! [#embedded-content-safety](https://ai-catalog.io/specification/#embedded-content-safety).

use std::fmt;

/// Catalog text for a text rendering: every Unicode `Cc` character is replaced by U+FFFD and
/// every other character is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayText(String);

impl DisplayText {
    /// Sanitises `text` for display, replacing every control character with U+FFFD.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog_client::DisplayText;
    ///
    /// assert_eq!(DisplayText::new("a\u{1b}[2Jb").as_str(), "a\u{FFFD}[2Jb");
    /// ```
    pub fn new(text: &str) -> Self {
        DisplayText(
            text.chars()
                .map(|c| if c.is_control() { '\u{FFFD}' } else { c })
                .collect(),
        )
    }

    /// Returns the sanitised text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DisplayText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
