# Fan-Files: Source Server Tracking — Design Spec

**Date:** 2026-06-20
**Status:** approved
**Branch:** main

## Overview

Add a `source_server` concept to fan-files so every indexed file records which server it came from. This enables Mac mini to scan remote servers (via SSH) and query all data centrally, knowing the origin of each file.

## Architecture: Model A — Centralized Scan + Index on Mac mini

```
Mac mini runs fan-files daemon
  ├─ [servers.local]   → local Scanner (walkdir), unchanged
  ├─ [servers.ai-srv]  → SSH + find             → IndexEngine
  ├─ [servers.yz-hpc]  → SSH + find             → IndexEngine
  └─ [servers.gpu-h100] → SSH + find             → IndexEngine
```

All indexed data lives in Mac mini's `~/.fan-files/index.db`. Remote servers only need SSH access — no fan-files or Rust installation required.

---

## Part 1: Data Model

### SQLite Schema Changes

```sql
-- Migration v2
ALTER TABLE files ADD COLUMN source_server TEXT NOT NULL DEFAULT 'local';
ALTER TABLE project ADD COLUMN source_server TEXT DEFAULT 'local';
CREATE INDEX IF NOT EXISTS idx_files_server ON files(source_server);
```

### FileEntry (types.rs)

```rust
pub struct FileEntry {
    pub id: i64,
    pub path: PathBuf,              // Remote absolute path
    pub source_server: String,      // NEW — "ai-srv", "yz-hpc", "local"...
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
```

### IndexStatus (types.rs) — Add per-server breakdown

```rust
#[derive(Debug, Clone, Serialize)]
pub struct ServerStats {
    pub server: String,
    pub file_count: u64,
    pub last_scan: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct IndexStatus {
    pub total_files: u64,
    pub indexed_files: u64,
    pub deleted_files: u64,
    pub last_full_scan: Option<i64>,
    pub last_change: Option<i64>,
    pub db_size_bytes: u64,
    pub servers: Vec<ServerStats>,  // NEW
}
```

### Design decisions

- **Field name:** `source_server` — clear semantics, distinct from hostname
- **Default value:** `"local"` — existing data auto-migrated, no disruption
- **Path content:** remote absolute path (e.g. `/mnt/data/proj/sample.fq`) — directly readable, displayed as `<server>:<path>`

---

## Part 2: Server Registry + Configuration

### TOML config format (`~/.fan-files/config.toml`)

```toml
[servers.local]
host = ""
scan_root = "/Users/kentnf/Data"
label = "Mac mini 本地"

[servers.ai-srv]
host = "ai-srv"                # SSH Host name (from ~/.ssh/config)
scan_root = "/mnt/data"
label = "AI 训练服务器"
enabled = true

[servers.yz-hpc]
host = "yz-hpc"
scan_root = "/data/seq"
label = "崖州湾 HPC"
enabled = true
```

### Rust config types (config.rs)

```rust
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ServersConfig {
    #[serde(flatten)]
    pub servers: HashMap<String, ServerConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub host: String,              // SSH host name, empty = local
    pub scan_root: String,         // Root directory to scan
    #[serde(default)]
    pub label: Option<String>,     // Human-readable description
    #[serde(default = "default_true")]
    pub enabled: bool,             // Whether to include in scan
}

fn default_true() -> bool { true }
```

### Design decisions

- **Config format:** `[servers.<name>]` — `<name>` becomes the `source_server` value
- **SSH Host:** Reuses `~/.ssh/config` Host names — no need to duplicate IP/port/key
- **scan_root:** One per server initially; extensible to array later
- **Backward compat:** Old `scan.include` auto-mapped to `[servers.local]`

### New CLI subcommands

```
fan-files servers list              # List all registered servers
fan-files servers add <name>       # Interactive add wizard
fan-files servers remove <name>    # Remove a server
fan-files servers scan <name>      # Scan only one server
```

---

## Part 3: SSH Remote Scanner

### Per-server scan flow

```
For each enabled server where host != "":
  1. ssh <host> "find <scan_root> -type f -printf '%p\t%s\t%T@\n'"
     → Parse: path, size_bytes, mtime_unix_epoch
  2. For each new/changed file:
       ssh <host> "head -c 512 '<path>' | base64"
     → Decode → magic_bytes (first 512B)
  3. Construct RawFileInfo:
       path = remote_absolute_path
       source_server = server_name
  4. Feed to existing IndexEngine pipeline (same as local)
  5. Files that disappeared on remote → mark_deleted()
```

### Two strategies

| | Fast scan (default) | Full scan |
|---|---|---|
| `find` with mtime/size | ✅ | ✅ |
| Remote magic bytes | ❌ skipped | ✅ `head -c 512` |
| Remote SHA256 | ❌ skipped | ✅ `sha256sum` |
| Use case | Initial discovery + incremental | First index build, periodic full sync |
| Speed | One SSH + find | One SSH per file |

### Incremental update (no inotify for remote)

1. `find` returns current file list
2. Compare with stored (path, mtime, size) per file
3. New/changed → fetch magic bytes → re-index
4. Disappeared → mark_deleted
5. Unchanged → skip

### Implementation approach

