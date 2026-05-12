use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 插件元信息
#[derive(Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    /// "format-detector" | "context-interpreter"
    pub plugin_type: PluginType,
    /// 优先级 0-100，越大越优先
    pub priority: u8,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginType {
    FormatDetector,
    ContextInterpreter,
}

/// Layer 1 输出: 文件物理格式
#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(default)]
pub struct FormatInfo {
    pub file_type: String,    // "FASTQ", "BAM", "CSV" ...
    pub mime: Option<String>,
}

impl Default for FormatInfo {
    fn default() -> Self {
        Self {
            file_type: String::new(),
            mime: None,
        }
    }
}

/// Layer 2 输出: 生物学元数据
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
#[serde(default)]
pub struct BioMetadata {
    pub assay_type: Option<String>,     // "RNA-seq", "ChIP-seq" ...
    pub species: Option<String>,        // "human", "mouse" ...
    pub tissue: Option<String>,
    pub genome_build: Option<String>,   // "hg38", "mm10" ...
    pub project: Option<String>,
    pub tags: Vec<String>,
    #[serde(flatten)]
    pub extra: HashMap<String, String>,
}

/// Layer 2 输入: 文件上下文
#[derive(Serialize, Deserialize, Debug)]
pub struct FileContext {
    pub file_path: String,
    pub siblings: Vec<String>,
    pub directory_tree: Vec<String>,
    pub metadata_files: Vec<String>,
    pub file_header_b64: String,
    pub format_tags: Vec<String>,
}

/// 检索结果
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SearchResult {
    pub path: String,
    pub score: f64,
    pub file_type: Option<String>,
    pub assay_type: Option<String>,
    pub species: Option<String>,
    pub tags: Vec<String>,
    pub summary: String,
    pub source: DataSource,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub enum DataSource {
    Local,
    Public { origin: String },
}
