# Source Server Tracking — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `source_server` field to fan-files data model + SSH remote scanner so Mac mini can scan multiple servers and know the origin of every indexed file.

**Architecture:** Add a `source_server` column to the `files` and `project` SQLite tables. Introduce `ServerConfig` to the config TOML. Build `RemoteScanner` that uses `ssh` + `find` to discover files on remote servers. Wire remote scanning into the daemon loop. Update all CLI commands to display server origin.

**Tech Stack:** Rust 1.96, rusqlite, tokio, walkdir (existing), std::process::Command (for SSH)

---

## Task 1: Add `source_server` to data types (types.rs)

**Files:**
- Modify: `crates/fan-core/src/types.rs` (entire file)

**Purpose:** Every file record carries its origin server name through the pipeline.

- [ ] **Step 1: Add `source_server` to `FileEntry` and `RawFileInfo`, add `ServerStats`, update `IndexStatus`**

Replace the entire content of `crates/fan-core/src/types.rs`:

```rust
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
```

- [ ] **Step 2: Stage and commit**

```bash
git add crates/fan-core/src/types.rs
git commit -m "feat: add source_server to FileEntry/RawFileInfo, add ServerStats to IndexStatus

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Task 2: Update Scanner to set `source_server` (scanner.rs)

**Files:**
- Modify: `crates/fan-core/src/scanner.rs` (lines 17-68, collect_info)

**Purpose:** The local Scanner must populate `source_server` on each `RawFileInfo` so the DB layer can persist it. Add a `source_server` parameter.

- [ ] **Step 1: Add `source_server` field to Scanner and populate in `collect_info`**

Replace `crates/fan-core/src/scanner.rs` content:

```rust
use crate::types::RawFileInfo;
use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;
use walkdir::WalkDir;

pub struct Scanner {
    include_dirs: Vec<String>,
    exclude_patterns: Vec<String>,
    source_server: String,
}

impl Scanner {
    pub fn new(include: Vec<String>, exclude: Vec<String>, source_server: String) -> Self {
        Self { include_dirs: include, exclude_patterns: exclude, source_server }
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
            source_server: self.source_server.clone(),
            size,
            mtime_secs: mtime,
            hash_sha256: None,
            magic_bytes: magic,
            mime_type: mime,
        }
    }

    fn is_excluded(&self, path: &Path) -> bool {
        let path_str = path.to_string_lossy();
        let file_name = path.file_name().map(|n| n.to_string_lossy());
        self.exclude_patterns.iter().any(|pat| {
            if pat.starts_with("*.") {
                path_str.ends_with(&pat[1..])
            } else if pat == ".*" {
                file_name.as_ref().map(|n| n.starts_with('.')).unwrap_or(false)
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

- [ ] **Step 2: Build check — Scanner::new now takes 3 args**

```bash
source "$HOME/.cargo/env" && cargo check 2>&1
```

Expected: compilation errors in daemon.rs and init.rs — expected, they still pass 2 args to `Scanner::new`. Will fix in Task 12.

- [ ] **Step 3: Commit**

```bash
git add crates/fan-core/src/scanner.rs
git commit -m "feat: add source_server to Scanner, populate in collect_info

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Task 3: SQLite migration + query updates (sqlite.rs)

**Files:**
- Modify: `crates/fan-core/src/index/sqlite.rs`

**Purpose:** Add `source_server` column to the `files` table, update all INSERT/SELECT queries, add a `status_by_server()` query method.

- [ ] **Step 1: Add migration v2 in `migrate()`**

In `crates/fan-core/src/index/sqlite.rs`, inside `fn migrate(&self)`, add this block AFTER the existing `CREATE TABLE IF NOT EXISTS project_relation` statement (before the closing `));`):

```rust
            // v2 migration: source_server tracking
            {
                let version: i64 = conn
                    .query_row(
                        "PRAGMA user_version",
                        [],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                if version < 2 {
                    conn.execute_batch(
                        "ALTER TABLE files ADD COLUMN source_server TEXT NOT NULL DEFAULT 'local';
                         ALTER TABLE project ADD COLUMN source_server TEXT DEFAULT 'local';
                         CREATE INDEX IF NOT EXISTS idx_files_server ON files(source_server);
                         PRAGMA user_version = 2;",
                    )?;
                }
            }
```

- [ ] **Step 2: Update `map_row` to read the new column**

Replace the `map_row` function (lines 96-115). The new column `source_server` is at position 12 (inserted after `path` at position 1, shifting positions 2+):

```rust
    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<FileEntry> {
        Ok(FileEntry {
            id: row.get(0)?,
            path: row.get::<_, String>(1)?.into(),
            source_server: row.get(2)?,
            size: row.get::<_, i64>(3)? as u64,
            mtime_secs: row.get(4)?,
            hash_sha256: row.get(5)?,
            magic_bytes: row.get(6)?,
            mime_type: row.get(7)?,
            format_info: row
                .get::<_, Option<String>>(8)?
                .and_then(|s| serde_json::from_str(&s).ok()),
            bio_metadata: row
                .get::<_, Option<String>>(9)?
                .and_then(|s| serde_json::from_str(&s).ok()),
            indexed_at: row.get(10)?,
            updated_at: row.get(11)?,
            deleted: row.get::<_, i32>(12)? != 0,
        })
    }
```

- [ ] **Step 3: Update `upsert` to INSERT the `source_server` column**

Replace the `upsert` method (lines 117-147):

```rust
    pub fn upsert(
        &self,
        info: &RawFileInfo,
        format_info: Option<&FormatInfo>,
    ) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = Self::now();
        let fi_json = format_info.map(|f| serde_json::to_string(f).unwrap());
        conn.execute(
            "INSERT INTO files (path, source_server, size, mtime_secs, hash_sha256, magic_bytes, mime_type, \
             format_info_json, indexed_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT(path) DO UPDATE SET
                source_server=excluded.source_server,
                size=excluded.size, mtime_secs=excluded.mtime_secs,
                hash_sha256=excluded.hash_sha256, magic_bytes=excluded.magic_bytes,
                mime_type=excluded.mime_type, format_info_json=excluded.format_info_json,
                updated_at=excluded.updated_at, deleted=0",
            params![
                info.path.to_string_lossy(),
                info.source_server,
                info.size as i64,
                info.mtime_secs,
                info.hash_sha256,
                info.magic_bytes,
                info.mime_type,
                fi_json,
                now,
                now,
            ],
        )?;
        Ok(conn.last_insert_rowid())
    }
```

- [ ] **Step 4: Update all SELECT queries to include `source_server`**

Every SELECT that uses `map_row` must include the new column. Replace the SELECT field lists in these methods — just update the column lists, keeping everything else the same:

In `get_by_path` (line 188), change the SELECT to:
```rust
            "SELECT id, path, source_server, size, mtime_secs, hash_sha256, magic_bytes, mime_type, \
             format_info_json, bio_metadata_json, indexed_at, updated_at, deleted
             FROM files WHERE path=?1",
```

In `get_by_id` (line 200), change the SELECT to:
```rust
            "SELECT id, path, source_server, size, mtime_secs, hash_sha256, magic_bytes, mime_type, \
             format_info_json, bio_metadata_json, indexed_at, updated_at, deleted
             FROM files WHERE id=?1",
```

In `list_by_tag` (line 210), change the SELECT to:
```rust
            "SELECT f.id, f.path, f.source_server, f.size, f.mtime_secs, f.hash_sha256, f.magic_bytes, \
             f.mime_type, f.format_info_json, f.bio_metadata_json, f.indexed_at, \
             f.updated_at, f.deleted
             FROM files f JOIN tags t ON f.id = t.file_id
             WHERE t.tag=?1 AND f.deleted=0 LIMIT ?2",
```

- [ ] **Step 5: Add `status_by_server` method**

Add this new method after the `status` method (after line 303):

```rust
    pub fn status_by_server(&self) -> rusqlite::Result<Vec<crate::types::ServerStats>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT source_server, COUNT(*) as cnt, MAX(indexed_at)
             FROM files WHERE deleted=0
             GROUP BY source_server ORDER BY cnt DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(crate::types::ServerStats {
                server: row.get(0)?,
                file_count: row.get::<_, i64>(1)? as u64,
                last_scan: row.get(2)?,
            })
        })?;
        rows.collect()
    }
