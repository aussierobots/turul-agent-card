//! Display-name, description, version and latest-entry resolution
//! ([#resolving-an-artifact-s-display-name](https://ai-catalog.io/specification/#resolving-an-artifact-s-display-name), [#resolving-an-artifact-s-description](https://ai-catalog.io/specification/#resolving-an-artifact-s-description),
//! [#resolving-an-artifact-s-version](https://ai-catalog.io/specification/#resolving-an-artifact-s-version), [#multi-version-entries](https://ai-catalog.io/specification/#multi-version-entries)).
//!
//! The artifact's own value is passed in by the caller; this crate does not read artifacts.

use std::cmp::Ordering;

use crate::identity::EntryIdentifier;
use crate::model::{AiCatalog, CatalogEntry};
use crate::semver;
use crate::timestamp::Timestamp;

/// Returns the name to show for `entry`: its `displayName`, else `artifact_name` (the name the
/// artifact itself declares), else the last `:`- or `/`-separated segment of its identifier, or
/// the whole identifier when that segment is empty.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
/// use turul_ai_catalog::model::{ArtifactSource, CatalogEntry};
/// use turul_ai_catalog::resolve::display_name;
///
/// let entry = CatalogEntry::new(
///     EntryIdentifier::new("urn:air:example.com:mcp:weather"),
///     ArtifactType::MCP_SERVER_CARD,
///     ArtifactSource::Url("https://mcp.example.com/.well-known/mcp/server-card.json".into()),
/// );
/// assert_eq!(display_name(&entry, None), "weather");
/// assert_eq!(display_name(&entry, Some("Weather MCP")), "Weather MCP");
/// ```
pub fn display_name<'a>(entry: &'a CatalogEntry, artifact_name: Option<&'a str>) -> &'a str {
    if let Some(n) = entry.display_name.as_deref() {
        return n;
    }
    if let Some(n) = artifact_name {
        return n;
    }
    let id = entry.identifier.as_str();
    match id.rsplit([':', '/']).next() {
        Some(seg) if !seg.is_empty() => seg,
        _ => id,
    }
}

/// Returns the description of `entry`: its own `description`, else `artifact_description`.
pub fn description<'a>(
    entry: &'a CatalogEntry,
    artifact_description: Option<&'a str>,
) -> Option<&'a str> {
    entry.description.as_deref().or(artifact_description)
}

/// Returns the version of `entry`: its own `version`, else `artifact_version`.
pub fn version<'a>(entry: &'a CatalogEntry, artifact_version: Option<&'a str>) -> Option<&'a str> {
    entry.version.as_deref().or(artifact_version)
}

/// The unique maximum under `cmp`, or `None` when the maximum is tied.
fn unique_max<'a, K>(
    keyed: Vec<(K, &'a CatalogEntry)>,
    cmp: impl Fn(&K, &K) -> Ordering,
) -> Option<&'a CatalogEntry> {
    let mut best: Option<(&K, &'a CatalogEntry)> = None;
    let mut tied = false;
    for (k, e) in &keyed {
        match best {
            None => best = Some((k, e)),
            Some((bk, _)) => match cmp(k, bk) {
                Ordering::Greater => {
                    best = Some((k, e));
                    tied = false;
                }
                Ordering::Equal => tied = true,
                Ordering::Less => {}
            },
        }
    }
    if tied { None } else { best.map(|(_, e)| e) }
}

/// Returns the latest entry with `identifier` in `catalog`, or `None` when there is no such
/// entry or the latest cannot be decided.
///
/// A lone entry is the latest. Among several, the highest SemVer 2.0.0 `version` wins when
/// every one has a valid version; otherwise the newest `updatedAt` wins when every one has a
/// valid timestamp. A tie, or a group without either ordering, has no latest.
///
/// # Examples
///
/// ```
/// use turul_ai_catalog::identity::EntryIdentifier;
/// use turul_ai_catalog::parse::parse;
/// use turul_ai_catalog::resolve::latest;
///
/// let catalog = parse(br#"{"specVersion":"1.0","entries":[
///   {"identifier":"urn:air:example.com:mcp:weather","type":"application/mcp-server-card+json",
///    "url":"https://mcp.example.com/v1.json","version":"1.9.0"},
///   {"identifier":"urn:air:example.com:mcp:weather","type":"application/mcp-server-card+json",
///    "url":"https://mcp.example.com/v2.json","version":"1.10.0"}]}"#)?;
/// let id = EntryIdentifier::new("urn:air:example.com:mcp:weather");
/// let newest = latest(&catalog, &id).ok_or("a latest entry")?;
/// assert_eq!(newest.version.as_deref(), Some("1.10.0"));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn latest<'a>(
    catalog: &'a AiCatalog,
    identifier: &EntryIdentifier,
) -> Option<&'a CatalogEntry> {
    let group: Vec<&CatalogEntry> = catalog
        .entries
        .iter()
        .filter(|e| &e.identifier == identifier)
        .collect();
    if group.len() <= 1 {
        return group.first().copied();
    }
    let semvers: Option<Vec<_>> = group
        .iter()
        .map(|e| {
            e.version
                .as_deref()
                .and_then(semver::parse)
                .map(|v| (v, *e))
        })
        .collect();
    if let Some(keyed) = semvers {
        return unique_max(keyed, semver::precedence);
    }
    let instants: Option<Vec<_>> = group
        .iter()
        .map(|e| {
            e.updated_at
                .as_deref()
                .and_then(|u| Timestamp::parse(u).ok())
                .map(|t| (t, *e))
        })
        .collect();
    instants.and_then(|keyed| unique_max(keyed, Ord::cmp))
}
