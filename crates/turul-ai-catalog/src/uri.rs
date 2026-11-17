//! RFC 3986 grammar, as far as a caller needs it.

fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

fn is_sub_delim(b: u8) -> bool {
    matches!(
        b,
        b'!' | b'$' | b'&' | b'\'' | b'(' | b')' | b'*' | b'+' | b',' | b';' | b'='
    )
}

/// Every byte is one of `allowed`, unreserved, a sub-delim or a `%XX` escape.
fn chars_ok(s: &str, allowed: &[u8]) -> bool {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'%' {
            if i + 2 >= b.len() {
                return false;
            }
            if !b[i + 1].is_ascii_hexdigit() || !b[i + 2].is_ascii_hexdigit() {
                return false;
            }
            i += 3;
            continue;
        }
        if !(is_unreserved(c) || is_sub_delim(c) || allowed.contains(&c)) {
            return false;
        }
        i += 1;
    }
    true
}

/// Returns whether `s` is an RFC 3986 `fragment` (`*( pchar / "/" / "?" )`, section 3.5).
///
/// The empty string is a fragment here; the caller decides whether it may be.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog::uri::is_fragment;
///
/// assert!(is_fragment("section-1/a%20b"));
/// assert!(!is_fragment("has space"));
/// ```
pub fn is_fragment(s: &str) -> bool {
    chars_ok(s, b":@/?")
}

/// RFC 3986 `URI` (section 3, fragment allowed) whose `hier-part` starts with `//` and has a
/// non-empty authority.
pub(crate) fn is_uri_with_authority(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once("://") else {
        return false;
    };
    let mut sc = scheme.bytes();
    if !sc.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !sc.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
    {
        return false;
    }
    let (rest, fragment) = match rest.split_once('#') {
        Some((a, f)) => (a, Some(f)),
        None => (rest, None),
    };
    if fragment.is_some_and(|f| !chars_ok(f, b":@/?")) {
        return false;
    }
    let (rest, query) = match rest.split_once('?') {
        Some((a, q)) => (a, Some(q)),
        None => (rest, None),
    };
    if query.is_some_and(|q| !chars_ok(q, b":@/?")) {
        return false;
    }
    let (authority, path) = match rest.find('/') {
        Some(i) => rest.split_at(i),
        None => (rest, ""),
    };
    !authority.is_empty() && chars_ok(authority, b":@[]") && chars_ok(path, b":@/")
}

fn is_ip_literal(inner: &str) -> bool {
    match inner.strip_prefix(['v', 'V']) {
        // IPvFuture = "v" 1*HEXDIG "." 1*( unreserved / sub-delims / ":" )
        Some(rest) => rest.split_once('.').is_some_and(|(h, t)| {
            !h.is_empty()
                && h.bytes().all(|c| c.is_ascii_hexdigit())
                && !t.is_empty()
                && t.bytes()
                    .all(|c| is_unreserved(c) || is_sub_delim(c) || c == b':')
        }),
        // IPv6address, checked by its alphabet and shape only.
        None => {
            inner.contains(':')
                && inner
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() || c == b':' || c == b'.')
        }
    }
}

fn is_authority(a: &str) -> bool {
    let host_port = match a.split_once('@') {
        Some((userinfo, rest)) => {
            if !chars_ok(userinfo, b":") {
                return false;
            }
            rest
        }
        None => a,
    };
    if let Some(rest) = host_port.strip_prefix('[') {
        let Some((inner, after)) = rest.split_once(']') else {
            return false;
        };
        return is_ip_literal(inner)
            && after
                .strip_prefix(':')
                .map_or(after.is_empty(), |p| p.bytes().all(|c| c.is_ascii_digit()));
    }
    let (host, port) = match host_port.split_once(':') {
        Some((h, p)) => (h, Some(p)),
        None => (host_port, None),
    };
    chars_ok(host, b"") && port.is_none_or(|p| p.bytes().all(|c| c.is_ascii_digit()))
}

/// RFC 3986 `absolute-URI`: `scheme ":" hier-part [ "?" query ]`, no fragment.
pub(crate) fn is_absolute_uri(s: &str) -> bool {
    let Some((scheme, rest)) = s.split_once(':') else {
        return false;
    };
    let mut sc = scheme.bytes();
    if !sc.next().is_some_and(|c| c.is_ascii_alphabetic())
        || !sc.all(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.'))
    {
        return false;
    }
    let (hier, query) = match rest.split_once('?') {
        Some((h, q)) => (h, Some(q)),
        None => (rest, None),
    };
    if query.is_some_and(|q| !chars_ok(q, b":@/?")) {
        return false;
    }
    match hier.strip_prefix("//") {
        Some(after) => {
            let (authority, path) = match after.find('/') {
                Some(i) => after.split_at(i),
                None => (after, ""),
            };
            is_authority(authority) && chars_ok(path, b":@/")
        }
        None => chars_ok(hier, b":@/"),
    }
}
