# Fan-Files Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 构建一个单二进制文件元数据检索引擎，扫描服务器文件、推断生物学业务元数据、支持自然语言检索，通过 CLI 供 Claude Code 调用。

**Architecture:** Rust workspace — `fan-core`(库: 类型/扫描/索引/插件引擎), `fan-plugin-sdk`(插件 SDK), `fan-files`(二进制: CLI/daemon), `plugins/`(内置 WASM 插件)。单二进制部署，systemd/LaunchAgent 管理 daemon 进程。

**Tech Stack:** Rust 1.95, rusqlite, tantivy, ort (ONNX), wasmtime, notify, walkdir, clap, serde, tokio

---

## File Structure

```
fan-files/
├── Cargo.toml                    # Workspace: fan-core, fan-plugin-sdk, fan-files, plugins/*
├── crates/
│   ├── fan-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs            # pub mod re-exports
│   │       ├── types.rs          # FileEntry, FileMeta, BioMeta, SearchResult
│   │       ├── scanner.rs        # walkdir-based scanner
│   │       ├── watcher.rs        # notify-based file watcher
│   │       ├── index/
│   │       │   ├── mod.rs        # IndexEngine facade
│   │       │   ├── sqlite.rs     # Structured metadata CRUD
│   │       │   ├── tantivy.rs    # Full-text index
│   │       │   └── embedding.rs  # ONNX embedding + semantic search
│   │       ├── plugin/
│   │       │   ├── mod.rs        # WasmHost: load/call WASM plugins
│   │       │   └── registry.rs   # Plugin discovery, scoring, dispatch
│   │       ├── suggest.rs        # Recommendation engine
│   │       └── config.rs         # Config struct + serde
│   ├── fan-plugin-sdk/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       └── lib.rs            # FormatDetector + ContextInterpreter traits + types
│   └── fan-files/
│       ├── Cargo.toml
│       └── src/
│           ├── main.rs           # clap CLI entry
│           ├── commands/
│           │   ├── mod.rs
│           │   ├── daemon.rs     # daemon mode: scan + watch loop
│           │   ├── search.rs     # search command
│           │   ├── suggest.rs    # suggest command
│           │   ├── list.rs       # list command
│           │   ├── info.rs       # info command
│           │   └── status.rs     # status command
│           └── skill.rs          # generate-skill subcommand
├── plugins/
│   ├── generic-detector/         # Built-in: fallback format detector
│   └── fastq-interpreter/        # Built-in: FASTQ context interpreter
├── skill/
│   └── fan-files.md              # Claude Code Skill (generated)
└── config/
    └── default.toml              # Shipped default config
```

---

### Task 1: Workspace & Project Skeleton

**Files:**
- Create: `Cargo.toml` (workspace root)
- Create: `crates/fan-core/Cargo.toml`, `crates/fan-core/src/lib.rs`
- Create: `crates/fan-plugin-sdk/Cargo.toml`, `crates/fan-plugin-sdk/src/lib.rs`
- Create: `crates/fan-files/Cargo.toml`, `crates/fan-files/src/main.rs`
- Create: `.gitignore`

- [ ] **Step 1: Create workspace Cargo.toml**

```toml
[workspace]
resolver = "3"
members = [
    "crates/fan-core",
    "crates/fan-plugin-sdk",
    "crates/fan-files",
    "plugins/generic-detector",
    "plugins/fastq-interpreter",
]

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"
tracing = "0.1"
tracing-subscriber = "0.3"
```

- [ ] **Step 2: Create fan-plugin-sdk — the plugin trait definitions**

`crates/fan-plugin-sdk/Cargo.toml`:
```toml
[package]
name = "fan-plugin-sdk"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["lib"]

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
```

`crates/fan-plugin-sdk/src/lib.rs`:
```rust
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
pub struct FormatInfo {
    pub file_type: String,    // "FASTQ", "BAM", "CSV" ...
    pub mime: Option<String>,
}

/// Layer 2 输出: 生物学元数据
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
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

// WASM 导出的插件函数签名见各 trait
```

- [ ] **Step 3: Create fan-core Cargo.toml**

```toml
[package]
name = "fan-core"
version = "0.1.0"
edition = "2024"

[dependencies]
fan-plugin-sdk = { path = "../fan-plugin-sdk" }
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
tracing = { workspace = true }
walkdir = "2"
notify = { version = "7", features = ["macos_fsevent"] }
rusqlite = { version = "0.34", features = ["bundled", "blob"] }
tantivy = "0.22"
ort = "2"
wasmtime = "27"
tokio = { version = "1", features = ["full"] }
toml = "0.8"
sha2 = "0.10"
mime_guess = "2"
```

- [ ] **Step 4: Create fan-files (binary) Cargo.toml**

```toml
[package]
name = "fan-files"
version = "0.1.0"
edition = "2024"

[[bin]]
name = "fan-files"
path = "src/main.rs"

[dependencies]
fan-core = { path = "../fan-core" }
clap = { version = "4", features = ["derive"] }
tokio = { workspace = true }
tracing = { workspace = true }
tracing-subscriber = { workspace = true }
```

- [ ] **Step 5: Create fan-core/src/lib.rs (placeholder)**

```rust
pub mod config;
pub mod types;
pub mod scanner;
pub mod watcher;
pub mod index;
pub mod plugin;
pub mod suggest;
```

- [ ] **Step 6: Create fan-files/src/main.rs (placeholder)**

```rust
fn main() {
    println!("fan-files v0.1.0");
}
```

- [ ] **Step 7: Create .gitignore**

```
target/
.fan-files/
*.wasm
```

- [ ] **Step 8: Build to verify**

```bash
cd fan-files && cargo build
```

Expected: `Compiling fan-files v0.1.0 ... Finished`

- [ ] **Step 9: Commit**

```bash
cd fan-files && git init && git add -A && git commit -m "feat: initialize workspace skeleton"
```

---

### Task 2: Core Types & Config

**Files:**
- Create: `crates/fan-core/src/types.rs`
- Create: `crates/fan-core/src/config.rs`

- [ ] **Step 1: Write types.rs**

```rust
use fan_plugin_sdk::{BioMetadata, FormatInfo, SearchResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// 文件基础条目（存储层）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub id: i64,
    pub path: PathBuf,
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
    pub size: u64,
    pub mtime_secs: i64,
    pub hash_sha256: Option<String>,
    pub magic_bytes: Vec<u8>,
    pub mime_type: String,
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
}
```