- New `RemoteScanner` struct in `fan-core/src/scanner.rs`
- Uses `std::process::Command` for SSH calls (existing pattern from `init.rs` line 139)
- Each server scanned via `tokio::spawn` in daemon loop for parallelism
- SSH failures: 3 retries with 2s backoff, then skip with error log

---

## Part 4: CLI Output Changes

### `fan-files status`

```
fan-files Index Status
======================
Indexed files:   138,421
Metadata coverage: 72% (99,663/138,421)

Servers:
  local         6,201 files  (last scan: 2026-06-20 14:30)
  ai-srv       45,830 files  (last scan: 2026-06-20 14:28)
  yz-hpc       62,180 files  (last scan: 2026-06-19 03:00)
  gpu-h100     24,210 files  (last scan: 2026-06-20 14:29)
```

### `fan-files search`

```
$ fan-files search "RNA-seq rice"
 1. ai-srv:/mnt/data/proj-rice/RNASeq/Sample_A.fastq.gz  (92%)
 2. ai-srv:/mnt/data/proj-rice/RNASeq/Sample_B.fastq.gz  (89%)
 3. yz-hpc:/data/seq/rice_rna/batch3/ctrl_1.fq           (85%)
```

Path format: `<server>:<remote_path>`

### `fan-files info`

```
$ fan-files info 1
Path:           /mnt/data/proj-rice/RNASeq/Sample_A.fastq.gz
Source Server:  ai-srv (AI 训练服务器)
Size:           2.1 GB
...
```

### `fan-files list`

Add `--server <name>` filter option.

### `fan-files projects`

Add `Server` column to table output.

---

## Part 5: Migration & Compatibility

### Database migration

- SQL ALTER TABLE adds `source_server` column with `DEFAULT 'local'`
- Existing files auto-tagged as `source_server = 'local'`
- New index `idx_files_server` for server-filtered queries
- All existing queries still work (column has default)

### Config migration

- Old `config.scan.include` paths → `[servers.local]` with `scan_root` set to the first entry
- Print migration notice on first load: `⚠️  scan.include migrated to [servers.local]`
- Old `config.watch.include` → `[servers.local]` watch paths (watcher only works for local)

### Approach: lazy migration

No forced migration at startup. Both old and new config formats coexist:
- If `servers` map is empty but `scan.include` is non-empty → treat `scan.include` as `[servers.local]` roots
- If `servers` map has entries → use only `servers` config
- New `fan-files init` writes the new format only

### Compatibility checklist

| Existing feature | Impact |
|---|---|
| `search` | Path display format: `<server>:<path>` |
| `suggest` | No change |
| `info` | One extra output line for `source_server` |
| `list` | Optional `--server` filter |
| `daemon` | Additional remote scan tasks |
| `projects` | One extra column |
| `pending` | No change |

---

## Part 6: Testing

| Scenario | Expected behavior |
|---|---|
| Local scan, fresh install | `source_server = local`, all existing features work |
| Remote `find` fails (SSH down) | Graceful degradation, error logged, other servers unaffected |
| SSH timeout | 3 retries with backoff, then skip, preserve `last_scan` from last success |
| Empty remote directory | No crash, file count = 0 |
| Cross-server search | Results from multiple servers merge correctly, sorted by relevance |
| Incremental update | Unchanged files (same mtime+size) not re-indexed |
| New file appears remotely | Detected, magic bytes fetched, indexed |
| File deleted remotely | `mark_deleted()` called |
| Config migration | Old `scan.include` → `[servers.local]` with migration notice |
| Database migration | `source_server = 'local'` on all existing rows |

---

## Files to Change

| File | Change |
|---|---|
| `crates/fan-core/src/types.rs` | Add `source_server` to `FileEntry`, add `ServerStats`, update `IndexStatus` |
| `crates/fan-core/src/index/sqlite.rs` | Migration v2, new column in queries, per-server stats query |
| `crates/fan-core/src/config.rs` | Add `ServersConfig`, `ServerConfig`, migration from old format |
| `crates/fan-core/src/scanner.rs` | Add `RemoteScanner` with SSH-based file discovery |
| `crates/fan-core/src/lib.rs` | Export new types |
| `crates/fan-files/src/commands/daemon.rs` | Per-server scan loop with tokio::spawn |
| `crates/fan-files/src/commands/status.rs` | Per-server stats display |
| `crates/fan-files/src/commands/search.rs` | `<server>:<path>` display format |
| `crates/fan-files/src/commands/info.rs` | Show source_server line |
| `crates/fan-files/src/commands/init.rs` | Add step for remote server registration |
| `crates/fan-files/src/commands/list.rs` | Add `--server` filter |
| `crates/fan-files/src/commands/mod.rs` | Register `servers` subcommand |
| `crates/fan-files/src/commands/servers.rs` | New file: `servers list/add/remove/scan` |
| `crates/fan-files/src/main.rs` | Add `servers` subcommand to CLI |
| `crates/fan-files/tests/cli_test.rs` | Add server-related test cases |

## Non-Goals (explicitly out of scope)

- Network API / gRPC layer for remote fan-files instances (Model B)
- Real-time file watching on remote servers
- SSHFS/NFS mount support
- Multi-directory scan per server (one `scan_root` per server)
- Authentication beyond `~/.ssh/config`
- Cross-server file deduplication
