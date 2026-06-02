# fan-files 代码质量改进 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 提取公共数据库连接函数，修复 daemon 关键路径 unwrap，为 CLI 命令加集成测试。

**Architecture:** fan-core 新增 `open_index()` 公共函数，各 CLI 命令替换自己的 open 调用。测试用 `assert_cmd` crate。

---

## Task 1: 提取公共连接函数 + 清理编译 warning

**Files:**
- Modify: `crates/fan-core/src/index/mod.rs` (add `IndexMode` enum + `open_index()`)
- Modify: `crates/fan-files/src/commands/` (9 个命令替换调用)

- [ ] **Step 1: 在 index/mod.rs 添加 IndexMode 和 open_index**

Read `crates/fan-core/src/index/mod.rs`，在 `IndexEngine` 前添加：

```rust
pub enum IndexMode {
    ReadOnly,
    ReadWrite,
}
```

添加公开函数：

```rust
/// Open the index with the given mode. This is the canonical entry point
/// for all CLI commands and daemon.
pub fn open_index(config: &Config, mode: IndexMode) -> Result<IndexEngine, Box<dyn std::error::Error>> {
    IndexEngine::open(config, matches!(mode, IndexMode::ReadOnly))
}
```

Export `IndexMode` from `crates/fan-core/src/lib.rs`：在 `pub mod index;` 下面添加 `pub use index::IndexMode;`

- [ ] **Step 2: 替换所有 IndexEngine::open 调用**

用以下模式替换 9 个命令文件中的 `IndexEngine::open(config, ...)`

```rust
use fan_core::index::{open_index, IndexMode};

let index = open_index(config, IndexMode::ReadOnly)
    .map_err(|e| eprintln!("Failed to open index: {}", e)).ok()?;
```

具体替换的文件和行（用 sed 批量）：
- `crates/fan-files/src/commands/search.rs:7` — `IndexMode::ReadOnly`
- `crates/fan-files/src/commands/suggest.rs:6` — `IndexMode::ReadOnly`
- `crates/fan-files/src/commands/list.rs:5` — `IndexMode::ReadOnly`
- `crates/fan-files/src/commands/info.rs:6` — `IndexMode::ReadOnly`
- `crates/fan-files/src/commands/status.rs:5` — `IndexMode::ReadOnly`
- `crates/fan-files/src/commands/daemon.rs:20` — `IndexMode::ReadWrite`

- [ ] **Step 3: Build & test**

```bash
cargo build && cargo test && git add -A && git commit -m "refactor: extract open_index helper, remove duplicated IndexEngine::open calls"
```

---

## Task 2: 修复 daemon 关键路径 unwrap

**Files:**
- Modify: `crates/fan-files/src/commands/daemon.rs`

- [ ] **Step 1: 替换 daemon 中的 expect/unwrap**

Read `crates/fan-files/src/commands/daemon.rs`，找到所有 `.expect()` 和 `.unwrap()` 调用，替换为 proper error handling。

关键行：
- `let index = IndexEngine::open(config, false).expect(...)` → 用 `?` 在 main 中处理
- `run_full_scan` 中的 `.unwrap_or_default()` → 保持（安全）
- watcher 创建处的 `.expect()` → 返回 Error

- [ ] **Step 2: Build & commit**

```bash
cargo build && cargo test && git add -A && git commit -m "fix: replace daemon expect/unwrap with proper error handling"
```

---

## Task 3: CLI 集成测试

**Files:**
- Modify: `crates/fan-files/Cargo.toml` (add dev-dependency)
- Create: `crates/fan-files/tests/cli_test.rs`

- [ ] **Step 1: 添加 assert_cmd 依赖**

Read `crates/fan-files/Cargo.toml`，添加：

```toml
[dev-dependencies]
assert_cmd = "2"
predicates = "3"
```

- [ ] **Step 2: 创建 CLI 集成测试**

```rust
use assert_cmd::Command;

#[test]
fn test_cli_help() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("--help").assert().success();
}

#[test]
fn test_cli_version() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("--version").assert().success();
}

#[test]
fn test_cli_status() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("status").assert().success();
}

#[test]
fn test_cli_projects() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("projects").assert().success();
}

#[test]
fn test_cli_pending() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("pending").assert().success();
}

#[test]
fn test_cli_search_empty() {
    let mut cmd = Command::cargo_bin("fan-files").unwrap();
    cmd.arg("search").arg("xyz_no_match").assert().success();
}
```

- [ ] **Step 3: Run tests & commit**

```bash
cargo test && git add -A && git commit -m "test: add CLI integration smoke tests (help, status, projects, search)"
```
