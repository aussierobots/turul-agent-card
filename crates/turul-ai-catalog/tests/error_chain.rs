//! A front end shows an error and every cause in one line.

use std::error::Error;
use std::fmt;

use turul_ai_catalog::error::chain;

#[derive(Debug)]
struct Outer(Inner);
#[derive(Debug)]
struct Inner;

impl fmt::Display for Outer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("terminal error")
    }
}
impl Error for Outer {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}
impl fmt::Display for Inner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a tty")
    }
}
impl Error for Inner {}

#[test]
fn causes_follow_the_message() {
    assert_eq!(chain(&Outer(Inner)), "terminal error: not a tty");
}

#[test]
fn an_error_without_a_source_is_its_message() {
    assert_eq!(chain(&Inner), "not a tty");
}
