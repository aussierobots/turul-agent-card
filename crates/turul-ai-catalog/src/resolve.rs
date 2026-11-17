//! Display-name, description, version and latest-entry resolution.

use std::cmp::Ordering;

use crate::identity::EntryIdentifier;
use crate::model::{AiCatalog, CatalogEntry};
use crate::semver;
use crate::timestamp::Timestamp;

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

pub fn description<'a>(
    entry: &'a CatalogEntry,
    artifact_description: Option<&'a str>,
) -> Option<&'a str> {
    entry.description.as_deref().or(artifact_description)
}

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