- [ ] **Step 2: Write config.rs**

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub scan: ScanConfig,
    #[serde(default)]
    pub watch: WatchConfig,
    #[serde(default)]
    pub embedding: EmbeddingConfig,
    #[serde(default)]
    pub plugins: PluginConfig,
    #[serde(default)]
    pub retention: RetentionConfig,
    #[serde(default)]
    pub schedule: ScheduleConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default = "default_socket")]
    pub socket: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScanConfig {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchConfig {
    #[serde(default)]
    pub include: Vec<String>,
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbeddingConfig {
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub external_api_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    #[serde(default = "default_plugin_dir")]
    pub dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionConfig {
    #[serde(default = "default_retention_days")]
    pub deleted_keep_days: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConfig {
    #[serde(default = "default_sync_time")]
    pub full_sync: String,
}

fn default_socket() -> PathBuf {
    dirs_fan().join("fan.sock")
}
fn default_model() -> String { "all-MiniLM-L6-v2".into() }
fn default_plugin_dir() -> PathBuf { dirs_fan().join("plugins") }
fn default_retention_days() -> u32 { 30 }
fn default_sync_time() -> String { "03:00".into() }

impl Default for Config {
    fn default() -> Self {
        Self {
            daemon: DaemonConfig { socket: default_socket() },
            scan: ScanConfig { include: vec![], exclude: vec!["/tmp".into(), "*.tmp".into()] },
            watch: WatchConfig { include: vec![], exclude: vec!["*.tmp".into(), ".*".into()] },
            embedding: EmbeddingConfig { model: default_model(), external_api_url: None },
            plugins: PluginConfig { dir: default_plugin_dir() },
            retention: RetentionConfig { deleted_keep_days: default_retention_days() },
            schedule: ScheduleConfig { full_sync: default_sync_time() },
        }
    }
}

impl Config {
    pub fn load() -> Result<Self, Box<dyn std::error::Error>> {
        let path = dirs_fan().join("config.toml");
        if path.exists() {
            let s = std::fs::read_to_string(&path)?;
            Ok(toml::from_str(&s)?)
        } else {
            let cfg = Config::default();
            std::fs::create_dir_all(dirs_fan())?;
            std::fs::write(&path, toml::to_string_pretty(&cfg)?)?;
            Ok(cfg)
        }
    }
}

pub fn dirs_fan() -> PathBuf {
    dirs_home().join(".fan-files")
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}
```

- [ ] **Step 3: Build**

```bash
cd fan-files && cargo build
```

Expected: COMPILE (fan-core with types + config)

- [ ] **Step 4: Commit**

```bash
git add crates/fan-core/src/types.rs crates/fan-core/src/config.rs
git commit -m "feat: add core types and config system"
```

---

### Task 3: SQLite Index Backend

**Files:**
- Create: `crates/fan-core/src/index/mod.rs`
- Create: `crates/fan-core/src/index/sqlite.rs`

- [ ] **Step 1: Write sqlite.rs — schema and CRUD**

```rust
use crate::types::{FileEntry, IndexStatus, RawFileInfo};
use fan_plugin_sdk::{BioMetadata, FormatInfo};
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(data_dir: &Path) -> rusqlite::Result<Self> {
        std::fs::create_dir_all(data_dir).ok();
        let conn = Connection::open(data_dir.join("index.db"))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        let store = Self { conn: Mutex::new(conn) };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT UNIQUE NOT NULL,
                size INTEGER NOT NULL,
                mtime_secs INTEGER NOT NULL,
                hash_sha256 TEXT,
                magic_bytes BLOB,
                mime_type TEXT,
                format_info_json TEXT,
                bio_metadata_json TEXT,
                indexed_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                deleted INTEGER NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
            CREATE INDEX IF NOT EXISTS idx_files_deleted ON files(deleted);
            CREATE INDEX IF NOT EXISTS idx_files_mtime ON files(mtime_secs);
            CREATE TABLE IF NOT EXISTS tags (
                file_id INTEGER NOT NULL REFERENCES files(id),
                tag TEXT NOT NULL,
                UNIQUE(file_id, tag)
            );
            CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags(tag);
            CREATE TABLE IF NOT EXISTS embeddings (
                file_id INTEGER PRIMARY KEY REFERENCES files(id),
                vector BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS relations (
                file_a_id INTEGER NOT NULL REFERENCES files(id),
                file_b_id INTEGER NOT NULL REFERENCES files(id),
                relation_type TEXT NOT NULL,
                score REAL NOT NULL DEFAULT 0.0,
                UNIQUE(file_a_id, file_b_id, relation_type)
            );"
        )?;
        Ok(())
    }

    pub fn upsert(&self, info: &RawFileInfo, format_info: Option<&FormatInfo>) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        let fi_json = format_info.map(|f| serde_json::to_string(f).unwrap());
        conn.execute(
            "INSERT INTO files (path, size, mtime_secs, hash_sha256, magic_bytes, mime_type, format_info_json, indexed_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(path) DO UPDATE SET
                size=excluded.size, mtime_secs=excluded.mtime_secs,
                hash_sha256=excluded.hash_sha256, magic_bytes=excluded.magic_bytes,
                mime_type=excluded.mime_type, format_info_json=excluded.format_info_json,
                updated_at=excluded.updated_at, deleted=0",
            params![
                info.path.to_string_lossy(), info.size as i64, info.mtime_secs,
                info.hash_sha256, info.magic_bytes, info.mime_type,
                fi_json, now, now,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_bio_metadata(&self, file_id: i64, meta: &BioMetadata) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        let json = serde_json::to_string(meta).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
        conn.execute(
            "UPDATE files SET bio_metadata_json=?1, updated_at=?2 WHERE id=?3",
            params![json, now, file_id],
        )?;
        // Sync tags
        conn.execute("DELETE FROM tags WHERE file_id=?1", params![file_id])?;
        for tag in &meta.tags {
            conn.execute("INSERT OR IGNORE INTO tags (file_id, tag) VALUES (?1, ?2)", params![file_id, tag])?;
        }
        Ok(())
    }

    pub fn mark_deleted(&self, path: &Path) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE files SET deleted=1, updated_at=?1 WHERE path=?2",
            params![
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64,
                path.to_string_lossy(),
            ],
        )?;
        Ok(())
    }

    pub fn purge_old_deleted(&self, keep_days: u32) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let cutoff = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
            - (keep_days as i64 * 86400);
        Ok(conn.execute("DELETE FROM files WHERE deleted=1 AND updated_at < ?1", params![cutoff])?)
    }

    pub fn get_by_path(&self, path: &Path) -> rusqlite::Result<Option<FileEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, path, size, mtime_secs, hash_sha256, magic_bytes, mime_type, format_info_json, bio_metadata_json, indexed_at, updated_at, deleted
             FROM files WHERE path=?1"
        )?;
        let mut rows = stmt.query_map(params![path.to_string_lossy()], |row| {
            Ok(FileEntry {
                id: row.get(0)?,
                path: row.get::<_, String>(1)?.into(),
                size: row.get::<_, i64>(2)? as u64,
                mtime_secs: row.get(3)?,
                hash_sha256: row.get(4)?,
                magic_bytes: row.get(5)?,
                mime_type: row.get(6)?,
                format_info: row.get::<_, Option<String>>(7)?.and_then(|s| serde_json::from_str(&s).ok()),
                bio_metadata: row.get::<_, Option<String>>(8)?.and_then(|s| serde_json::from_str(&s).ok()),
                indexed_at: row.get(9)?,
                updated_at: row.get(10)?,
                deleted: row.get::<_, i32>(11)? != 0,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn get_by_id(&self, id: i64) -> rusqlite::Result<Option<FileEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, path, size, mtime_secs, hash_sha256, magic_bytes, mime_type, format_info_json, bio_metadata_json, indexed_at, updated_at, deleted
             FROM files WHERE id=?1"
        )?;
        let mut rows = stmt.query_map(params![id], |row| {
            Ok(FileEntry {
                id: row.get(0)?,
                path: row.get::<_, String>(1)?.into(),
                size: row.get::<_, i64>(2)? as u64,
                mtime_secs: row.get(3)?,
                hash_sha256: row.get(4)?,
                magic_bytes: row.get(5)?,
                mime_type: row.get(6)?,
                format_info: row.get::<_, Option<String>>(7)?.and_then(|s| serde_json::from_str(&s).ok()),
                bio_metadata: row.get::<_, Option<String>>(8)?.and_then(|s| serde_json::from_str(&s).ok()),
                indexed_at: row.get(9)?,
                updated_at: row.get(10)?,
                deleted: row.get::<_, i32>(11)? != 0,
            })
        })?;
        Ok(rows.next().transpose()?)
    }

    pub fn list_by_tag(&self, tag: &str, limit: usize) -> rusqlite::Result<Vec<FileEntry>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT f.id, f.path, f.size, f.mtime_secs, f.hash_sha256, f.magic_bytes, f.mime_type,
                    f.format_info_json, f.bio_metadata_json, f.indexed_at, f.updated_at, f.deleted
             FROM files f JOIN tags t ON f.id = t.file_id
             WHERE t.tag=?1 AND f.deleted=0 LIMIT ?2"
        )?;
        let rows = stmt.query_map(params![tag, limit as i64], |row| {
            Ok(FileEntry {
                id: row.get(0)?,
                path: row.get::<_, String>(1)?.into(),
                size: row.get::<_, i64>(2)? as u64,
                mtime_secs: row.get(3)?,
                hash_sha256: row.get(4)?,
                magic_bytes: row.get(5)?,
                mime_type: row.get(6)?,
                format_info: row.get::<_, Option<String>>(7)?.and_then(|s| serde_json::from_str(&s).ok()),
                bio_metadata: row.get::<_, Option<String>>(8)?.and_then(|s| serde_json::from_str(&s).ok()),
                indexed_at: row.get(9)?,
                updated_at: row.get(10)?,
                deleted: row.get::<_, i32>(11)? != 0,
            })
        })?;
        rows.collect()
    }

    pub fn all_paths(&self) -> rusqlite::Result<Vec<(i64, String, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT id, path, mtime_secs FROM files WHERE deleted=0")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get(0)?, row.get::<_, String>(1)?, row.get(2)?))
        })?;
        rows.collect()
    }

    pub fn store_embedding(&self, file_id: i64, vector: &[f32]) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        let bytes: Vec<u8> = vector.iter().flat_map(|f| f.to_le_bytes()).collect();
        conn.execute(
            "INSERT OR REPLACE INTO embeddings (file_id, vector) VALUES (?1, ?2)",
            params![file_id, bytes],
        )?;
        Ok(())
    }

    pub fn status(&self) -> rusqlite::Result<IndexStatus> {
        let conn = self.conn.lock().unwrap();
        let total: i64 = conn.query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))?;
        let indexed: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE deleted=0", [], |r| r.get(0))?;
        let deleted: i64 = conn.query_row("SELECT COUNT(*) FROM files WHERE deleted=1", [], |r| r.get(0))?;
        let last_scan: Option<i64> = conn.query_row("SELECT MAX(indexed_at) FROM files", [], |r| r.get(0))?;
        let last_change: Option<i64> = conn.query_row("SELECT MAX(updated_at) FROM files", [], |r| r.get(0))?;
        Ok(IndexStatus {
            total_files: total as u64,
            indexed_files: indexed as u64,
            deleted_files: deleted as u64,
            last_full_scan: last_scan,
            last_change: last_change,
            db_size_bytes: 0,
        })
    }
}
```

- [ ] **Step 2: Write index/mod.rs (facade, placeholder)**

```rust
pub mod sqlite;
pub mod tantivy;
pub mod embedding;

