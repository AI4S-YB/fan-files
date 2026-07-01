pub mod sqlite;
pub mod tantivy;
pub mod embedding;

use crate::config::{Config, DataLayer};
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
        Self::open_at(&data_dir, config, read_only)
    }

    pub fn open_at(
        data_dir: &std::path::PathBuf,
        config: &Config,
        read_only: bool,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        // Clean stale locks
        let _ = std::fs::remove_file(data_dir.join("tantivy/.tantivy-writer.lock"));
        let _ = std::fs::remove_file(data_dir.join("tantivy/.tantivy-meta.lock"));

        Ok(Self {
            sqlite: SqliteStore::open(data_dir)?,
            tantivy: tantivy::TantivyIndex::open(data_dir, read_only)?,
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

/// Open the index with the given mode. This is the canonical entry point
/// for all CLI commands and daemon.
pub fn open_index(config: &crate::config::Config, mode: IndexMode) -> Result<IndexEngine, Box<dyn std::error::Error>> {
    IndexEngine::open(config, matches!(mode, IndexMode::ReadOnly))
}

/// Open the index for a specific data layer, respecting read/write mode.
/// Global layer is always opened read-only for non-root users.
pub fn open_index_for_layer(
    config: &crate::config::Config,
    layer: &DataLayer,
    mode: IndexMode,
) -> Result<IndexEngine, Box<dyn std::error::Error>> {
    let data_dir = match layer {
        DataLayer::User => crate::config::dirs_fan().join("data"),
        DataLayer::Global => crate::config::dirs_fan_global().join("data"),
    };
    // Global layer is always read-only for non-root users
    let effective_mode = match layer {
        DataLayer::Global if matches!(mode, IndexMode::ReadWrite) => {
            let is_root = unsafe { libc::geteuid() == 0 };
            if !is_root { IndexMode::ReadOnly } else { IndexMode::ReadWrite }
        }
        _ => mode,
    };
    IndexEngine::open_at(&data_dir, config, matches!(effective_mode, IndexMode::ReadOnly))
}