```

- [ ] **Step 6: Update `count_with_bio_metadata` to filter by server (optional parameter)**

Replace the existing `count_with_bio_metadata` (lines 275-283) with a version that accepts an optional server filter:

```rust
    pub fn count_with_bio_metadata(&self) -> rusqlite::Result<u64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT COUNT(*) FROM files WHERE bio_metadata_json IS NOT NULL AND bio_metadata_json != '' AND deleted=0",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c as u64)
    }

    pub fn search_by_server(
        &self,
        server: &str,
        limit: usize,
    ) -> rusqlite::Result<Vec<(i64, String, i64)>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, path, mtime_secs FROM files WHERE source_server=?1 AND deleted=0 LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![server, limit as i64], |row| {
            Ok((row.get(0)?, row.get::<_, String>(1)?, row.get(2)?))
        })?;
        rows.collect()
    }
```

- [ ] **Step 7: Build check**

```bash
source "$HOME/.cargo/env" && cargo check 2>&1
```

Expected: sqlite.rs compiles cleanly. Other crates may have errors from Scanner::new signature change — expected, fix in later tasks.

- [ ] **Step 8: Commit**

```bash
git add crates/fan-core/src/index/sqlite.rs
git commit -m "feat: add source_server column to files table, migration v2, per-server stats query

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Task 4: Add ServerConfig to config (config.rs)

**Files:**
- Modify: `crates/fan-core/src/config.rs`

**Purpose:** Load `[servers]` from `config.toml`, with backward compatibility for old `scan.include`.

- [ ] **Step 1: Add `ServerConfig` and `ServersConfig` structs, update `Config`**

In `crates/fan-core/src/config.rs`, add these structs after `ScheduleConfig` (after line 112):

```rust
/// 服务器注册表配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServersConfig {
    #[serde(flatten)]
    pub servers: std::collections::HashMap<String, ServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// SSH Host 名（~/.ssh/config 中定义），空字符串 = 本地
    pub host: String,
    /// 扫描根目录
    pub scan_root: String,
    /// 人类可读的描述（可选）
    #[serde(default)]
    pub label: Option<String>,
    /// 是否参与扫描
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool { true }
```

- [ ] **Step 2: Add `servers` field to `Config`**

In the `Config` struct (lines 5-22), add a `servers` field after `llm`:

```rust
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
    #[serde(default)]
    pub llm: LlmConfig,
    #[serde(default)]
    pub servers: ServersConfig,
}
```

- [ ] **Step 3: Update `Config::default()` to include `servers`**

In the `impl Default for Config` block (lines 153-183), add the servers field:

```rust
            servers: ServersConfig::default(),
```

- [ ] **Step 4: Add a helper method on `Config` for server resolution**

After the `Config::load()` method (after line 198), add:

```rust
    /// Return the list of (server_name, ServerConfig) for enabled servers.
    /// If `servers` map is empty but `scan.include` is populated (old config),
    /// implicitly treat that as a single "local" server.
    pub fn enabled_servers(&self) -> Vec<(String, ServerConfig)> {
        if self.servers.servers.is_empty() && !self.scan.include.is_empty() {
            vec![(
                "local".to_string(),
                ServerConfig {
                    host: String::new(),
                    scan_root: self.scan.include.first().cloned().unwrap_or_default(),
                    label: Some("本地 (自动迁移)".to_string()),
                    enabled: true,
                },
            )]
        } else {
            let mut v: Vec<_> = self
                .servers
                .servers
                .iter()
                .filter(|(_, cfg)| cfg.enabled)
                .map(|(name, cfg)| (name.clone(), cfg.clone()))
                .collect();
            v.sort_by(|a, b| a.0.cmp(&b.0));
            v
        }
    }
```

- [ ] **Step 5: Update `lib.rs` to export new types**

In `crates/fan-core/src/lib.rs`, no change needed — the config module is already `pub mod config`. The new types are re-exported through it.

- [ ] **Step 6: Build check**

```bash
source "$HOME/.cargo/env" && cargo check 2>&1
```

Expected: config.rs compiles. Still have Scanner::new errors in daemon.rs and init.rs.

- [ ] **Step 7: Commit**

```bash
git add crates/fan-core/src/config.rs
git commit -m "feat: add ServersConfig/ServerConfig, enabled_servers() helper with old-config fallback

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Task 5: Add RemoteScanner (scanner.rs)

**Files:**
- Modify: `crates/fan-core/src/scanner.rs`

**Purpose:** Add a `RemoteScanner` that uses `ssh` + `find -printf` to discover files on remote servers. Provides an iterator of `RawFileInfo` (same as local scanner) so daemon can use a uniform interface.

- [ ] **Step 1: Append `RemoteScanner` struct to scanner.rs**

Add this code at the end of `crates/fan-core/src/scanner.rs`:

```rust
/// RemoteScanner discovers files on a remote server via SSH.
/// Uses `ssh <host> find <root> -type f -printf` for file listing
/// and `ssh <host> head -c 512 <path> | base64` for magic bytes.
pub struct RemoteScanner {
    pub server_name: String,
    pub ssh_host: String,
    pub scan_root: String,
}

#[derive(Debug)]
pub struct RemoteFileEntry {
    pub path: String,
    pub size: u64,
    pub mtime_secs: i64,
}

impl RemoteScanner {
    pub fn new(server_name: String, ssh_host: String, scan_root: String) -> Self {
        Self { server_name, ssh_host, scan_root }
    }

    /// Run `find` on the remote server, return list of (path, size, mtime).
    /// Uses `find <root> -type f -printf '%p\\t%s\\t%T@\\n'` which outputs
    /// tab-separated: full_path \t size_bytes \t mtime_unix_epoch.
    pub fn discover_files(&self) -> Result<Vec<RemoteFileEntry>, String> {
        let find_cmd = format!(
            "find {} -type f -printf '%p\\t%s\\t%T@\\n' 2>/dev/null",
            shell_escape(&self.scan_root)
        );
        let output = ssh_exec(&self.ssh_host, &find_cmd)?;
        let mut files = Vec::new();
        for line in output.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() != 3 {
                continue;
            }
            let size: u64 = parts[1].parse().unwrap_or(0);
            let mtime_float: f64 = parts[2].parse().unwrap_or(0.0);
            let mtime_secs = mtime_float as i64;
            files.push(RemoteFileEntry {
                path: parts[0].to_string(),
                size,
                mtime_secs,
            });
        }
        Ok(files)
    }

    /// Fetch magic bytes (first 512 bytes) from a remote file via SSH.
    pub fn fetch_magic_bytes(&self, remote_path: &str) -> Vec<u8> {
        let cmd = format!(
            "head -c 512 {} | base64 2>/dev/null",
            shell_escape(remote_path)
        );
        match ssh_exec(&self.ssh_host, &cmd) {
            Ok(output) => {
                use std::io::Read;
                // output is base64-encoded, decode it
                let trimmed = output.trim();
                if trimmed.is_empty() {
                    return Vec::new();
                }
                // base64 might have line wraps from head, strip them
                let single_line = trimmed.replace('\n', "").replace('\r', "");
                base64_decode(&single_line).unwrap_or_default()
            }
            Err(_) => Vec::new(),
        }
    }

    /// Scan: discover + optionally fetch magic bytes, yield RawFileInfo.
    pub fn scan(&self, fetch_magic: bool) -> Result<Vec<RawFileInfo>, String> {
        let entries = self.discover_files()?;
        let mut results = Vec::with_capacity(entries.len());
        for entry in &entries {
            let magic_bytes = if fetch_magic {
                self.fetch_magic_bytes(&entry.path)
            } else {
                Vec::new()
            };
            let mime = mime_guess::from_path(&entry.path)
                .first_or_octet_stream()
                .to_string();
            results.push(RawFileInfo {
                path: std::path::PathBuf::from(&entry.path),
                source_server: self.server_name.clone(),
                size: entry.size,
                mtime_secs: entry.mtime_secs,
                hash_sha256: None,
                magic_bytes,
                mime_type: mime,
            });
        }
        Ok(results)
    }
}