use crate::config::Config;
use crate::types::{FileEntry, IndexStatus, RawFileInfo};
use fan_plugin_sdk::{BioMetadata, FormatInfo, SearchResult};
use sqlite::SqliteStore;
use std::path::Path;
use std::sync::Arc;

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

    pub fn index_file(&self, info: &RawFileInfo, format_info: Option<&FormatInfo>) -> Result<i64, Box<dyn std::error::Error>> {
        let id = self.sqlite.upsert(info, format_info)?;
        let metadata_text = format!("{} {:?}", info.path.display(), format_info);
        self.tantivy.index_file(id, &info.path, &metadata_text, &[])?;
        Ok(id)
    }
}
```

- [ ] **Step 3: Build & verify**

```bash
cd fan-files && cargo build
```

Expected: COMPILE

- [ ] **Step 4: Commit**

```bash
git add crates/fan-core/src/index/ crates/fan-core/src/types.rs crates/fan-core/src/config.rs
git commit -m "feat: add SQLite index backend with schema migration"
```

---

### Task 4: Directory Scanner

**Files:**
- Create: `crates/fan-core/src/scanner.rs`

- [ ] **Step 1: Write scanner.rs**

```rust
use crate::types::RawFileInfo;
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub struct Scanner {
    include_dirs: Vec<String>,
    exclude_patterns: Vec<String>,
}

impl Scanner {
    pub fn new(include: Vec<String>, exclude: Vec<String>) -> Self {
        Self { include_dirs: include, exclude_patterns: exclude }
    }

    pub fn scan(&self) -> impl Iterator<Item = RawFileInfo> + '_ {
        self.include_dirs.iter().flat_map(move |dir| {
            WalkDir::new(dir)
                .follow_links(false)
                .into_iter()
                .filter_entry(move |e| !self.is_excluded(e.path()))
                .filter_map(|entry| {
                    let entry = entry.ok()?;
                    if !entry.file_type().is_file() { return None; }
                    Some(self.collect_info(entry.path()))
                })
        })
    }

    fn collect_info(&self, path: &Path) -> RawFileInfo {
        let meta = fs::metadata(path).ok();
        let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let mtime = meta.and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        let magic = read_magic(path);
        let mime = mime_guess::from_path(path).first_or_octet_stream().to_string();
        RawFileInfo {
            path: path.to_path_buf(),
            size,
            mtime_secs: mtime,
            hash_sha256: None, // lazy: only compute on first index
            magic_bytes: magic,
            mime_type: mime,
        }
    }

    fn is_excluded(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        self.exclude_patterns.iter().any(|pat| {
            if pat.starts_with("*.") {
                path_str.ends_with(&pat[1..])
            } else if pat.starts_with(".*") {
                path.file_name().map(|n| n.to_string_lossy().starts_with(".")).unwrap_or(false)
            } else {
                path_str.starts_with(pat.as_str())
            }
        })
    }

    pub fn scan_single(&self, path: &Path) -> Option<RawFileInfo> {
        if path.is_file() { Some(self.collect_info(path)) } else { None }
    }
}

fn read_magic(path: &Path) -> Vec<u8> {
    fs::File::open(path)
        .ok()
        .and_then(|mut f| {
            use std::io::Read;
            let mut buf = vec![0u8; 512];
            let n = f.read(&mut buf).ok()?;
            buf.truncate(n);
            Some(buf)
        })
        .unwrap_or_default()
}
```

- [ ] **Step 2: Build**

```bash
cd fan-files && cargo build
```

- [ ] **Step 3: Commit**

```bash
git add crates/fan-core/src/scanner.rs
git commit -m "feat: add directory scanner with exclude pattern matching"
```

---

### Task 5: Tantivy Full-Text Index

**Files:**
- Create: `crates/fan-core/src/index/tantivy.rs`

- [ ] **Step 1: Write tantivy.rs**

```rust
use std::path::Path;
use tantivy::collector::TopDocs;
use tantivy::query::QueryParser;
use tantivy::schema::*;
use tantivy::{doc, Index, IndexReader, IndexWriter, ReloadPolicy};
use tantivy::directory::MmapDirectory;
use std::sync::Mutex;

