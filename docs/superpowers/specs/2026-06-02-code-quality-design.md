# fan-files 代码质量改进

## 审查发现

| # | 问题 | 严重度 | 影响 |
|---|------|--------|------|
| 1 | CLI 命令重复打开数据库 | 中 | 9 个命令各自 `IndexEngine::open()` 或 `SqliteStore::open()` |
| 2 | 58 个 `.unwrap()` | 中 | Panic 风险，daemon 等常驻进程不能崩 |
| 3 | 测试覆盖不均 | 中 | CLI 命令零测试，改了 search 不知道坏了没 |
| 4 | 4 个编译 warning | 低 | ✅ 已修复 |

## 改进方案

### 1. 提取公共连接函数

`fan-core` 新增 `open_index()` 函数：

```rust
use crate::config::Config;
use crate::index::IndexEngine;

pub enum IndexMode {
    ReadOnly,     // CLI 命令（search/suggest/list/info/status/projects）
    ReadWrite,    // daemon
    SqliteOnly,   // infer/init（不需要 Tantivy + ONNX）
}

pub fn open_index(config: &Config, mode: IndexMode) -> Result<IndexEngine, Box<dyn std::error::Error>> {
    match mode {
        IndexMode::ReadOnly => IndexEngine::open(config, true),
        IndexMode::ReadWrite => IndexEngine::open(config, false),
        IndexMode::SqliteOnly => unreachable!(), // handled by caller
    }
}
```

### 2. 修复关键路径 unwrap

优先级：daemon > search > infer > suggest > 其他

```rust
// Before
let index = IndexEngine::open(config, true).unwrap();

// After
let index = IndexEngine::open(config, true)
    .map_err(|e| format!("Failed to open index: {}", e))?;
```

### 3. CLI 集成测试

测试 binary 本身——验证 `--help`、`status`、`projects` 不崩溃：

```rust
#[test]
fn test_cli_help() {
    let output = Command::new("fan-files").arg("--help").output().unwrap();
    assert!(output.status.success());
}

#[test]
fn test_cli_status() {
    let output = Command::new("fan-files").arg("status").output().unwrap();
    assert!(output.status.success());
}
```
