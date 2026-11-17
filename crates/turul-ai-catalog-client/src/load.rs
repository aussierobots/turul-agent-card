//! [`load`]: fetches or reads a root catalog and follows its nested catalogs.

use std::fmt;
use std::sync::Arc;

use turul_ai_catalog::identity::ArtifactType;
use turul_ai_catalog::model::{AiCatalog, ArtifactSource};
use turul_ai_catalog::parse::{ParseError, parse};
use turul_ai_catalog::validate::{NestingDepth, validate_at};

use crate::accept::{AcceptError, LoadWarning, accept_response};
use crate::fetch::{FetchError, FetchPolicy, RequestBudget, Stop, hops};
use crate::reference::resolve_reference;
use crate::source::Source;
use crate::tree::{Child, EntryRow, Nested, ResolvedCatalog, SkipReason};

/// Why [`load`] could not load the root catalog.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum LoadError {
    /// The root file could not be read.
    Io(Arc<std::io::Error>),
    /// The root file is not a parseable catalog.
    Parse(ParseError),
    /// The root request was refused or failed.
    Fetch(FetchError),
    /// The root response was not an acceptable catalog.
    Accept(AcceptError),
    /// The root needed more than [`FetchPolicy::max_requests`] requests.
    Budget {
        /// The request limit, [`FetchPolicy::max_requests`].
        limit: u32,
    },
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadError::Io(e) => write!(f, "{e}"),
            LoadError::Parse(e) => write!(f, "{e}"),
            LoadError::Fetch(e) => write!(f, "{e}"),
            LoadError::Accept(e) => write!(f, "{e}"),
            LoadError::Budget { limit } => write!(f, "request budget of {limit} spent"),
        }
    }
}

impl std::error::Error for LoadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            LoadError::Io(e) => e.source(),
            LoadError::Parse(e) => e.source(),
            LoadError::Fetch(e) => e.source(),
            LoadError::Accept(e) => e.source(),
            LoadError::Budget { .. } => None,
        }
    }
}

/// Loads a catalog and the catalogs nested in it by `url`.
///
/// Nested catalogs are followed depth-first in entry order, one request at a time, to level 4
/// (the root is level 1). A nested catalog on the current path, a level beyond the limit, an
/// unresolvable `url`, a failed fetch or a spent request budget
/// ([`FetchPolicy::max_requests`]) leaves that child [`Child::Skipped`]; only a root failure
/// is an error. Each loaded catalog is validated; the diagnostics are in its
/// [`report`](ResolvedCatalog::report) and do not fail the load.
///
/// Blocking: call through `tokio::task::spawn_blocking` from async code.
///
/// # Errors
///
/// Returns [`LoadError::Io`] when a root file cannot be read, [`LoadError::Parse`] when it does
/// not parse, [`LoadError::Fetch`] or [`LoadError::Accept`] when the root request fails or its
/// response is refused by [`accept_response`], and [`LoadError::Budget`] when the root request
/// cannot be sent within [`FetchPolicy::max_requests`].
///
/// # Examples
///
/// ```
/// use turul_ai_catalog_client::{FetchPolicy, Source, load};
///
/// let dir = std::env::temp_dir().join(format!("ai-catalog-load-doc-{}", std::process::id()));
/// std::fs::create_dir_all(&dir)?;
/// let file = dir.join("ai-catalog.json");
/// std::fs::write(&file, br#"{"specVersion":"1.0","entries":[]}"#)?;
///
/// let tree = load(&Source::File(file), &FetchPolicy::default())?;
/// assert!(tree.rows.is_empty());
/// assert!(!tree.has_errors());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn load(source: &Source, policy: &FetchPolicy) -> Result<ResolvedCatalog, LoadError> {
    let mut budget = RequestBudget::new(policy.max_requests);
    let mut path = Vec::new();
    let mut root = match source {
        Source::File(file) => {
            let bytes = std::fs::read(file).map_err(|e| LoadError::Io(Arc::new(e)))?;
            let document = parse(&bytes).map_err(LoadError::Parse)?;
            node(document, None, Vec::new(), NestingDepth::ROOT)
        }
        other => {
            let url = other.request_url().expect("non-file sources have a URL");
            let (document, base, warnings) =
                retrieve(&url, policy, &mut budget).map_err(|e| e.root(policy.max_requests))?;
            path.push(url);
            path.extend(base.clone());
            node(document, base, warnings, NestingDepth::ROOT)
        }
    };
    expand(&mut root, policy, &mut budget, &mut path);
    let skipped = count_budget_skips(&root);
    if skipped > 0 {
        root.warnings.push(LoadWarning::RequestBudget {
            limit: policy.max_requests,
            skipped,
        });
    }
    Ok(root)
}

