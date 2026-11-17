//! SemVer 2.0.0 grammar (section 2) and precedence (sections 10 and 11); nothing else.

use std::cmp::Ordering;

#[derive(Debug)]
pub(crate) struct SemVer<'a> {
    core: [&'a str; 3],
    pre: Vec<&'a str>,
}

fn digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

fn numeric(s: &str) -> bool {
    digits(s) && (s == "0" || !s.starts_with('0'))
}

fn ident(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

pub(crate) fn parse(s: &str) -> Option<SemVer<'_>> {
    let (rest, build) = match s.split_once('+') {
        Some((r, b)) => (r, Some(b)),
        None => (s, None),
    };
    if build.is_some_and(|b| !b.split('.').all(ident)) {
        return None;
    }
    let (core, pre) = match rest.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (rest, None),
    };
    let mut it = core.split('.');
    let core = [it.next()?, it.next()?, it.next()?];
    if it.next().is_some() || !core.iter().all(|p| numeric(p)) {
        return None;
    }
    let pre = match pre {
        None => Vec::new(),
        Some(p) => {
            let ids: Vec<&str> = p.split('.').collect();
            if !ids.iter().all(|i| ident(i) && (!digits(i) || numeric(i))) {
                return None;
            }
            ids
        }
    };
    Some(SemVer { core, pre })
}

fn cmp_numeric(a: &str, b: &str) -> Ordering {
    a.len().cmp(&b.len()).then_with(|| a.cmp(b))
}

pub(crate) fn precedence(a: &SemVer<'_>, b: &SemVer<'_>) -> Ordering {
    for i in 0..3 {
        let o = cmp_numeric(a.core[i], b.core[i]);
        if o != Ordering::Equal {
            return o;
        }
    }
    match (a.pre.is_empty(), b.pre.is_empty()) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    for (x, y) in a.pre.iter().zip(&b.pre) {
        let o = match (digits(x), digits(y)) {
            (true, true) => cmp_numeric(x, y),
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            (false, false) => x.cmp(y),
        };
        if o != Ordering::Equal {
            return o;
        }
    }
    a.pre.len().cmp(&b.pre.len())
}
