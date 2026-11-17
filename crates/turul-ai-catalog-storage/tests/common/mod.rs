#![allow(dead_code)]

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::{ArtifactSource, CatalogEntry, HostInfo};
use turul_ai_catalog::timestamp::Timestamp;
use turul_ai_catalog_storage::{CatalogId, EntryRecord, HostRecord, Provenance, WriterId};

pub fn catalog(id: &str) -> CatalogId {
    CatalogId::new(id).unwrap()
}

pub fn entry(identifier: &str, version: Option<&str>) -> CatalogEntry {
    let mut e = CatalogEntry::new(
        EntryIdentifier::new(identifier),
        ArtifactType::MCP_SERVER_CARD,
        ArtifactSource::Url("https://example.com/card.json".to_string()),
    );
    e.version = version.map(str::to_string);
    e
}

pub fn record(identifier: &str, version: Option<&str>) -> EntryRecord {
    EntryRecord::new(entry(identifier, version)).unwrap()
}

pub fn host() -> HostRecord {
    HostRecord::new(HostInfo::new("Example Operator".to_string())).unwrap()
}

pub fn number(lexeme: &str) -> Json {
    turul_ai_catalog::json::read(lexeme.as_bytes()).unwrap()
}

pub fn writer(id: &str) -> WriterId {
    WriterId::new(id).unwrap()
}

/// Curated provenance of the writer `operator`.
pub fn curated() -> Provenance {
    curated_by("operator")
}

pub fn curated_by(id: &str) -> Provenance {
    Provenance::Curated { writer: writer(id) }
}

pub fn lease(id: &str, expires_at: &str) -> Provenance {
    Provenance::Lease {
        writer: writer(id),
        expires_at: Timestamp::parse(expires_at).unwrap(),
    }
}

/// The instant every assembly check runs at: 2026-11-17T12:00:00Z.
pub fn now() -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(1_794_916_800)
}

/// `now()` moved by `nanos` nanoseconds, earlier when `earlier`.
pub fn now_shifted(nanos: u64, earlier: bool) -> SystemTime {
    let d = Duration::from_nanos(nanos);
    if earlier { now() - d } else { now() + d }
}

pub fn ts(text: &str) -> Timestamp {
    Timestamp::parse(text).unwrap()
}