/// Execute a command via SSH, returning stdout on success.
fn ssh_exec(host: &str, cmd: &str) -> Result<String, String> {
    let output = std::process::Command::new("ssh")
        .args(["-o", "ConnectTimeout=10", "-o", "BatchMode=yes", host, cmd])
        .output()
        .map_err(|e| format!("ssh failed to start: {}", e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ssh exited with {}: {}", output.status, stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Basic shell escaping for single-quoted strings in find/head args.
fn shell_escape(s: &str) -> String {
    // Replace single quotes with '\'' and wrap in single quotes
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Decode base64 without external deps — uses the `base64` crate (already in deps).
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(input)
        .map_err(|e| format!("base64 decode error: {}", e))
}
```

- [ ] **Step 2: Build check**

```bash
source "$HOME/.cargo/env" && cargo check 2>&1
```

Expected: scanner.rs compiles successfully (base64 is already a dependency of the project — check Cargo.lock). If `base64` isn't a direct dependency of `fan-core`, add it:

```bash
# Check if base64 is in fan-core's Cargo.toml
grep -r "base64" crates/fan-core/Cargo.toml
# If not found, we use a different approach — the project already has base64 0.13, 0.21, 0.22 in the lock file
# Use the simplest: bundle a minimal base64 decoder inline instead
```

**Fallback if `base64` crate isn't available in fan-core deps:** Replace the `base64_decode` function with a minimal built-in implementation:

```rust
fn base64_decode(input: &str) -> Result<Vec<u8>, String> {
    // Minimal base64 decode using only std
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut buf = Vec::with_capacity(input.len() * 3 / 4);
    let mut accum: u32 = 0;
    let mut bits: u32 = 0;
    for b in input.bytes() {
        if b == b'=' { break; }
        if b == b'\n' || b == b'\r' || b == b' ' { continue; }
        let idx = CHARS.iter().position(|&c| c == b).ok_or("invalid base64 char")?;
        accum = (accum << 6) | idx as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            buf.push((accum >> bits) as u8);
        }
    }
    Ok(buf)
}
```

- [ ] **Step 3: Commit**

```bash
git add crates/fan-core/src/scanner.rs
git commit -m "feat: add RemoteScanner for SSH-based file discovery on remote servers

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Task 6: Add `servers` CLI subcommand

**Files:**
- Create: `crates/fan-files/src/commands/servers.rs`
- Modify: `crates/fan-files/src/commands/mod.rs`
- Modify: `crates/fan-files/src/main.rs`

**Purpose:** New `fan-files servers list|add|remove|scan` commands for managing the server registry.

- [ ] **Step 1: Create the servers command module**

Create `crates/fan-files/src/commands/servers.rs`:

```rust
use fan_core::config::{Config, ServerConfig};
use std::io::{self, Write};

pub fn list(config: &Config) {
    let servers = config.enabled_servers();
    if servers.is_empty() {
        println!("No servers configured.");
        println!("Use 'fan-files servers add <name>' to add one.");
        return;
    }
    println!("{:<15} {:<20} {:<40} {}", "Server", "Host", "Scan Root", "Label");
    println!("{}", "-".repeat(90));
    for (name, cfg) in &servers {
        let host = if cfg.host.is_empty() { "localhost" } else { &cfg.host };
        let label = cfg.label.as_deref().unwrap_or("-");
        println!("{:<15} {:<20} {:<40} {}", name, host, cfg.scan_root, label);
    }

    // Also show disabled servers
    let disabled: Vec<_> = config.servers.servers.iter()
        .filter(|(_, cfg)| !cfg.enabled)
        .collect();
    if !disabled.is_empty() {
        println!();
        println!("Disabled servers:");
        for (name, cfg) in &disabled {
            let host = if cfg.host.is_empty() { "localhost" } else { &cfg.host };
            println!("  {} (host={}, root={})", name, host, cfg.scan_root);
        }
    }
}

pub fn add(name: &str) {
    println!("Adding server: {}", name);
    println!();

    let host = ask(&format!("SSH Host (from ~/.ssh/config, empty for local) [{}]: ", name));
    let host = if host.is_empty() { name.to_string() } else { host };

    let default_root = "/";
    let root_prompt = format!("Scan root directory [{}]: ", default_root);
    let scan_root = ask_with_default(&root_prompt, default_root);

    let label = ask("Label (optional): ");

    // Test SSH connection if remote
    if !host.is_empty() {
        print!("Testing SSH connection to {}... ", host);
        io::stdout().flush().ok();
        let result = std::process::Command::new("ssh")
            .args(["-o", "ConnectTimeout=5", "-o", "BatchMode=yes", &host, "echo ok"])
            .output();
        match result {
            Ok(output) if output.status.success() => println!("✅ OK"),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                println!("⚠️  Failed: {}", stderr.trim());
            }
            Err(e) => println!("⚠️  Error: {}", e),
        }

        // Quick file count
        print!("Counting files remotely (find)... ");
        io::stdout().flush().ok();
        let find_cmd = format!("find '{}' -type f 2>/dev/null | wc -l", scan_root.replace('\'', "'\\''"));
        match std::process::Command::new("ssh")
            .args(["-o", "ConnectTimeout=10", "-o", "BatchMode=yes", &host, &find_cmd])
            .output()
        {
            Ok(output) if output.status.success() => {
                let count: String = String::from_utf8_lossy(&output.stdout).trim().to_string();
                println!("{} files found", count);
            }
            _ => println!("could not count (will retry during scan)"),
        }
    }

    // Load existing config, add server, save
    let mut config = Config::load().expect("Failed to load config");
    config.servers.servers.insert(name.to_string(), ServerConfig {
        host,
        scan_root,
        label: if label.is_empty() { None } else { Some(label) },
        enabled: true,
    });

    let config_path = fan_core::config::dirs_fan().join("config.toml");
    std::fs::create_dir_all(fan_core::config::dirs_fan()).ok();
    if let Ok(toml_str) = toml::to_string_pretty(&config) {
        std::fs::write(&config_path, toml_str).ok();
        println!("✅ Server '{}' added to config.", name);
    } else {
        eprintln!("Failed to serialize config.");
    }
}

pub fn remove(name: &str) {
    let mut config = Config::load().expect("Failed to load config");
    if config.servers.servers.remove(name).is_some() {
        let config_path = fan_core::config::dirs_fan().join("config.toml");
        if let Ok(toml_str) = toml::to_string_pretty(&config) {
            std::fs::write(&config_path, toml_str).ok();
            println!("✅ Server '{}' removed.", name);
        }
    } else {
        eprintln!("Server '{}' not found in config.", name);
    }
}

pub fn scan_one(name: &str) {
    let config = Config::load().expect("Failed to load config");
    let server_cfg = match config.servers.servers.get(name) {
        Some(c) if c.enabled => c.clone(),
        Some(_) => {
            eprintln!("Server '{}' is disabled. Enable it first.", name);
            return;
        }
        None => {
            eprintln!("Server '{}' not found. Use 'fan-files servers list'.", name);
            return;
        }
    };

    if server_cfg.host.is_empty() {
        // Local scan
        println!("Scanning local server '{}' in {}...", name, server_cfg.scan_root);
        let scanner = fan_core::scanner::Scanner::new(
            vec![server_cfg.scan_root.clone()],
            vec!["/tmp".into(), "*.tmp".into()],
            name.to_string(),
        );
        let index = fan_core::index::open_index(&config, fan_core::index::IndexMode::ReadWrite)
            .expect("Failed to open index");
        let mut count = 0u64;
        for file_info in scanner.scan() {
            match index.index_file(&file_info, None) {
                Ok(_) => count += 1,
                Err(e) => eprintln!("Failed to index {}: {}", file_info.path.display(), e),
            }
        }
        index.tantivy.commit().ok();
        println!("✅ Scanned {}: {} files indexed", name, count);
    } else {
        // Remote scan
        println!("Scanning remote server '{}' in {}...", name, server_cfg.scan_root);
        let remote = fan_core::scanner::RemoteScanner::new(
            name.to_string(),
            server_cfg.host.clone(),
            server_cfg.scan_root.clone(),
        );
        match remote.scan(true) {
            Ok(entries) => {
                let index = fan_core::index::open_index(&config, fan_core::index::IndexMode::ReadWrite)
                    .expect("Failed to open index");
                let mut count = 0u64;
                for file_info in &entries {
                    match index.index_file(file_info, None) {
                        Ok(_) => count += 1,
                        Err(e) => eprintln!("Failed to index {}: {}", file_info.path.display(), e),
                    }
                }
                index.tantivy.commit().ok();
                println!("✅ Scanned {}: {} files indexed", name, count);
            }
            Err(e) => eprintln!("Remote scan failed: {}", e),
        }
    }
}

fn ask(prompt: &str) -> String {
    print!("{}", prompt);
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    input.trim().to_string()
}

fn ask_with_default(prompt: &str, default: &str) -> String {
    let input = ask(prompt);
    if input.is_empty() { default.to_string() } else { input }
}
```

- [ ] **Step 2: Register module in commands/mod.rs**

Add after line 13 (`pub mod update;`):

```rust
pub mod servers;
```

- [ ] **Step 3: Add CLI variant in main.rs**

In `crates/fan-files/src/main.rs`, add to the `Commands` enum (after `Init`):

```rust
    /// Manage registered servers
    #[command(subcommand)]
    Servers(ServersAction),
```

Add the `ServersAction` enum after `ProjectAction`:

```rust
#[derive(Subcommand)]
enum ServersAction {
    /// List all registered servers
    List,
    /// Add a new server (interactive)
    Add {
        name: String,
    },
    /// Remove a server
    Remove {
        name: String,
    },
    /// Scan a single server
    Scan {
        name: String,
    },
}
```

Add the match arm in `main()` (after `Commands::Init`):

```rust
        Commands::Servers(action) => match action {
            ServersAction::List => commands::servers::list(&config),
            ServersAction::Add { name } => commands::servers::add(&name),
            ServersAction::Remove { name } => commands::servers::remove(&name),
            ServersAction::Scan { name } => commands::servers::scan_one(&name),
        },
```

- [ ] **Step 4: Build check**

```bash
source "$HOME/.cargo/env" && cargo check 2>&1
```

- [ ] **Step 5: Commit**

```bash
git add crates/fan-files/src/commands/servers.rs crates/fan-files/src/commands/mod.rs crates/fan-files/src/main.rs
git commit -m "feat: add 'fan-files servers list|add|remove|scan' CLI commands

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Task 7: Update `status` command with per-server breakdown

**Files:**
- Modify: `crates/fan-files/src/commands/status.rs`

**Purpose:** Show per-server file counts in `fan-files status`.

- [ ] **Step 1: Replace status.rs with per-server-aware version**

Replace the entire content of `crates/fan-files/src/commands/status.rs`:

```rust
use fan_core::config::Config;

pub fn run(config: &Config) {
    let index = match fan_core::index::open_index(config, fan_core::index::IndexMode::ReadOnly) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Failed to open index: {}", e);
            return;
        }
    };

    match index.sqlite.status() {
        Ok(status) => {
            println!("fan-files Index Status");
            println!("====================");
            println!("Indexed files:   {}", status.indexed_files);
            println!("Total tracked:   {}", status.total_files);
            println!("Deleted (soft):  {}", status.deleted_files);

            // Show metadata coverage
            let with_meta = index.sqlite.count_with_bio_metadata().unwrap_or(0);
            let pct = if status.indexed_files > 0 {
                (with_meta as f64 / status.indexed_files as f64) * 100.0
            } else {
                0.0
            };
            println!("Metadata coverage: {:.0}% ({}/{})", pct, with_meta, status.indexed_files);
            if pct < 50.0 && status.indexed_files > 10 {
                println!("  ⚠ Metadata coverage is low. Run 'fan-files infer' for better results.");
            }

            let fmt_ts = |ts: i64| -> String {
                std::time::UNIX_EPOCH
                    .checked_add(std::time::Duration::from_secs(ts as u64))
                    .map(|t| format!("{:?}", t))
                    .unwrap_or_else(|| ts.to_string())
            };

            // Per-server breakdown
            match index.sqlite.status_by_server() {
                Ok(servers) => {
                    if !servers.is_empty() {
                        println!();
                        println!("Servers:");
                        let max_name = servers.iter().map(|s| s.server.len()).max().unwrap_or(6);
                        for s in &servers {
                            let last = s.last_scan.map(fmt_ts).unwrap_or_else(|| "never".to_string());
                            println!(
                                "  {:<width$} {:>8} files  (last scan: {})",
                                s.server,
                                s.file_count,
                                last,
                                width = max_name + 2,
                            );
                        }
                    }
                }
                Err(e) => {
                    // Silently skip per-server stats on older DBs
                    let _ = e;
                }
            }

            if let Some(ts) = status.last_full_scan {
                println!("Last scan:       {}", fmt_ts(ts));
            }
            if let Some(ts) = status.last_change {
                println!("Last change:     {}", fmt_ts(ts));
            }
        }
        Err(e) => eprintln!("Error querying status: {}", e),
    }
}
```

- [ ] **Step 2: Build check**

```bash
source "$HOME/.cargo/env" && cargo check 2>&1
```

- [ ] **Step 3: Commit**

```bash
git add crates/fan-files/src/commands/status.rs
git commit -m "feat: add per-server file counts to 'fan-files status' output

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Task 8: Update `search` output with `<server>:<path>`

