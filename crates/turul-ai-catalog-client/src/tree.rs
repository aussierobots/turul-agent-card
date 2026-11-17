//! The resolved catalog tree and its filter.

use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
use turul_ai_catalog::model::{AiCatalog, CatalogEntry};
use turul_ai_catalog::resolve;
use turul_ai_catalog::validate::{NestingDepth, Report};

use crate::accept::LoadWarning;
use crate::load::LoadError;

/// A loaded catalog with its validation report and the nested catalogs resolved from it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ResolvedCatalog {
    /// The parsed catalog.
    pub document: AiCatalog,
    /// The validation report of `document` at its nesting depth.
    pub report: Report,
    /// The fetched URL; `None` for a file.
    pub base: Option<String>,
    /// The nesting level of this catalog; the root is level 1.
    pub depth: NestingDepth,
    /// Retrieval warnings for this catalog.
    pub warnings: Vec<LoadWarning>,
    /// One display row per entry of `document`, in order, minus any removed by
    /// [`filter`](ResolvedCatalog::filter).
    pub rows: Vec<EntryRow>,
    /// The nested catalogs, one per entry of type `application/ai-catalog+json` with a `url`.
    pub children: Vec<Nested>,
}

/// A nested catalog entry and what loading it produced.
#[derive(Clone, Debug)]
pub struct Nested {
    /// Index in the parent's `document.entries`.
    pub entry: usize,
    /// The loaded catalog, or why it was skipped.
    pub child: Child,
}

/// The outcome of loading one nested catalog.
#[derive(Clone, Debug)]
pub enum Child {
    /// The nested catalog was loaded.
    Catalog(Box<ResolvedCatalog>),
    /// The nested catalog was not loaded.
    Skipped {
        /// The resolved URL, or the entry's `url` text when it did not resolve.
        url: String,
        /// Why it was not loaded.
        reason: SkipReason,
    },
}

#[derive(Clone, Debug)]
/// Why a nested catalog was not loaded.
#[non_exhaustive]
pub enum SkipReason {
    /// The request budget was spent.
    Budget,
    /// The catalog is deeper than the maximum nesting depth.
    Depth,
    /// The catalog is already on the path from the root.
    Cycle,
    /// The entry `url` cannot be resolved to an absolute URL.
    Unresolvable,
    /// The request failed or its response was refused.
    Fetch(LoadError),
}

/// Display values of one entry, resolved without fetching its artifact.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EntryRow {
    /// Index in `document.entries`.
    pub index: usize,
    /// The display name, falling back to the identifier's name.
    pub name: String,
    /// The description, if any.
    pub description: Option<String>,
    /// The version, if any.
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
    /// Matches entries with this identifier.
    pub identifier: Option<EntryIdentifier>,
    /// Matches entries with this version.
    pub version: Option<String>,
    /// Matches entries of this artifact type.
    pub artifact_type: Option<ArtifactType>,
    /// Matches entries carrying this tag.
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
    /// Returns whether any loaded catalog, this node or a nested one, has an error diagnostic.
    pub fn has_errors(&self) -> bool {
        self.report.has_errors()
            || self.children.iter().any(|n| match &n.child {
                Child::Catalog(c) => c.has_errors(),
                Child::Skipped { .. } => false,
            })
    }

    /// Returns the same tree with only the rows that match `filter`, in every node.
    ///
    /// Nested catalogs are kept whatever the filter; only `rows` are pruned.
    ///
    /// # Examples
    ///
    /// ```
    /// use turul_ai_catalog_client::{FetchPolicy, Filter, Source, load};
    ///
    /// let dir = std::env::temp_dir().join(format!("ai-catalog-filter-doc-{}", std::process::id()));
    /// std::fs::create_dir_all(&dir)?;
    /// let file = dir.join("ai-catalog.json");
    /// std::fs::write(&file, br#"{"specVersion":"1.0","entries":[
    ///   {"identifier":"urn:air:example.com:mcp:a","type":"application/mcp-server-card+json","url":"https://example.com/a"},
    ///   {"identifier":"urn:air:example.com:mcp:b","type":"application/mcp-server-card+json","url":"https://example.com/b","tags":["weather"]}]}"#)?;
    /// let tree = load(&Source::File(file), &FetchPolicy::default())?;
    ///
    /// let mut filter = Filter::default();
    /// filter.tag = Some("weather".into());
    /// assert_eq!(tree.filter(&filter).rows.len(), 1);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
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
