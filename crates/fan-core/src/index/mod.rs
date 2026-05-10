pub mod sqlite;
pub mod tantivy;
pub mod embedding;

use crate::config::Config;
use crate::types::RawFileInfo;
use fan_plugin_sdk::FormatInfo;
use sqlite::SqliteStore;

pub struct IndexEngine {
    pub sqlite: SqliteStore,
    pub tantivy: tantivy::TantivyIndex,
    pub embedding: embedding::EmbeddingEngine,
}

impl IndexEngine {
    pub fn open(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let data_dir = crate::config::dirs_fan().join("data");
        Ok(Self {
            sqlite: SqliteStore::open(&data_dir)?,
            tantivy: tantivy::TantivyIndex::open(&data_dir)?,
            embedding: embedding::EmbeddingEngine::new(config)?,
        })
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