**Files:**
- Modify: `crates/fan-files/src/commands/search.rs`

**Purpose:** Search results display paths as `<server>:<path>`.

- [ ] **Step 1: Update search display format**

In `crates/fan-files/src/commands/search.rs`, in the non-JSON branch (lines 127-136), replace the `println!` format:

From:
```rust
            println!(
                "{:.3}  {}  {:?}  {:?}  {}",
                r.score, r.path, r.assay_type, r.species, r.summary
            );
```

To:
```rust
            // Build display path: <server>:<path>
            let display_path = if entry.source_server != "local" {
                format!("{}:{}", entry.source_server, r.path)
            } else {
                r.path.to_string()
            };
            println!(
                "{:.3}  {}  {:?}  {:?}  {}",
                r.score, display_path, r.assay_type, r.species, r.summary
            );
```

But wait — `entry` is not in scope at line 127. The `entry` is inside the loop above (line 70-111). The `SearchResult` already has `path` set, but `source_server` isn't on `SearchResult`. We need to handle this differently.

**Better approach:** Fetch the `source_server` inside the display loop. In the non-JSON loop (lines 127-137), update to:

```rust
    if json {
        println!("{}", serde_json::to_string_pretty(&results).unwrap());
    } else {
        for r in &results {
            // Look up source_server for display path
            let display_path = lookup_server_prefix(&index.sqlite, &r.path);
            println!(
                "{:.3}  {}  {:?}  {:?}  {}",
                r.score, display_path, r.assay_type, r.species, r.summary
            );
        }
        if results.is_empty() {
            println!("No results found for: {}", query);
        }
    }
}

/// Build display path: prepend "<server>:" if path's source_server is not "local".
fn lookup_server_prefix(sqlite: &fan_core::index::sqlite::SqliteStore, path: &str) -> String {
    let conn = sqlite.conn.lock().unwrap();
    let server: Option<String> = conn
        .query_row(
            "SELECT source_server FROM files WHERE path=?1 LIMIT 1",
            rusqlite::params![path],
            |row| row.get(0),
        )
        .ok();
    match server {
        Some(ref s) if s != "local" => format!("{}:{}", s, path),
        _ => path.to_string(),
    }
}
```

Also update the JSON output to include `source_server`. In the JSON branch, iterate results AND lookup server:

For JSON, we use a simpler approach: add the server prefix directly to the path field in JSON. Update the JSON output to include source_server:

In the JSON output section, replace:
```rust
    if json {
        println!("{}", serde_json::to_string_pretty(&results).unwrap());
```

With:
```rust
    if json {
        // Enrich with source_server
        let enriched: Vec<serde_json::Value> = results.iter().map(|r| {
            let server = lookup_server_prefix(&index.sqlite, &r.path);
            serde_json::json!({
                "path": server,
                "score": r.score,
                "file_type": r.file_type,
                "assay_type": r.assay_type,
                "species": r.species,
                "tags": r.tags,
                "summary": r.summary,
            })
        }).collect();
        println!("{}", serde_json::to_string_pretty(&enriched).unwrap());
```

- [ ] **Step 2: Build check**

```bash
source "$HOME/.cargo/env" && cargo check 2>&1
```

