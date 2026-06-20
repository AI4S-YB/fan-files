use fan_plugin_sdk::{BioMetadata, FormatInfo};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 文件基础条目（存储层）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: i64,
    pub path: PathBuf,
    pub source_server: String,
    pub size: u64,
    pub mtime_secs: i64,
    pub hash_sha256: Option<String>,
    pub magic_bytes: Option<Vec<u8>>,
    pub mime_type: Option<String>,
    pub format_info: Option<FormatInfo>,
    pub bio_metadata: Option<BioMetadata>,
    pub indexed_at: i64,
    pub updated_at: i64,
    pub deleted: bool,
}

/// 扫描阶段的基础信息（入库前）
#[derive(Debug, Clone)]
pub struct RawFileInfo {
    pub path: PathBuf,
    pub source_server: String,
    pub size: u64,
    pub mtime_secs: i64,
    pub hash_sha256: Option<String>,
    pub magic_bytes: Vec<u8>,
    pub mime_type: String,
}

/// 单台服务器的统计
#[derive(Debug, Clone, Serialize)]
pub struct ServerStats {
    pub server: String,
    pub file_count: u64,
    pub last_scan: Option<i64>,
}

/// 索引统计
#[derive(Debug, Clone, Serialize)]
pub struct IndexStatus {
    pub total_files: u64,
    pub indexed_files: u64,
    pub deleted_files: u64,
    pub last_full_scan: Option<i64>,
    pub last_change: Option<i64>,
    pub db_size_bytes: u64,
    pub servers: Vec<ServerStats>,
}
