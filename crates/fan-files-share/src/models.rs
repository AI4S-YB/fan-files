use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
pub struct Envelope<T> {
    pub data: T,
}
#[derive(Debug, Serialize)]
pub struct PageEnvelope<T> {
    pub data: Vec<T>,
    pub meta: PageMeta,
}
#[derive(Debug, Serialize)]
pub struct PageMeta {
    pub limit: u32,
    pub next_cursor: Option<i64>,
    pub has_more: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub type_counts: Option<Vec<Facet>>,
}

#[derive(Debug, Serialize)]
pub struct DatasetSummary {
    pub id: i64,
    pub name: String,
    #[serde(rename = "type")]
    pub dataset_type: Option<String>,
    pub species: Option<String>,
    pub summary: Option<String>,
    pub path: Option<String>,
    pub asset_count: i64,
    pub file_count: i64,
    pub updated_at: i64,
}

#[derive(Debug, Serialize)]
pub struct DatasetDetail {
    pub id: i64,
    pub name: String,
    #[serde(rename = "type")]
    pub dataset_type: Option<String>,
    pub species: Option<String>,
    pub species_confidence: Option<String>,
    pub summary: Option<String>,
    pub path: Option<String>,
    pub updated_at: i64,
    pub assets: Vec<AssetSummary>,
}
#[derive(Debug, Serialize)]
pub struct AssetSummary {
    pub id: i64,
    pub name: Option<String>,
    #[serde(rename = "type")]
    pub asset_type: Option<String>,
    pub file_count: i64,
}
#[derive(Debug, Serialize)]
pub struct FileSummary {
    pub id: i64,
    pub asset_id: i64,
    pub name: String,
    pub size: u64,
    pub role: Option<String>,
    pub mime_type: Option<String>,
    pub source_server: String,
    pub path: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DatasetQuery {
    pub q: Option<String>,
    pub species: Option<String>,
    #[serde(rename = "type")]
    pub dataset_type: Option<String>,
    pub cursor: Option<i64>,
    pub limit: Option<u32>,
    pub sort: Option<String>,
    pub order: Option<String>,
}
#[derive(Debug, Deserialize, Default)]
pub struct FileQuery {
    pub asset_id: Option<i64>,
    pub cursor: Option<i64>,
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, Clone)]
pub struct Facet {
    pub value: String,
    pub count: i64,
}
#[derive(Debug, Serialize, Clone)]
pub struct Facets {
    pub species: Vec<Facet>,
    pub types: Vec<Facet>,
}
#[derive(Debug, Serialize, Clone)]
pub struct Stats {
    pub datasets_upper_bound: i64,
    pub assets_upper_bound: i64,
    pub files_upper_bound: i64,
    pub linked_files_upper_bound: i64,
    pub last_indexed_at: Option<i64>,
    pub approximate: bool,
}
#[derive(Debug, Serialize)]
pub struct Health {
    pub status: &'static str,
}
#[derive(Debug, Serialize)]
pub struct Readiness {
    pub status: &'static str,
    pub database: &'static str,
    pub schema_version: i64,
}