- [ ] **Step 3: Commit**

```bash
git add crates/fan-files/src/commands/search.rs
git commit -m "feat: display <server>:<path> in search results

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Task 9: Update `info` command with source_server

**Files:**
- Modify: `crates/fan-files/src/commands/info.rs`

**Purpose:** Show `Source Server:` line in `fan-files info`.

- [ ] **Step 1: Add source_server to info output**

In `crates/fan-files/src/commands/info.rs`, after the `Path:` line (line 39), add a source_server line. Also update the JSON output.

Non-JSON output — add after `println!("Path:       {}", entry.path.display());`:

```rust
                if entry.source_server != "local" {
                    // Try to get label from config
                    let label = config.servers.servers
                        .get(&entry.source_server)
                        .and_then(|c| c.label.as_ref())
                        .map(|l| format!(" ({})", l))
                        .unwrap_or_default();
                    println!("Source:      {}{}", entry.source_server, label);
                }
```

JSON output — add `"source_server"` to the json! macro:

```rust
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "path": entry.path.to_string_lossy(),
                        "source_server": entry.source_server,
                        ...
```

Full replacement for the `Ok(Some(entry))` branch (lines 14-70): replace the non-JSON Path line with the version that includes Source:

```rust
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "path": entry.path.to_string_lossy(),
                        "source_server": entry.source_server,
                        "size": entry.size,
                        "size_mb": format!("{:.2}", entry.size as f64 / 1_048_576.0),
                        "mtime": ts_to_str(entry.mtime_secs),
                        "mime": entry.mime_type,
                        "format": entry.format_info,
                        "bio_metadata": entry.bio_metadata,
                        "indexed_at": ts_to_str(entry.indexed_at),
                        "updated_at": ts_to_str(entry.updated_at),
                    }))
                    .unwrap()
                );
            } else {
                println!("Path:       {}", entry.path.display());
                if entry.source_server != "local" {
                    let label = config.servers.servers
                        .get(&entry.source_server)
                        .and_then(|c| c.label.as_ref())
                        .map(|l| format!(" ({})", l))
                        .unwrap_or_default();
                    println!("Source:     {}{}", entry.source_server, label);
                }
                println!(
                    "Size:       {:.2} MB ({} bytes)",
                    entry.size as f64 / 1_048_576.0,
                    entry.size
                );
                // ... rest unchanged
```

- [ ] **Step 2: Build check**

```bash
source "$HOME/.cargo/env" && cargo check 2>&1
```

- [ ] **Step 3: Commit**

```bash
git add crates/fan-files/src/commands/info.rs
git commit -m "feat: show source_server in 'fan-files info' output

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Task 10: Add `--server` filter to `list` command

**Files:**
- Modify: `crates/fan-files/src/commands/list.rs`
- Modify: `crates/fan-files/src/main.rs` (add `--server` arg to `List` variant)

**Purpose:** Filter file listing by source_server.

- [ ] **Step 1: Add `server` parameter to `list::run`**

In `crates/fan-files/src/commands/list.rs`, change the function signature:

```rust
pub fn run(config: &Config, category: Option<&str>, tag: Option<&str>, server: Option<&str>, json: bool) {
```

In the body, after the `let entries =` block (after line 28), add server filtering:

```rust
    // Apply server filter if specified
    let entries: Vec<_> = entries.into_iter()
        .filter(|e| {
            server.as_ref().map_or(true, |s| e.source_server == *s)
        })
        .collect();
```

- [ ] **Step 2: Add `--server` argument to main.rs**

In `crates/fan-files/src/main.rs`, update the `List` variant:

```rust
    List {
        #[arg(long)]
        category: Option<String>,
        #[arg(long)]
        tag: Option<String>,
        #[arg(long)]
        server: Option<String>,
        #[arg(long)]
        json: bool,
    },
```

Update the `Commands::List` match arm:

```rust
        Commands::List { category, tag, server, json } => {
            commands::list::run(&config, category.as_deref(), tag.as_deref(), server.as_deref(), json)
        }
```

- [ ] **Step 3: Build check**

```bash
source "$HOME/.cargo/env" && cargo check 2>&1
```

- [ ] **Step 4: Commit**

```bash
git add crates/fan-files/src/commands/list.rs crates/fan-files/src/main.rs
git commit -m "feat: add --server filter to 'fan-files list' command

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Task 11: Add server column to `projects` output

**Files:**
- Modify: `crates/fan-files/src/commands/projects.rs`

**Purpose:** Display server in project listing and detail view.

- [ ] **Step 1: Update `list` function to show server column**

In `crates/fan-files/src/commands/projects.rs`, update the `list` function (lines 23-46). Replace the `println!` in the loop with a version that includes the project's `source_server`:

The `Project` struct in `project.rs` doesn't have `source_server` yet. We need to query it from the DB. Let's use a simpler approach: query it from the project table via SQL.

In `fan-core/src/project.rs`, the `map_row` currently reads 10 columns but the project table has `source_server` at position after `updated_at`. We need to update `Project` and `ProjectStore` to include `source_server`.

Actually, let's keep this task focused. The `source_server` was added to the `project` table in Task 3's migration. But the `Project` struct and `ProjectStore` haven't been updated. Let's do it inline here.

**In `crates/fan-core/src/project.rs`:**
- Add `pub source_server: Option<String>,` to `Project` struct (after `summary`)
- Update `map_row` to read column 10: `source_server: row.get(10)?,`
- Update `insert` to include `source_server`: add `source_server` parameter
- Update SELECT queries to include `source_server`

But this is getting larger than expected. For the plan to be clean, let me make this its own subtask. Actually, looking at the spec, the projects display is a minor feature. Let me simplify: just do a raw SQL query in the projects list command to get the source_server for each project.

**Simpler approach for projects.rs `list`:** Use a direct SQL query that joins project with project_file and files to get the predominant server for each project.

Actually, the simplest: just update the `ProjectStore::map_row` and SELECTs to include the new column. Since the migration in Task 3 already added the column, we need to:

1. Add `source_server` to the `Project` struct
2. Update `map_row` 
3. Update all SELECT columns

Let me do this now:

**In `crates/fan-core/src/project.rs`:**

Add to `Project` struct after `summary`:
```rust
    pub source_server: Option<String>,
```

Update `map_row` (line 104-117) to:
```rust
    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<Project> {
        Ok(Project {
            id: row.get(0)?,
            name: row.get(1)?,
            assay_type: row.get(2)?,
            species: row.get(3)?,
            species_confidence: row.get(4)?,
            species_source: row.get(5)?,
            root_dirs: row.get(6)?,
            summary: row.get(7)?,
            source_server: row.get(8)?,
            created_at: row.get(9)?,
            updated_at: row.get(10)?,
        })
    }
```

Update all SELECT queries to include `source_server` (the 11th column). In:
- `get_by_name` (line 77-78): add `source_server` between `summary` and `created_at`
- `all` (line 87-88): same
- `get` (line 97-98): same

All three SELECTs become:
```sql
SELECT id, name, assay_type, species, species_confidence, species_source, root_dirs, summary, source_server, created_at, updated_at
```

**In `crates/fan-files/src/commands/projects.rs`:**

In the `list` function, update the `println!` format to include server:

```rust
                println!(
                    "{:<25} {:<22} {:<18} {:<10} {} files",
                    truncate(&p.name, 25),
                    assay,
                    format!("{} ({})", species, conf),
                    p.source_server.as_deref().unwrap_or("?"),
                    file_count,
                );
```

Also add header line before the loop:
```rust
                println!("{:<25} {:<22} {:<18} {:<10} {}", "Project", "Assay", "Species", "Server", "Files");
```

In the `show` function, add a server line after `Project: {}`:
```rust
            if let Some(ref srv) = p.source_server {
                println!("  Server:      {}", srv);
            }
```

- [ ] **Step 2: Build check**

```bash
source "$HOME/.cargo/env" && cargo check 2>&1
```

- [ ] **Step 3: Commit**

```bash
git add crates/fan-core/src/project.rs crates/fan-files/src/commands/projects.rs
git commit -m "feat: add source_server to Project struct and projects command display

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Task 12: Wire remote scanning into daemon

**Files:**
- Modify: `crates/fan-files/src/commands/daemon.rs`

**Purpose:** The daemon loop runs per-server scans: local via `Scanner`, remote via `RemoteScanner`.

- [ ] **Step 1: Update daemon to use per-server scanning**

The daemon needs significant changes. The key principles:
1. Read `config.enabled_servers()` instead of `config.scan.include`
2. For each server: create `Scanner` (local) or `RemoteScanner` (remote)
3. Run initial full scan for each server
4. Remote servers use periodic `find`-based incremental update instead of real-time watcher

Replace the daemon::run function. Here's the complete new version:

```rust
use fan_core::config::Config;
use fan_core::detector::BuiltinDetector;
use fan_core::infer;
use fan_core::index::IndexEngine;
use fan_core::interpreter::InterpreterRegistry;
use fan_core::llm::LlmClient;
use fan_core::plugin::registry::PluginRegistry;
use fan_core::project::ProjectStore;
use fan_core::scanner::{RemoteScanner, Scanner};
use fan_core::watcher::FileWatcher;
use fan_plugin_sdk::{FileContext, FormatInfo};
use std::path::Path;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tracing::{error, info, warn};

pub fn run(config: &Config) {
    info!("Starting fan-files daemon...");

    let index = match fan_core::index::open_index(config, fan_core::index::IndexMode::ReadWrite) {
        Ok(i) => i,
        Err(e) => {
            error!("Failed to open index engine: {}", e);
            return;
        }
    };
    let mut plugins = PluginRegistry::new(config.plugins.dir.clone());
    let n = plugins.discover().unwrap_or(0);
    info!("Discovered {} plugins", n);

    let interpreter_registry = InterpreterRegistry::new();

    let servers = config.enabled_servers();
    let has_local = servers.iter().any(|(_, cfg)| cfg.host.is_empty());
    let has_remote = servers.iter().any(|(_, cfg)| !cfg.host.is_empty());
    let local_server_names: Vec<String> = servers
        .iter()
        .filter(|(_, cfg)| cfg.host.is_empty())
        .map(|(n, _)| n.clone())
        .collect();

    // Initial full scan: local + remote
    for (name, cfg) in &servers {
        if cfg.host.is_empty() {
            // Local scan
            let scanner = Scanner::new(
                vec![cfg.scan_root.clone()],
                config.scan.exclude.clone(),
                name.clone(),
            );
            info!("Starting local scan: {} ({})", name, cfg.scan_root);
            run_full_scan(&index, &scanner, &plugins, &interpreter_registry);
        } else {
            // Remote scan (fast mode: no magic bytes for initial)
            let remote = RemoteScanner::new(
                name.clone(),
                cfg.host.clone(),
                cfg.scan_root.clone(),
            );
            info!("Starting remote scan: {} ({})", name, cfg.scan_root);
            match remote.scan(false) {
                Ok(entries) => {
                    let start = Instant::now();
                    let mut count = 0u64;
                    for file_info in &entries {
                        let path_str = file_info.path.to_string_lossy();
                        let format_info = plugins
                            .detect_format(&path_str, &file_info.magic_bytes)
                            .or_else(|| BuiltinDetector::detect(&path_str, &file_info.magic_bytes));
                        match index.index_file(file_info, format_info.as_ref()) {
                            Ok(file_id) => {
                                count += 1;
                                // Remote embeddings are expensive, skip for now
                            }
                            Err(e) => error!("Failed to index {}: {}", file_info.path.display(), e),
                        }
                    }
                    index.tantivy.commit().ok();
                    info!(
                        "Remote scan complete: {} ({}) — {} files in {:.1}s",
                        name, cfg.scan_root, count, start.elapsed().as_secs_f64()
                    );
                }
                Err(e) => error!("Remote scan failed for {}: {}", name, e),
            }
        }
    }

    // After initial scan, run LLM inference if configured
    {
        let llm_client = LlmClient::new(config.llm.clone());
        if llm_client.is_configured() {
            let project_store = ProjectStore::new(Arc::clone(&index.sqlite.conn));
            let scan_root = config
                .enabled_servers()
                .first()
                .map(|(_, c)| c.scan_root.as_str())
                .unwrap_or("/");
            info!("Running LLM inference on indexed files...");
            match infer::run_inference(&index.sqlite, &project_store, &llm_client, scan_root) {
                Ok((p, r)) => info!("LLM inference complete: {} projects, {} relations", p, r),
                Err(e) => warn!("LLM inference failed: {}", e),
            }
        }
    }

    // File watcher: only for local servers
    let watcher = if local_server_names.is_empty() {
        warn!("No local servers — file watcher disabled");
        return; // Pure remote mode: one-shot scan, no continuous loop
    } else {
        let watch_dirs: Vec<String> = servers
            .iter()
            .filter(|(_, cfg)| cfg.host.is_empty())
            .map(|(_, cfg)| cfg.scan_root.clone())
            .collect();
        match FileWatcher::new(&watch_dirs) {
            Ok(w) => w,
            Err(e) => {
                error!("Failed to start file watcher: {}", e);
                return;
            }
        }
    };
    info!("File watcher started for local servers");

    let sync_time = parse_sync_time(&config.schedule.full_sync);
    let retention_days = config.retention.deleted_keep_days;
    let mut last_sync_day: Option<u64> = None;
    let mut last_purge_day: Option<u64> = None;
    let mut new_files_since_infer: u64 = 0;

    loop {
        let now_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let current_day = now_secs / 86400;
        let current_hour_min = hour_minute_from_secs(now_secs);

        // Scheduled full sync
        if last_sync_day != Some(current_day)
            && current_hour_min.0 == sync_time.0
            && current_hour_min.1 >= sync_time.1
            && current_hour_min.1 < sync_time.1 + 10
        {
            info!("Running scheduled full sync...");
            // Rescan local
            for name in &local_server_names {
                if let Some((_, cfg)) = servers.iter().find(|(n, _)| n == name) {
                    let scanner = Scanner::new(
                        vec![cfg.scan_root.clone()],
                        config.scan.exclude.clone(),
                        name.clone(),
                    );
                    run_full_scan(&index, &scanner, &plugins, &interpreter_registry);
                }
            }
            // Rescan remote (fast mode)
            for (name, cfg) in &servers {
                if !cfg.host.is_empty() {
                    let remote = RemoteScanner::new(
                        name.clone(),
                        cfg.host.clone(),
                        cfg.scan_root.clone(),
                    );
                    match remote.scan(false) {
                        Ok(entries) => {
                            let mut count = 0u64;
                            for file_info in &entries {
                                let path_str = file_info.path.to_string_lossy();
                                let format_info = plugins
                                    .detect_format(&path_str, &file_info.magic_bytes)
                                    .or_else(|| BuiltinDetector::detect(&path_str, &file_info.magic_bytes));
                                if let Ok(file_id) = index.index_file(file_info, format_info.as_ref()) {
                                    count += 1;
                                }
                            }
                            index.tantivy.commit().ok();
                            info!("Scheduled remote scan: {} — {} files", name, count);
                        }
                        Err(e) => warn!("Scheduled remote scan failed for {}: {}", name, e),
                    }
                }
            }

            // After scheduled scan, run LLM inference
            {
                let llm_client = LlmClient::new(config.llm.clone());
                if llm_client.is_configured() {
                    let project_store = ProjectStore::new(Arc::clone(&index.sqlite.conn));
                    let scan_root = servers.first().map(|(_, c)| c.scan_root.as_str()).unwrap_or("/");
                    match infer::run_inference(&index.sqlite, &project_store, &llm_client, scan_root) {
                        Ok((p, r)) => info!("LLM inference: {} projects, {} relations", p, r),
                        Err(e) => warn!("LLM inference failed: {}", e),
                    }
                }
            }
            new_files_since_infer = 0;
            last_sync_day = Some(current_day);
        }

        // Daily purge
        if last_purge_day != Some(current_day) {
            match index.sqlite.purge_old_deleted(retention_days) {
                Ok(n) if n > 0 => info!("Purged {} old deleted entries", n),
                Ok(_) => {}
                Err(e) => error!("Failed to purge: {}", e),
            }
            last_purge_day = Some(current_day);
        }

        // Watch local file changes
        match watcher.events().recv_timeout(Duration::from_secs(10)) {
            Ok(paths) => {
                for path in &paths {
                    if path.exists() {
                        if let Some(ref local_name) = local_server_names.first() {
                            let file_info = {
                                let meta = std::fs::metadata(path).ok();
                                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
                                let mtime = meta
                                    .and_then(|m| m.modified().ok())
                                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                                    .map(|d| d.as_secs() as i64)
                                    .unwrap_or(0);
                                let magic = read_magic(path);
                                let mime = mime_guess::from_path(path)
                                    .first_or_octet_stream()
                                    .to_string();
                                fan_core::types::RawFileInfo {
                                    path: path.to_path_buf(),
                                    source_server: local_name.clone(),
                                    size,
                                    mtime_secs: mtime,
                                    hash_sha256: None,
                                    magic_bytes: magic,
                                    mime_type: mime,
                                }
                            };
                            let path_str = file_info.path.to_string_lossy();
                            let format_info = plugins
                                .detect_format(&path_str, &file_info.magic_bytes)
                                .or_else(|| BuiltinDetector::detect(&path_str, &file_info.magic_bytes));
                            match index.index_file(&file_info, format_info.as_ref()) {
                                Ok(file_id) => {
                                    info!("Re-indexed: {}", path.display());
                                    new_files_since_infer += 1;
                                    if new_files_since_infer >= 10 {
                                        auto_infer(&index, config);
                                        new_files_since_infer = 0;
                                    }
                                }
                                Err(e) => error!("Failed to re-index {}: {}", path.display(), e),
                            }
                        }
                    } else {
                        if let Err(e) = index.sqlite.mark_deleted(path) {
                            error!("Failed to mark deleted {}: {}", path.display(), e);
                        } else {
                            info!("Marked deleted: {}", path.display());
                        }
                    }
                }
                index.tantivy.commit().ok();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                error!("Watcher channel disconnected");
                break;
            }
        }
    }

    info!("Daemon shutting down");
}

fn auto_infer(index: &IndexEngine, config: &Config) {
    let llm_client = LlmClient::new(config.llm.clone());
    if llm_client.is_configured() {
        let project_store = ProjectStore::new(Arc::clone(&index.sqlite.conn));
        let scan_root = config
            .enabled_servers()
            .first()
            .map(|(_, c)| c.scan_root.as_str())
            .unwrap_or("/");
        match infer::run_inference(&index.sqlite, &project_store, &llm_client, scan_root) {
            Ok((p, r)) => info!("Auto-infer: {} projects, {} relations", p, r),
            Err(e) => warn!("Auto-infer failed: {}", e),
        }
    }
}

fn run_full_scan(
    index: &IndexEngine,
    scanner: &Scanner,
    plugins: &PluginRegistry,
    interpreter_registry: &InterpreterRegistry,
) {
    info!("Starting full scan...");
    let start = Instant::now();
    let mut count = 0u64;
    for file_info in scanner.scan() {
        let path_str = file_info.path.to_string_lossy();
        let format_info = plugins
            .detect_format(&path_str, &file_info.magic_bytes)
            .or_else(|| BuiltinDetector::detect(&path_str, &file_info.magic_bytes));
        match index.index_file(&file_info, format_info.as_ref()) {
            Ok(file_id) => {
                count += 1;
                // Run context interpretation
                run_interpretation(
                    index,
                    file_id,
                    file_info.path.as_ref(),
                    format_info.as_ref(),
                    interpreter_registry,
                );
                // Generate embedding
                run_embedding(index, file_id, file_info.path.as_ref());
            }
            Err(e) => error!("Failed to index {}: {}", file_info.path.display(), e),
        }
    }
    index.tantivy.commit().ok();
    info!(
        "Full scan complete: {} files indexed in {:.1}s",
        count,
        start.elapsed().as_secs_f64()
    );
}

fn read_magic(path: &Path) -> Vec<u8> {
    std::fs::File::open(path)
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

// ---- keep existing helper functions ----
// build_file_context, run_interpretation, run_embedding, build_embedding_text,
// parse_sync_time, hour_minute_from_secs
// (unchanged from current daemon.rs, lines 229-355)
```

**IMPORTANT:** Keep the existing `build_file_context`, `run_interpretation`, `run_embedding`, `build_embedding_text`, `parse_sync_time`, and `hour_minute_from_secs` functions unchanged from the current daemon.rs (lines 229-355).

- [ ] **Step 2: Build check**

```bash
source "$HOME/.cargo/env" && cargo check 2>&1
```

- [ ] **Step 3: Commit**

```bash
git add crates/fan-files/src/commands/daemon.rs
git commit -m "feat: per-server daemon scanning — local Scanner + remote RemoteScanner

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Task 13: Update `init` wizard with server step

**Files:**
- Modify: `crates/fan-files/src/commands/init.rs`

**Purpose:** init wizard now includes remote server configuration in step 2.

- [ ] **Step 1: Add server setup step to init**

Replace `crates/fan-files/src/commands/init.rs` entirely:

```rust
use fan_core::config::LLM_PROVIDERS;
use std::io::{self, Write};
use std::process::{Command, Stdio};
use fan_core::config::{Config, ServerConfig};

pub fn run(config: &Config) {
    println!();
    println!("  ╔══════════════════════════════════════╗");
    println!("  ║   Fan-Files 初始化配置向导          ║");
    println!("  ╚══════════════════════════════════════╝");
    println!();
    let mut new_config = config.clone();

    // Step 1: Local directories
    run_step_1(&mut new_config);

    // Step 2: Remote servers (NEW)
    run_step_servers(&mut new_config);

    // Step 3: LLM
    run_step_3(&mut new_config);  // was step_2

    // Step 4: Start
    run_step_4(&new_config);  // was step_3
    println!("  配置已保存。");
}

fn ask(prompt: &str) -> String {
    print!("{}", prompt);
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();
    input.trim().to_string()
}

fn run_step_1(config: &mut Config) {
    println!("  ▸ 步骤 1/4：本地扫描目录");
    println!();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let mut dirs: Vec<String> = if config.scan.include.is_empty() {
        vec![home.clone()]
    } else {
        config.scan.include.clone()
    };

    loop {
        println!("  当前扫描目录:");
        for (i, d) in dirs.iter().enumerate() {
            println!("    [{}] {}", i + 1, d);
        }
        println!();
        println!("  [a] 添加目录  [d] 删除目录  [Enter] 完成");
        let input = ask("  请输入: ");

        match input.as_str() {
            "a" | "A" => {
                let path = ask("  请输入目录路径: ");
                if !path.is_empty() && !dirs.contains(&path) {
                    dirs.push(path);
                }
            }
            "d" | "D" => {
                let n = ask("  输入要删除的序号: ");
                if let Ok(idx) = n.parse::<usize>() {
                    if idx > 0 && idx <= dirs.len() {
                        dirs.remove(idx - 1);
                    }
                }
            }
            "" => break,
            _ => println!("  无效输入"),
        }
    }
    // Store local directories under [servers.local]
    if !dirs.is_empty() {
        config.servers.servers.entry("local".to_string()).or_insert(ServerConfig {
            host: String::new(),
            scan_root: dirs[0].clone(),
            label: Some("Mac mini 本地".to_string()),
            enabled: true,
        });
    }
    config.scan.include = dirs.clone();
    config.watch.include = dirs;
    println!();
}

fn run_step_servers(config: &mut Config) {
    println!("  ▸ 步骤 2/4：远程服务器（SSH）");
    println!();
    println!("  fan-files 可以通过 SSH 扫描远程服务器上的数据目录。");
    println!("  前提：~/.ssh/config 中已配置对应 Host。");
    println!();

    loop {
        let existing: Vec<String> = config.servers.servers
            .iter()
            .filter(|(n, _)| *n != "local")
            .map(|(n, _)| n.clone())
            .collect();

        if !existing.is_empty() {
            println!("  当前远程服务器:");
            for (i, name) in existing.iter().enumerate() {
                if let Some(cfg) = config.servers.servers.get(name) {
                    println!("    [{}] {} — ssh {} '{}'", i + 1, name, cfg.host, cfg.scan_root);
                }
            }
            println!();
        }

        println!("  [a] 添加服务器  [d] 删除服务器  [Enter] 继续");
        let input = ask("  请输入: ");

        match input.as_str() {
            "a" | "A" => {
                let name = ask("  服务器名称 (如 ai-srv): ");
                if name.is_empty() { continue; }
                let host = ask(&format!("  SSH Host [{}]: ", name));
                let host = if host.is_empty() { name.clone() } else { host };
                let root = ask("  扫描根目录 [/]: ");
                let root = if root.is_empty() { "/".to_string() } else { root };
                let label = ask("  描述 (可选): ");

                // Test SSH
                print!("  测试 SSH 连接... ");
                io::stdout().flush().ok();
                let result = Command::new("ssh")
                    .args(["-o", "ConnectTimeout=5", "-o", "BatchMode=yes", &host, "echo ok"])
                    .output();
                match result {
                    Ok(out) if out.status.success() => println!("✅"),
                    _ => println!("⚠️  连接失败（可稍后重试）"),
                }

                config.servers.servers.insert(name, ServerConfig {
                    host,
                    scan_root: root,
                    label: if label.is_empty() { None } else { Some(label) },
                    enabled: true,
                });
            }
            "d" | "D" => {
                if existing.is_empty() {
                    println!("  没有可删除的服务器");
                    continue;
                }
                let n = ask("  输入要删除的序号: ");
                if let Ok(idx) = n.parse::<usize>() {
                    if idx > 0 && idx <= existing.len() {
                        config.servers.servers.remove(&existing[idx - 1]);
                        println!("  已删除");
                    }
                }
            }
            "" => break,
            _ => println!("  无效输入"),
        }
    }
    println!();
}

fn run_step_3(config: &mut Config) {
    println!("  ▸ 步骤 3/4：LLM 元数据推断");
    // ... keep existing run_step_2 content (lines 76-111 of current init.rs), just rename
    println!();
    println!("  LLM 可自动识别项目、物种、实验类型。请选择:");
    for (i, p) in LLM_PROVIDERS.iter().enumerate() {
        println!("  [{}] {} — {}", i + 1, p.name, p.description);
    }
    println!("  [s] 暂时跳过");

    let input = ask("  请输入: ");

    if let Ok(idx) = input.parse::<usize>() {
        if idx > 0 && idx <= LLM_PROVIDERS.len() {
            let provider = &LLM_PROVIDERS[idx - 1];
            if !provider.endpoint.is_empty() {
                config.llm.endpoint = provider.endpoint.to_string();
            } else {
                let ep = ask("  endpoint: ");
                config.llm.endpoint = ep;
            }
            config.llm.model = provider.default_model.to_string();
            let key = ask("  API Key: ");
            config.llm.api_key = key;

            // Quick connection test
            print!("  测试连接... ");
            io::stdout().flush().ok();
            let client = fan_core::llm::LlmClient::new(config.llm.clone());
            match client.infer_candidates("test") {
                Ok(_) => println!("✅ 连接成功"),
                Err(_) => println!("⚠️ 连接失败（配置已保存，可稍后修改）"),
            }
        }
    }
    println!();
}

fn run_step_4(config: &Config) {
    println!("  ▸ 步骤 4/4：开始扫描");
    println!();
    println!("  是否现在开始扫描和推断？");
    println!("  [1] 后台运行（推荐）");
    println!("  [2] 前台运行");
    println!("  [3] 稍后手动 'fan-files daemon'");

    let input = ask("  请输入: ");

    // Save config first
    let config_path = fan_core::config::dirs_fan().join("config.toml");
    std::fs::create_dir_all(fan_core::config::dirs_fan()).ok();
    if let Ok(toml_str) = toml::to_string_pretty(config) {
        std::fs::write(&config_path, toml_str).ok();
    }

    match input.as_str() {
        "1" => {
            println!("  启动后台扫描（含远程服务器）...");
            let log_path = fan_core::config::dirs_fan().join("daemon.log");

            match std::env::current_exe() {
                Ok(bin) => {
                    let log_file = std::fs::File::create(&log_path)
                        .expect("Failed to create daemon log");
                    let result = Command::new(&bin)
                        .arg("daemon")
                        .stdin(Stdio::null())
                        .stdout(Stdio::from(log_file))
                        .stderr(Stdio::null())
                        .spawn();

                    match result {
                        Ok(child) => {
                            println!("  ✅ 后台扫描已启动 (PID: {})", child.id());
                            println!("  日志: {}", log_path.display());
                            println!("  'fan-files status' 查看进度");
                        }
                        Err(e) => {
                            eprintln!("  ❌ 启动失败: {}", e);
                            eprintln!("  请手动运行 'fan-files daemon'");
                        }
                    }
                }
                Err(e) => {
                    eprintln!("  ❌ 找不到可执行文件: {}", e);
                    eprintln!("  请手动运行 'fan-files daemon'");
                }
            }
        }
        "2" => {
            println!("  启动扫描（Ctrl+C 停止）...");
            crate::commands::daemon::run(config);
        }
        _ => {
            println!("  完成。运行 'fan-files daemon' 开始扫描。");
        }
    }
}
```

- [ ] **Step 2: Build check**

```bash
source "$HOME/.cargo/env" && cargo check 2>&1
```

- [ ] **Step 3: Commit**

```bash
git add crates/fan-files/src/commands/init.rs
git commit -m "feat: add remote server step to 'fan-files init' wizard (now 4 steps)

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Task 14: Build and run all tests

**Files:**
- Modify: `crates/fan-files/tests/cli_test.rs`

**Purpose:** Verify the whole system compiles and existing tests pass.

- [ ] **Step 1: Add server-related CLI test**

Append to `crates/fan-files/tests/cli_test.rs`:

```rust
#[test]
fn test_cli_servers_list() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("servers").arg("list").assert().success();
}

#[test]
fn test_cli_servers_help() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("servers").arg("--help").assert().success();
}
```

- [ ] **Step 2: Full build**

```bash
source "$HOME/.cargo/env" && cargo build --release 2>&1
```

Expected: SUCCESS, all crates compile.

- [ ] **Step 3: Run tests**

```bash
source "$HOME/.cargo/env" && cargo test 2>&1
```

Expected: All tests pass.

- [ ] **Step 4: Run integration smoke tests**

```bash
source "$HOME/.cargo/env" && ./target/release/fan-files --version
./target/release/fan-files --help
./target/release/fan-files status
./target/release/fan-files servers list
```

- [ ] **Step 5: Install updated binary**

```bash
source "$HOME/.cargo/env" && cargo install --path crates/fan-files
```

- [ ] **Step 6: Final verification**

```bash
source "$HOME/.cargo/env" && fan-files --version && fan-files status
```

- [ ] **Step 7: Commit**

```bash
git add crates/fan-files/tests/cli_test.rs
git commit -m "test: add servers CLI smoke tests, final build verification

via HAPI (https://hapi.run)

Co-Authored-By: HAPI <noreply@hapi.run>"
```

---

## Summary of All Commits

| # | Task | Commit Message |
|---|------|---------------|
| 1 | types.rs | `feat: add source_server to FileEntry/RawFileInfo, add ServerStats to IndexStatus` |
| 2 | scanner.rs (local) | `feat: add source_server to Scanner, populate in collect_info` |
| 3 | sqlite.rs | `feat: add source_server column to files table, migration v2, per-server stats query` |
| 4 | config.rs | `feat: add ServersConfig/ServerConfig, enabled_servers() helper with old-config fallback` |
| 5 | scanner.rs (remote) | `feat: add RemoteScanner for SSH-based file discovery on remote servers` |
| 6 | servers.rs + main.rs | `feat: add 'fan-files servers list|add|remove|scan' CLI commands` |
| 7 | status.rs | `feat: add per-server file counts to 'fan-files status' output` |
| 8 | search.rs | `feat: display <server>:<path> in search results` |
| 9 | info.rs | `feat: show source_server in 'fan-files info' output` |
| 10 | list.rs + main.rs | `feat: add --server filter to 'fan-files list' command` |
| 11 | project.rs + projects.rs | `feat: add source_server to Project struct and projects command display` |
| 12 | daemon.rs | `feat: per-server daemon scanning — local Scanner + remote RemoteScanner` |
| 13 | init.rs | `feat: add remote server step to 'fan-files init' wizard (now 4 steps)` |
| 14 | cli_test.rs | `test: add servers CLI smoke tests, final build verification` |
