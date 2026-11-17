//! The one sanitiser for displayed catalog text (#embedded-content-safety).

use std::fmt;

/// Catalog text for a text rendering: every Unicode `Cc` character is replaced by U+FFFD and
/// every other character is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DisplayText(String);

impl DisplayText {
    pub fn new(text: &str) -> Self {
        DisplayText(
            text.chars()
                .map(|c| if c.is_control() { '\u{FFFD}' } else { c })
                .collect(),
        )
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DisplayText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