pub struct TantivyIndex {
    schema: Schema,
    writer: Mutex<IndexWriter>,
    reader: IndexReader,
    path_field: Field,
    metadata_field: Field,
    tags_field: Field,
}

impl TantivyIndex {
    pub fn open(data_dir: &Path) -> tantivy::Result<Self> {
        let index_dir = data_dir.join("tantivy");
        std::fs::create_dir_all(&index_dir).ok();

        let mut schema_builder = Schema::builder();
        let path_field = schema_builder.add_text_field("path", STRING | STORED);
        let metadata_field = schema_builder.add_text_field("metadata", TEXT);
        let tags_field = schema_builder.add_text_field("tags", STRING);
        let schema = schema_builder.build();

        let dir = MmapDirectory::open(&index_dir)?;
        let index = Index::open_or_create(dir, schema.clone())?;
        let writer = index.writer(50_000_000)?;
        let reader = index.reader_builder()
            .reload_policy(ReloadPolicy::OnCommitWithDelay)
            .try_into()?;

        Ok(Self {
            schema, reader,
            writer: Mutex::new(writer),
            path_field, metadata_field, tags_field,
        })
    }

    pub fn index_file(&self, id: i64, path: &Path, metadata_text: &str, tags: &[String]) -> tantivy::Result<()> {
        let mut writer = self.writer.lock().unwrap();
        let tags_str = tags.join(" ");
        writer.add_document(doc!(
            self.path_field => path.to_string_lossy().to_string(),
            self.metadata_field => metadata_text,
            self.tags_field => tags_str,
        ))?;
        Ok(())
    }

    pub fn commit(&self) -> tantivy::Result<()> {
        let mut writer = self.writer.lock().unwrap();
        writer.commit()?;
        Ok(())
    }

    pub fn search(&self, query_str: &str, limit: usize) -> tantivy::Result<Vec<(String, f32)>> {
        self.reader.reload()?;
        let searcher = self.reader.searcher();
        let query_parser = QueryParser::for_index(&searcher.index(), vec![
            self.metadata_field, self.tags_field, self.path_field,
        ]);
        let query = query_parser.parse_query(query_str)?;
        let top_docs = searcher.search(&query, &TopDocs::with_limit(limit))?;
        Ok(top_docs.into_iter().map(|(score, doc_addr)| {
            let doc = searcher.doc(doc_addr).unwrap();
            let path = doc.get_first(self.path_field)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            (path, score)
        }).collect())
    }
}
```

- [ ] **Step 2: Build**

```bash
cd fan-files && cargo build
```

- [ ] **Step 3: Commit**

```bash
git add crates/fan-core/src/index/tantivy.rs
git commit -m "feat: add Tantivy full-text index"
```

---

### Task 6: ONNX Embedding Engine

**Files:**
- Create: `crates/fan-core/src/index/embedding.rs`

- [ ] **Step 1: Write embedding.rs**

```rust
use crate::config::Config;
use ort::{Environment, Session, SessionBuilder, Value};
use std::path::PathBuf;
use std::sync::Arc;

pub struct EmbeddingEngine {
    session: Option<Arc<Session>>,
    model_name: String,
    dim: usize,
}

impl EmbeddingEngine {
    pub fn new(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let model_path = Self::ensure_model(&config.embedding.model)?;
        let env = Arc::new(Environment::builder().with_name("fan-files").build()?);
        let session = Arc::new(SessionBuilder::new(&env)?.with_model_from_file(&model_path)?);
        Ok(Self {
            session: Some(session),
            model_name: config.embedding.model.clone(),
            dim: 384,
        })
    }

    fn ensure_model(model_name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let dir = crate::config::dirs_fan().join("models");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join(format!("{}.onnx", model_name));
        if !path.exists() {
            // TODO: download from HuggingFace or bundle with binary
            // For MVP, return error with download instructions
            return Err(format!("Model {} not found at {}. Please download from https://huggingface.co/sentence-transformers/{}",
                model_name, path.display(), model_name).into());
        }
        Ok(path)
    }

    /// Tokenize text into input_ids (simplified — real impl uses tokenizers crate)
    fn tokenize(&self, text: &str) -> Vec<i64> {
        // Basic whitespace tokenization placeholder
        // Real implementation: use tokenizers crate with BERT vocab
        text.split_whitespace()
            .filter_map(|w| w.bytes().next().map(|b| b as i64))
            .collect()
    }

    pub fn embed(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        let session = self.session.as_ref().ok_or("Model not loaded")?;
        // Simplified — real impl uses proper tokenization + ONNX input
        // For MVP: return random vec as placeholder; real model integration in Task 6b
        Ok(vec![0.0_f32; self.dim])
    }

    pub fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, Box<dyn std::error::Error>> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
        if na == 0.0 || nb == 0.0 { return 0.0; }
        (dot / (na * nb)) as f64
    }

    pub fn search_similar(
        &self,
        query_vec: &[f32],
        candidates: &[(i64, Vec<f32>)],
        top_k: usize,
    ) -> Vec<(i64, f64)> {
        let mut scored: Vec<(i64, f64)> = candidates
            .iter()
            .map(|(id, vec)| (*id, Self::cosine_similarity(query_vec, vec)))
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        scored.truncate(top_k);
        scored
    }
}

/// Dummy engine for when ONNX model is not available
pub struct DummyEmbedding;

impl DummyEmbedding {
    pub fn embed(&self, text: &str) -> Vec<f32> {
        // Simple bag-of-words hash embedding as fallback
        let mut vec = vec![0.0f32; 128];
        for (i, b) in text.bytes().enumerate() {
            vec[i % 128] += b as f32 / 255.0;
        }
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 { vec.iter_mut().for_each(|x| *x /= norm); }
        vec
    }
}
```

- [ ] **Step 2: Build**

```bash
cd fan-files && cargo build
```

(May need to adjust `ort` crate features depending on platform)

- [ ] **Step 3: Commit**

```bash
git add crates/fan-core/src/index/embedding.rs
git commit -m "feat: add ONNX embedding engine with dummy fallback"
```

---

### Task 7: WASM Plugin Host

**Files:**
- Create: `crates/fan-core/src/plugin/mod.rs`
- Create: `crates/fan-core/src/plugin/registry.rs`

- [ ] **Step 1: Write plugin/mod.rs — WASM host runtime**

```rust
use fan_plugin_sdk::{BioMetadata, FileContext, FormatInfo, PluginInfo, PluginType};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wasmtime::{Config, Engine, Linker, Module, Store};

pub mod registry;

/// A loaded WASM plugin instance
pub struct WasmPlugin {
    pub info: PluginInfo,
    engine: Engine,
    module: Module,
}

impl WasmPlugin {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let mut config = Config::default();
        config.async_support(false);
        let engine = Engine::new(&config)?;
        let module = Module::from_file(&engine, path)?;
        
        // Extract plugin info from WASM exports (custom section or exported globals)
        // For MVP: derive info from filename
        let info = PluginInfo {
            name: path.file_stem().unwrap().to_string_lossy().to_string(),
            version: "0.1.0".into(),
            description: String::new(),
            plugin_type: PluginType::FormatDetector,
            priority: 50,
        };
        