enum Failure {
    Budget,
    Error(LoadError),
}

impl Failure {
    fn root(self, limit: u32) -> LoadError {
        match self {
            Failure::Budget => LoadError::Budget { limit },
            Failure::Error(e) => e,
        }
    }
}

/// Fetches and accepts one catalog; returns it, its final URL and its warnings.
fn retrieve(
    url: &str,
    policy: &FetchPolicy,
    budget: &mut RequestBudget,
) -> Result<(AiCatalog, Option<String>, Vec<LoadWarning>), Failure> {
    let fetched = hops(url, None, policy, Some(budget)).map_err(|s| match s {
        Stop::Budget => Failure::Budget,
        Stop::Refused(e) => Failure::Error(LoadError::Fetch(e)),
    })?;
    let accepted = accept_response(&fetched).map_err(|e| Failure::Error(LoadError::Accept(e)))?;
    Ok((
        accepted.catalog,
        fetched.url().map(String::from),
        accepted.warnings,
    ))
}

fn node(
    document: AiCatalog,
    base: Option<String>,
    warnings: Vec<LoadWarning>,
    depth: NestingDepth,
) -> ResolvedCatalog {
    let report = validate_at(&document, depth);
    let rows = document
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| EntryRow::new(i, e))
        .collect();
    ResolvedCatalog {
        document,
        report,
        base,
        depth,
        warnings,
        rows,
        children: Vec::new(),
    }
}

/// Visits the entries of `node`; `path` holds the URLs of `node` and its ancestors.
fn expand(
    node: &mut ResolvedCatalog,
    policy: &FetchPolicy,
    budget: &mut RequestBudget,
    path: &mut Vec<String>,
) {
    let mut children = Vec::new();
    for (entry, e) in node.document.entries.iter().enumerate() {
        let ArtifactSource::Url(text) = &e.source else {
            continue;
        };
        if e.artifact_type != ArtifactType::AI_CATALOG {
            continue;
        }
        let child = visit(node, text, policy, budget, path);
        children.push(Nested { entry, child });
    }
    node.children = children;
}

fn visit(
    parent: &ResolvedCatalog,
    text: &str,
    policy: &FetchPolicy,
    budget: &mut RequestBudget,
    path: &mut Vec<String>,
) -> Child {
    let resolved = resolve_reference(parent.base.as_deref(), text);
    let url = resolved.clone().unwrap_or_else(|_| text.to_string());
    let skip = |reason| Child::Skipped {
        url: url.clone(),
        reason,
    };
    if budget.is_spent() {
        return skip(SkipReason::Budget);
    }
    let Some(depth) = parent.depth.child() else {
        return skip(SkipReason::Depth);
    };
    if resolved.is_err() {
        return skip(SkipReason::Unresolvable);
    }
    if path.contains(&url) {
        return skip(SkipReason::Cycle);
    }
    let (document, base, warnings) = match retrieve(&url, policy, budget) {
        Ok(got) => got,
        Err(Failure::Budget) => return skip(SkipReason::Budget),
        Err(Failure::Error(e)) => return skip(SkipReason::Fetch(e)),
    };
    if base.as_ref().is_some_and(|b| path.contains(b)) {
        return skip(SkipReason::Cycle);
    }
    let mut child = node(document, base.clone(), warnings, depth);
    let mark = path.len();
    path.push(url.clone());
    path.extend(base);
    expand(&mut child, policy, budget, path);
    path.truncate(mark);
    Child::Catalog(Box::new(child))
}

fn count_budget_skips(node: &ResolvedCatalog) -> usize {
    node.children
        .iter()
        .map(|n| match &n.child {
            Child::Catalog(c) => count_budget_skips(c),
            Child::Skipped {
                reason: SkipReason::Budget,
                ..
            } => 1,
            Child::Skipped { .. } => 0,
        })
        .sum()
}
