pub mod sqlite;
pub mod tantivy;
pub mod embedding;

use crate::config::Config;
use crate::types::RawFileInfo;
use fan_plugin_sdk::FormatInfo;
use sqlite::SqliteStore;

pub enum IndexMode {
    ReadOnly,
    ReadWrite,
}

pub struct IndexEngine {
    pub sqlite: SqliteStore,
    pub tantivy: tantivy::TantivyIndex,
    pub embedding: embedding::EmbeddingEngine,
}

impl IndexEngine {
    pub fn open(config: &Config, read_only: bool) -> Result<Self, Box<dyn std::error::Error>> {
        let data_dir = crate::config::dirs_fan().join("data");
        let sqlite = SqliteStore::open(&data_dir)?;
        let tantivy = match tantivy::TantivyIndex::open(&data_dir, read_only) {
            Ok(t) => t,
            Err(_e) if !read_only => {
                // Stale lock from crashed process? Clean and retry once.
                let lock_files = [
                    data_dir.join("tantivy/.tantivy-writer.lock"),
                    data_dir.join("tantivy/.tantivy-meta.lock"),
                ];
                for lock in &lock_files {
                    if lock.exists() {
                        std::fs::remove_file(lock).ok();
                    }
                }
                tantivy::TantivyIndex::open(&data_dir, read_only)?
            }
            Err(e) => return Err(e),
        };
        let embedding = embedding::EmbeddingEngine::new(config)?;
        Ok(Self { sqlite, tantivy, embedding })
    }

    pub fn index_file(
        &self,
        info: &RawFileInfo,
        format_info: Option<&FormatInfo>,
    ) -> Result<i64, Box<dyn std::error::Error>> {
        let id = self.sqlite.upsert(info, format_info)?;
        let metadata_text = format!("{} {:?}", info.path.display(), format_info);
        self.tantivy
            .index_file(id, &info.path, &metadata_text, &[])?;
        Ok(id)
    }
}

/// Open the index with the given mode. This is the canonical entry point
/// for all CLI commands and daemon.
pub fn open_index(config: &crate::config::Config, mode: IndexMode) -> Result<IndexEngine, Box<dyn std::error::Error>> {
    IndexEngine::open(config, matches!(mode, IndexMode::ReadOnly))
}