        Ok(Self { info, engine, module })
    }

    pub fn detect_format(&self, _path: &str, _magic: &[u8]) -> Option<FormatInfo> {
        // WASM call: invoke exported "can_handle" and "detect" functions
        // MVP: return None, rely on built-in detectors
        None
    }

    pub fn interpret_context(&self, _ctx: &FileContext) -> Option<BioMetadata> {
        // WASM call: invoke exported "score" and "extract" functions
        None
    }
}
```

- [ ] **Step 2: Write plugin/registry.rs**

```rust
use super::WasmPlugin;
use fan_plugin_sdk::{BioMetadata, FileContext, FormatInfo, PluginType};
use std::path::PathBuf;

pub struct PluginRegistry {
    plugins_dir: PathBuf,
    format_detectors: Vec<WasmPlugin>,
    context_interpreters: Vec<WasmPlugin>,
}

impl PluginRegistry {
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self { plugins_dir, format_detectors: vec![], context_interpreters: vec![] }
    }

    pub fn discover(&mut self) -> Result<usize, Box<dyn std::error::Error>> {
        let dir = &self.plugins_dir;
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
            return Ok(0);
        }
        let mut count = 0;
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "wasm").unwrap_or(false) {
                if let Ok(plugin) = WasmPlugin::load(&path) {
                    match plugin.info.plugin_type {
                        PluginType::FormatDetector => self.format_detectors.push(plugin),
                        PluginType::ContextInterpreter => self.context_interpreters.push(plugin),
                    }
                    count += 1;
                }
            }
        }
        // Sort by priority (highest first)
        self.format_detectors.sort_by_key(|p| 100 - p.info.priority);
        self.context_interpreters.sort_by_key(|p| 100 - p.info.priority);
        Ok(count)
    }

    pub fn detect_format(&self, path: &str, magic: &[u8]) -> Option<FormatInfo> {
        for plugin in &self.format_detectors {
            if let Some(info) = plugin.detect_format(path, magic) {
                return Some(info);
            }
        }
        None
    }

    pub fn interpret(&self, ctx: &FileContext) -> Vec<(String, f64, BioMetadata)> {
        self.context_interpreters.iter().filter_map(|p| {
            let meta = p.interpret_context(ctx)?;
            // Use a simple heuristic for score if WASM doesn't provide one
            Some((p.info.name.clone(), 0.8, meta))
        }).collect()
    }
}
```

- [ ] **Step 3: Build**

```bash
cd fan-files && cargo build
```

- [ ] **Step 4: Commit**

```bash
git add crates/fan-core/src/plugin/
git commit -m "feat: add WASM plugin host and registry"
```

---

### Task 8: Built-in Format Detectors

**Files:**
- Create: `plugins/generic-detector/Cargo.toml`
- Create: `plugins/generic-detector/src/lib.rs`
- Create: `plugins/fastq-interpreter/Cargo.toml`
- Create: `plugins/fastq-interpreter/src/lib.rs`

- [ ] **Step 1: Create generic-detector WASM plugin**

`plugins/generic-detector/Cargo.toml`:
```toml
[package]
name = "generic-detector"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
fan-plugin-sdk = { path = "../../crates/fan-plugin-sdk" }
```

`plugins/generic-detector/src/lib.rs`:
```rust
use fan_plugin_sdk::FormatInfo;
use std::collections::HashMap;

/// Map magic bytes + extension → file type
static MAGIC_SIGNATURES: &[(&[u8], &str)] = &[
    (b"\x1f\x8b", "gzip-compressed"),
    (b"BZh", "bzip2-compressed"),
    (b"PK\x03\x04", "ZIP archive"),
    (b"\x89PNG", "PNG image"),
    (b"\xff\xd8\xff", "JPEG image"),
    (b"CRAM", "CRAM file"),
    (b"BAM\x01", "BAM file"),
    (b"@HD\t", "SAM header"),
    (b"##fileformat=VCF", "VCF file"),
    (b">", "FASTA (tentative)"),
    (b"@", "FASTQ (tentative)"),
    (b"HDF", "HDF5 file"),
    (b"\x89HDF", "HDF5 file"),
];

#[no_mangle]
pub extern "C" fn can_handle(path: *const u8, path_len: usize, magic: *const u8, magic_len: usize) -> i32 {
    let magic_slice = unsafe { std::slice::from_raw_parts(magic, magic_len) };
    for (sig, _) in MAGIC_SIGNATURES {
        if magic_slice.starts_with(sig) { return 1; }
    }
    // Check by extension
    let path_str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(path, path_len)) };
    let ext = std::path::Path::new(path_str).extension().and_then(|e| e.to_str()).unwrap_or("");
    let known_exts = ["fastq", "fq", "fasta", "fa", "fna", "bam", "sam", "cram", "vcf", "bcf", "gff", "gtf", "bed", "h5", "hdf5", "csv", "tsv", "txt", "gz"];
    if known_exts.contains(&ext) { 1 } else { 0 }
}

#[no_mangle]
pub extern "C" fn detect(path: *const u8, path_len: usize, magic: *const u8, magic_len: usize) -> i32 {
    // Return pointer to JSON-encoded FormatInfo (simplified — real impl uses proper memory management)
    0
}
```

- [ ] **Step 2: Create fastq-interpreter WASM plugin**

`plugins/fastq-interpreter/Cargo.toml`:
```toml
[package]
name = "fastq-interpreter"
version = "0.1.0"
edition = "2024"

[lib]
crate-type = ["cdylib"]

