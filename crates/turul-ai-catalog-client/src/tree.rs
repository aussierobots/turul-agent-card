//! The resolved catalog tree and its filter.

use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
use turul_ai_catalog::model::{AiCatalog, CatalogEntry};
use turul_ai_catalog::resolve;
use turul_ai_catalog::validate::{NestingDepth, Report};

use crate::accept::LoadWarning;
use crate::load::LoadError;

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ResolvedCatalog {
    pub document: AiCatalog,
    pub report: Report,
    /// The fetched URL; `None` for a file.
    pub base: Option<String>,
    pub depth: NestingDepth,
    pub warnings: Vec<LoadWarning>,
    pub rows: Vec<EntryRow>,
    pub children: Vec<Nested>,
}

#[derive(Clone, Debug)]
pub struct Nested {
    /// Index in the parent's `document.entries`.
    pub entry: usize,
    pub child: Child,
}

#[derive(Clone, Debug)]
pub enum Child {
    Catalog(Box<ResolvedCatalog>),
    Skipped { url: String, reason: SkipReason },
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum SkipReason {
    Budget,
    Depth,
    Cycle,
    Unresolvable,
    Fetch(LoadError),
}

/// Display values of one entry, resolved without fetching its artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryRow {
    pub index: usize,
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
}

impl EntryRow {
    pub(crate) fn new(index: usize, entry: &CatalogEntry) -> Self {
        EntryRow {
            index,
            name: resolve::display_name(entry, None).to_string(),
            description: resolve::description(entry, None).map(String::from),
            version: resolve::version(entry, None).map(String::from),
        }
    }
}

/// Criteria that must all hold; an unset criterion matches everything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Filter {
    pub identifier: Option<EntryIdentifier>,
    pub version: Option<String>,
    pub artifact_type: Option<ArtifactType>,
    pub tag: Option<String>,
}

impl Filter {
    fn matches(&self, entry: &CatalogEntry) -> bool {
        self.identifier
            .as_ref()
            .is_none_or(|i| *i == entry.identifier)
            && self
                .version
                .as_ref()
                .is_none_or(|v| entry.version.as_ref() == Some(v))
            && self
                .artifact_type
                .as_ref()
                .is_none_or(|t| *t == entry.artifact_type)
            && self
                .tag
                .as_ref()
                .is_none_or(|t| entry.tags.as_ref().is_some_and(|tags| tags.contains(t)))
    }
}

impl ResolvedCatalog {
    /// Whether any loaded catalog, this node or a nested one, has an error diagnostic.
    pub fn has_errors(&self) -> bool {
        self.report.has_errors()
            || self.children.iter().any(|n| match &n.child {
                Child::Catalog(c) => c.has_errors(),
                Child::Skipped { .. } => false,
            })
    }

    /// The same tree with only the rows that match, in every node.
    pub fn filter(&self, filter: &Filter) -> ResolvedCatalog {
        let mut out = self.clone();
        out.prune(filter);
        out
    }

    fn prune(&mut self, filter: &Filter) {
        let entries = &self.document.entries;
        self.rows.retain(|r| filter.matches(&entries[r.index]));
        for n in &mut self.children {
            if let Child::Catalog(c) = &mut n.child {
                c.prune(filter);
            }
        }
    }
}
