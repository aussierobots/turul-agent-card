#![allow(dead_code)]

use turul_ai_catalog::identity::{ArtifactType, EntryIdentifier};
use turul_ai_catalog::json::Json;
use turul_ai_catalog::model::{ArtifactSource, CatalogEntry, HostInfo};
use turul_ai_catalog_storage::{CatalogId, EntryRecord, HostRecord};

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