[dependencies]
fan-plugin-sdk = { path = "../../crates/fan-plugin-sdk" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

`plugins/fastq-interpreter/src/lib.rs`:
```rust
use fan_plugin_sdk::BioMetadata;
// Context interpreter for FASTQ files
// Reads filename patterns to infer sequencing type

/// Common FASTQ naming patterns
/// - sample_R1.fastq.gz → paired-end, read 1
/// - sample_R2.fastq.gz → paired-end, read 2
/// - sample.fastq.gz → single-end
#[no_mangle]
pub extern "C" fn score(path: *const u8, path_len: usize) -> f64 {
    let path_str = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(path, path_len)) };
    let lower = path_str.to_lowercase();
    if lower.contains("fastq") || lower.contains(".fq") { 0.9 } else { 0.0 }
}

#[no_mangle]
pub extern "C" fn extract(path: *const u8, path_len: usize) -> i32 {
    // Will infer: paired/single-end, read number, sample name from filename
    // For MVP: return null pointer, built-in Rust code handles common cases
    0
}
```

- [ ] **Step 3: Build WASM targets**

```bash
cd fan-files
cargo build --package generic-detector --target wasm32-unknown-unknown --release
cargo build --package fastq-interpreter --target wasm32-unknown-unknown --release
```

(May need `rustup target add wasm32-unknown-unknown`)

- [ ] **Step 4: Commit**

```bash
git add plugins/
git commit -m "feat: add built-in generic-detector and fastq-interpreter WASM plugins"
```

---

### Task 9: File Watcher

**Files:**
- Create: `crates/fan-core/src/watcher.rs`

- [ ] **Step 1: Write watcher.rs**

```rust
use notify::{Config as NotifyConfig, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use tracing::{debug, info};

pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    rx: Receiver<Vec<PathBuf>>,
    batch_tx: Sender<PathBuf>,
}

#[derive(Debug, Clone)]
pub enum ChangeEvent {
    Created(PathBuf),
    Modified(PathBuf),
    Removed(PathBuf),
}

impl FileWatcher {
    pub fn new(dirs: &[String]) -> notify::Result<Self> {
        let (batch_tx, batch_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        
        // Batching goroutine
        std::thread::spawn(move || {
            let mut batch: Vec<PathBuf> = Vec::new();
            let mut last_flush = Instant::now();
            loop {
                match batch_rx.recv_timeout(Duration::from_secs(1)) {
                    Ok(path) => {
                        // Dedup: check if same path already in batch
                        if !batch.contains(&path) { batch.push(path); }
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
                if !batch.is_empty() && (batch.len() >= 100 || last_flush.elapsed() > Duration::from_secs(5)) {
                    let drained: Vec<PathBuf> = batch.drain(..).collect();
                    event_tx.send(drained).ok();
                    last_flush = Instant::now();
                }
            }
        });
        
        let mut watcher = RecommendedWatcher::new(
            move |res: notify::Result<Event>| {
                if let Ok(event) = res {
                    match event.kind {
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                            for path in event.paths {
                                batch_tx.send(path).ok();
                            }
                        }
                        _ => {}
                    }
                }
            },
            NotifyConfig::default(),
        )?;
        
        for dir in dirs {
            if Path::new(dir).exists() {
                watcher.watch(Path::new(dir), RecursiveMode::Recursive)?;
                info!("Watching: {}", dir);
            }
        }
        
        Ok(Self { _watcher: watcher, rx: event_rx, batch_tx })
    }
    
    pub fn events(&self) -> &Receiver<Vec<PathBuf>> {
        &self.rx
    }
}
```

- [ ] **Step 2: Build**

```bash
cd fan-files && cargo build
```

- [ ] **Step 3: Commit**

```bash
git add crates/fan-core/src/watcher.rs
git commit -m "feat: add file watcher with batching and dedup"
```

---

### Task 10: Recommendation Engine

**Files:**
- Create: `crates/fan-core/src/suggest.rs`

- [ ] **Step 1: Write suggest.rs**

```rust
use crate::index::IndexEngine;
use crate::types::FileEntry;
use fan_plugin_sdk::{BioMetadata, SearchResult, DataSource};
use std::collections::HashMap;

/// 实验类型互补矩阵
static COMPLEMENTARY_ASSAYS: &[(&str, &[&str])] = &[
    ("RNA-seq", &["ChIP-seq", "ATAC-seq", "WGBS"]),
    ("WGS", &["WGBS", "RNA-seq", "ChIP-seq"]),
    ("scRNA-seq", &["scATAC-seq", "CITE-seq"]),
    ("ChIP-seq", &["RNA-seq", "ATAC-seq"]),
    ("ATAC-seq", &["RNA-seq", "ChIP-seq"]),
    ("WGBS", &["RNA-seq", "WGS"]),
];

pub struct SuggestEngine;

impl SuggestEngine {
    /// Given a project directory, suggest related datasets on the server
    pub fn suggest(
        index: &IndexEngine,
        project_dir: &str,
        limit: usize,
    ) -> Result<Vec<SearchResult>, Box<dyn std::error::Error>> {
        // 1. Get metadata for files in the project directory
        let project_files = index.tantivy.search(&format!("path:{}", project_dir), 50)?;
        
        // 2. Collect bio metadata from those files
        let mut project_meta = Vec::new();
        for (path, _) in &project_files {
            if let Some(entry) = index.sqlite.get_by_path(std::path::Path::new(path)).ok().flatten() {
                if let Some(meta) = &entry.bio_metadata {
                    project_meta.push(meta.clone());
                }
            }
        }
        
        if project_meta.is_empty() {
            return Ok(vec![]);
        }
        
        // 3. Extract key dimensions
        let species = project_meta.iter().find_map(|m| m.species.clone());
        let tissue = project_meta.iter().find_map(|m| m.tissue.clone());
        let project = project_meta.iter().find_map(|m| m.project.clone());
        let assay_types: Vec<String> = project_meta.iter()
            .filter_map(|m| m.assay_type.clone()).collect();
        
        // 4. Find complementary assay types
        let want_assays: Vec<String> = assay_types.iter()
            .flat_map(|a| {
                COMPLEMENTARY_ASSAYS.iter()
                    .filter(|(k, _)| k == &a.as_str())
                    .flat_map(|(_, v)| v.iter().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .collect();
        
        // 5. Score all indexed files
        // (In production: pre-compute and cache. MVP: iterate SQLite)
        let candidates = index.sqlite.all_paths().unwrap_or_default();
        let mut scored: Vec<SearchResult> = Vec::new();
        
        for (id, path_str, _) in &candidates {
            if project_files.iter().any(|(p, _)| p == path_str) { continue; }
            if let Some(entry) = index.sqlite.get_by_id(*id).ok().flatten() {
                let mut score = 0.0;
                let mut reasons: Vec<String> = Vec::new();
                
                if let Some(ref meta) = entry.bio_metadata {
                    if species.as_ref().zip(meta.species.as_ref()).is_some() {
                        score += 0.3;
                        reasons.push("same species".into());
                    }
                    if tissue.as_ref().zip(meta.tissue.as_ref()).is_some() {
                        score += 0.2;
                        reasons.push("same tissue".into());
                    }
                    if project.as_ref().zip(meta.project.as_ref()).is_some() {
                        score += 0.3;
                        reasons.push("same project".into());
                    }
                    if meta.assay_type.as_ref().map(|a| want_assays.contains(a)).unwrap_or(false) {
                        score += 0.15;
                        reasons.push("complementary assay".into());
                    }
                }
                
                if score > 0.1 {
                    scored.push(SearchResult {
                        path: path_str.clone(),
                        score,
                        file_type: entry.format_info.as_ref().map(|f| f.file_type.clone()),
                        assay_type: entry.bio_metadata.as_ref().and_then(|m| m.assay_type.clone()),
                        species: entry.bio_metadata.as_ref().and_then(|m| m.species.clone()),
                        tags: entry.bio_metadata.as_ref().map(|m| m.tags.clone()).unwrap_or_default(),
                        summary: reasons.join(", "),
                        source: DataSource::Local,
                    });
                }
            }
        }
        
        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());
        scored.truncate(limit);
        
        // 6. Merge public data results (reserved)
        Ok(scored)
    }
}
```

- [ ] **Step 2: Build**

```bash
cd fan-files && cargo build
```

- [ ] **Step 3: Commit**

```bash
git add crates/fan-core/src/suggest.rs
git commit -m "feat: add recommendation engine with complementary assay matrix"
```

---

### Task 11: CLI — clap entry point + daemon mode

**Files:**
- Modify: `crates/fan-files/src/main.rs`
- Create: `crates/fan-files/src/commands/mod.rs`
- Create: `crates/fan-files/src/commands/daemon.rs`

- [ ] **Step 1: Write main.rs with clap CLI**

```rust
mod commands;
mod skill;

use clap::{Parser, Subcommand};
use fan_core::config::Config;

#[derive(Parser)]
#[command(name = "fan-files", version = "0.1.0", about = "智能文件元数据检索引擎")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start daemon (scan + watch + serve)
    Daemon,
    /// Search files by natural language query
    Search {
        query: String,
        #[arg(long)]
        json: bool,
    },
    /// Suggest related datasets for a project directory
    Suggest {
        path: String,
        #[arg(long)]
        json: bool,
    },
    /// List files by category/tag
    List {
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Get detailed metadata for a file
    Info {
        path: String,
        #[arg(long)]
        json: bool,
    },
    /// Show index status
    Status,
}

fn main() {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    let config = Config::load().expect("Failed to load config");

    match cli.command {
        Commands::Daemon => commands::daemon::run(&config),
        Commands::Search { query, json } => commands::search::run(&config, &query, json),
        Commands::Suggest { path, json } => commands::suggest::run(&config, &path, json),
        Commands::List { category, tag, json } => commands::list::run(&config, category, tag, json),
        Commands::Info { path, json } => commands::info::run(&config, &path, json),
        Commands::Status => commands::status::run(&config),
    }
}
```

- [ ] **Step 2: Write commands/mod.rs**

```rust
pub mod daemon;
pub mod search;
pub mod suggest;
pub mod list;
pub mod info;
pub mod status;
```

- [ ] **Step 3: Write commands/daemon.rs**

```rust
use fan_core::config::Config;
use fan_core::index::IndexEngine;
use fan_core::plugin::registry::PluginRegistry;
use fan_core::scanner::Scanner;
use fan_core::watcher::FileWatcher;
use tracing::{error, info};

pub fn run(config: &Config) {
    info!("Starting fan-files daemon...");
    
    let index = IndexEngine::open(config).expect("Failed to open index");
    let mut plugins = PluginRegistry::new(config.plugins.dir.clone());
    let n = plugins.discover().unwrap_or(0);
    info!("Loaded {} plugins", n);
    
    // Initial scan
    let scanner = Scanner::new(config.scan.include.clone(), config.scan.exclude.clone());
    info!("Starting initial scan...");
    for info in scanner.scan() {
        let format_info = plugins.detect_format(
            &info.path.to_string_lossy(),
            &info.magic_bytes,
        );
        if let Err(e) = index.index_file(&info, format_info.as_ref()) {
            error!("Failed to index {}: {}", info.path.display(), e);
        }
    }
    index.tantivy.commit().ok();
    info!("Initial scan complete");
    
    // Start watcher
    if !config.watch.include.is_empty() {
        let watcher = FileWatcher::new(&config.watch.include).expect("Failed to start watcher");
        info!("File watcher started");
        
        loop {
            match watcher.events().recv() {
                Ok(paths) => {
                    for path in paths {
                        info!("Changed: {}", path.display());
                        if let Some(info) = scanner.scan_single(&path) {
                            let format_info = plugins.detect_format(
                                &info.path.to_string_lossy(),
                                &info.magic_bytes,
                            );
                            index.index_file(&info, format_info.as_ref()).ok();
                        } else if !path.exists() {
                            index.sqlite.mark_deleted(&path).ok();
                        }
                    }
                    index.tantivy.commit().ok();
                }
                Err(_) => break,
            }
        }
    }
}
```

- [ ] **Step 4: Build**

```bash
cd fan-files && cargo build
```

- [ ] **Step 5: Commit**

```bash
git add crates/fan-files/src/
git commit -m "feat: add CLI with clap and daemon mode"
```

---

### Task 12: CLI — search, suggest, list, info, status commands

**Files:**
- Create: `crates/fan-files/src/commands/search.rs`
- Create: `crates/fan-files/src/commands/suggest.rs`
- Create: `crates/fan-files/src/commands/list.rs`
- Create: `crates/fan-files/src/commands/info.rs`
- Create: `crates/fan-files/src/commands/status.rs`

- [ ] **Step 1: Write search.rs**

```rust
use fan_core::config::Config;
use fan_core::index::IndexEngine;
use fan_plugin_sdk::{SearchResult, DataSource};

pub fn run(config: &Config, query: &str, json: bool) {
    let index = IndexEngine::open(config).expect("Failed to open index");
    
    // Search tantivy for keyword matches
    let tantivy_results = index.tantivy.search(query, 20).unwrap_or_default();
    
    // Semantic search via embedding (if model available)
    let query_embedding = index.embedding.embed(query).unwrap_or_default();
    
    let mut results: Vec<SearchResult> = Vec::new();
    for (path, score) in &tantivy_results {
        if let Some(entry) = index.sqlite.get_by_path(std::path::Path::new(path)).ok().flatten() {
            results.push(SearchResult {
                path: path.clone(),
                score: *score as f64,
                file_type: entry.format_info.as_ref().map(|f| f.file_type.clone()),
                assay_type: entry.bio_metadata.as_ref().and_then(|m| m.assay_type.clone()),
                species: entry.bio_metadata.as_ref().and_then(|m| m.species.clone()),
                tags: entry.bio_metadata.as_ref().map(|m| m.tags.clone()).unwrap_or_default(),
                summary: entry.bio_metadata.as_ref()
                    .map(|m| format!("{:?}", m))
                    .unwrap_or_default(),
                source: DataSource::Local,
            });
        }
    }
    
    if json {
        println!("{}", serde_json::to_string_pretty(&results).unwrap());
    } else {
        for r in &results {
            println!("{:.3}  {}  {:?}  {:?}", r.score, r.path, r.assay_type, r.species);
        }
    }
}
```

- [ ] **Step 2: Write suggest.rs**

```rust
use fan_core::config::Config;
use fan_core::index::IndexEngine;
use fan_core::suggest::SuggestEngine;

pub fn run(config: &Config, path: &str, json: bool) {
    let index = IndexEngine::open(config).expect("Failed to open index");
    let suggestions = SuggestEngine::suggest(&index, path, 10).unwrap_or_default();
    
    if json {
        println!("{}", serde_json::to_string_pretty(&suggestions).unwrap());
    } else {
        for s in &suggestions {
            println!("{:.3}  {}  {}  {}", s.score, s.path, s.assay_type.as_deref().unwrap_or("-"), s.summary);
        }
    }
}
```

- [ ] **Step 3: Write list.rs**

```rust
use fan_core::config::Config;
use fan_core::index::IndexEngine;

pub fn run(config: &Config, category: Option<String>, tag: Option<String>, json: bool) {
    let index = IndexEngine::open(config).expect("Failed to open index");
    
    let results = if let Some(tag) = tag {
        index.sqlite.list_by_tag(&tag, 100).unwrap_or_default()
    } else {
        // Fallback: tantivy search by category keyword
        let q = category.unwrap_or_default();
        let paths = index.tantivy.search(&q, 100).unwrap_or_default();
        paths.iter().filter_map(|(p, _)| {
            index.sqlite.get_by_path(std::path::Path::new(p)).ok().flatten()
        }).collect()
    };
    
    if json {
        println!("{}", serde_json::to_string_pretty(&results.iter().map(|e| {
            serde_json::json!({
                "path": e.path,
                "type": e.format_info.as_ref().map(|f| &f.file_type),
                "assay": e.bio_metadata.as_ref().and_then(|m| m.assay_type.as_ref()),
                "species": e.bio_metadata.as_ref().and_then(|m| m.species.as_ref()),
                "tags": e.bio_metadata.as_ref().map(|m| &m.tags),
            })
        }).collect::<Vec<_>>()).unwrap());
    } else {
        for e in &results {
            let assay = e.bio_metadata.as_ref().and_then(|m| m.assay_type.as_ref()).map(|s| s.as_str()).unwrap_or("-");
            println!("{}  [{}]", e.path.display(), assay);
        }
    }
}
```

- [ ] **Step 4: Write info.rs**

```rust
use fan_core::config::Config;
use fan_core::index::IndexEngine;
use std::path::Path;

pub fn run(config: &Config, path: &str, json: bool) {
    let index = IndexEngine::open(config).expect("Failed to open index");
    
    match index.sqlite.get_by_path(Path::new(path)) {
        Ok(Some(entry)) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&serde_json::json!({
                    "path": entry.path,
                    "size": entry.size,
                    "mtime": entry.mtime_secs,
                    "mime": entry.mime_type,
                    "format": entry.format_info,
                    "bio_metadata": entry.bio_metadata,
                    "indexed_at": entry.indexed_at,
                })).unwrap());
            } else {
                println!("Path: {}", entry.path.display());
                println!("Size: {} bytes", entry.size);
                println!("MIME: {}", entry.mime_type.as_deref().unwrap_or("-"));
                println!("Format: {:?}", entry.format_info);
                println!("Bio: {:?}", entry.bio_metadata);
            }
        }
        Ok(None) => eprintln!("File not in index: {}", path),
        Err(e) => eprintln!("Error: {}", e),
    }
}
```

- [ ] **Step 5: Write status.rs**

```rust
use fan_core::config::Config;
use fan_core::index::IndexEngine;

pub fn run(config: &Config) {
    let index = IndexEngine::open(config).expect("Failed to open index");
    let status = index.sqlite.status().unwrap();
    println!("Indexed files:  {}", status.indexed_files);
    println!("Total tracked:  {}", status.total_files);
    println!("Deleted (soft): {}", status.deleted_files);
    if let Some(ts) = status.last_full_scan {
        println!("Last scan:      {} (unix timestamp)", ts);
    }
    if let Some(ts) = status.last_change {
        println!("Last change:    {} (unix timestamp)", ts);
    }
}
```

- [ ] **Step 6: Build**

```bash
cd fan-files && cargo build
```

- [ ] **Step 7: Commit**

```bash
git add crates/fan-files/src/commands/
git commit -m "feat: add search, suggest, list, info, status CLI commands"
```

---

### Task 13: Claude Code Skill Generation

**Files:**
- Create: `crates/fan-files/src/skill.rs`
- Create: `skill/fan-files.md`

- [ ] **Step 1: Write skill.rs (generate-skill subcommand)**

```rust
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
pub struct GenerateSkill {
    #[arg(long, default_value = "skill/fan-files.md")]
    output: PathBuf,
}

pub fn run(opts: &GenerateSkill) {
    let content = include_str!("../../skill/fan-files.md");
    std::fs::write(&opts.output, content).expect("Failed to write skill file");
    println!("Skill written to {}", opts.output.display());
}
```

- [ ] **Step 2: Write skill/fan-files.md**

```markdown
---
name: fan-files
description: Use when analyzing bioinformatics data - find data files, reference genomes, and related datasets on the server
---

# Fan-Files: Server Data Intelligence

You have access to a tool called `fan-files` that knows about ALL files on this server.
Use it whenever you need to find data files, reference genomes, or discover related datasets.

## When to Use fan-files

### Before Starting Analysis
When a user asks you to analyze data, FIRST check what's available:
```
fan-files search "<description of what you need>"
```
Examples:
- `fan-files search "human reference genome hg38"`
- `fan-files search "lung cancer RNA-seq data"`
- `fan-files search "gene annotation file GTF"`

### During Analysis
When you sense the user might benefit from additional data:
```
fan-files suggest <current_project_directory>
```
This tells you what other data on the server could be analyzed together.

### Listing Available Resources
```
fan-files list --category genome     # List all reference genomes
fan-files list --category rnaseq     # List all RNA-seq datasets
fan-files list --tag human           # List human-related data
```

### Getting File Details
```
fan-files info <path>               # Get complete metadata for a file
```

### Checking System Status
```
fan-files status                     # Check index coverage
```

## Best Practices

1. **Always check before using a reference genome** - the server likely has it indexed
2. **Suggest complementary data** - if user analyzes RNA-seq, check for matching ChIP-seq or ATAC-seq
3. **Use `--json`** when you need to parse results programmatically
4. **Be proactive** - mention available data even if the user didn't ask
```

- [ ] **Step 3: Commit**

```bash
git add skill/ crates/fan-files/src/skill.rs
git commit -m "feat: add Claude Code Skill file and generation command"
```

---

### Task 14: Integration Test & Config Default

**Files:**
- Create: `config/default.toml`
- Create: `crates/fan-core/tests/integration_test.rs`
- Modify: `crates/fan-core/Cargo.toml` (add dev-dependencies)

- [ ] **Step 1: Write integration test**

```rust
use fan_core::config::Config;
use fan_core::scanner::Scanner;
use std::fs;
use std::io::Write;

#[test]
fn test_scanner_discovers_files() {
    let tmp = tempfile::tempdir().unwrap();
    let file_path = tmp.path().join("test.fastq");
    fs::File::create(&file_path).unwrap()
        .write_all(b"@SEQ_ID\nACGT\n+\nIIII\n").unwrap();
    
    let scanner = Scanner::new(
        vec![tmp.path().to_string_lossy().to_string()],
        vec![],
    );
    
    let files: Vec<_> = scanner.scan().collect();
    assert!(!files.is_empty());
    assert!(files.iter().any(|f| f.path == file_path));
}
```

- [ ] **Step 2: Add dev-dependencies to fan-core/Cargo.toml**

```toml
[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Write config/default.toml**

```toml
[scan]
include = ["/data"]
exclude = ["/tmp", "*.tmp"]

[watch]
include = ["/data"]
exclude = ["/tmp", "*.tmp", ".*"]

[embedding]
model = "all-MiniLM-L6-v2"

[plugins]
dir = "~/.fan-files/plugins"

[retention]
deleted_keep_days = 30

[schedule]
full_sync = "03:00"
```

- [ ] **Step 4: Run test**

```bash
cd fan-files && cargo test
```

- [ ] **Step 5: Commit**

```bash
git add config/ crates/fan-core/tests/ crates/fan-core/Cargo.toml
git commit -m "test: add scanner integration test and default config"
```

---

### Task 15: Binary Size Optimization & Release Build

**Files:**
- Modify: `Cargo.toml` (workspace root, add profile)
- Modify: `crates/fan-files/Cargo.toml`

- [ ] **Step 1: Add release profile for small binary**

Add to workspace `Cargo.toml`:
```toml
[profile.release]
opt-level = "z"      # Optimize for size
lto = true           # Link-time optimization
codegen-units = 1    # Single codegen unit for better LTO
strip = true         # Strip symbols
panic = "abort"      # Smaller panic handler
```

- [ ] **Step 2: Build release**

```bash
cd fan-files && cargo build --release
ls -lh target/release/fan-files
```

Expected: single binary, ~10-25MB (with embedded SQLite + tantivy)

- [ ] **Step 3: Verify CLI works**

```bash
./target/release/fan-files --version
./target/release/fan-files --help
```

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml
git commit -m "build: add release profile for optimized binary size"
```
