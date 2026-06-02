# fan-files 易用性改进 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `fan-files init` 交互式配置 + daemon 自动 infer + 搜索体验优化 + 后台运行。

**Architecture:** 新增 `init` CLI 命令（终端问答式向导），config.rs 添加 LLM 厂商预设，daemon 增强自动触发逻辑，search 增加元数据覆盖率提示。

---

## Task 1: LLM 厂商预设 + config.rs 增强

**Files:**
- Modify: `crates/fan-core/src/config.rs`

- [ ] **Step 1: 添加 LLM 厂商预设常量**

Read `crates/fan-core/src/config.rs`

在文件末尾添加：

```rust
/// 国内常见 LLM 厂商预设
pub const LLM_PROVIDERS: &[LlmProvider] = &[
    LlmProvider {
        name: "DeepSeek",
        endpoint: "https://api.deepseek.com/v1/chat/completions",
        default_model: "deepseek-chat",
        description: "国内推荐，性价比最高（≈2元/百万tokens）",
    },
    LlmProvider {
        name: "通义千问 (Qwen)",
        endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1/chat/completions",
        default_model: "qwen-plus",
        description: "阿里云，模型矩阵丰富",
    },
    LlmProvider {
        name: "智谱 GLM",
        endpoint: "https://open.bigmodel.cn/api/paas/v4/chat/completions",
        default_model: "glm-4-flash",
        description: "国产均衡，教育优惠",
    },
    LlmProvider {
        name: "百度文心 (ERNIE)",
        endpoint: "https://aip.baidubce.com/rpc/2.0/ai_custom/v1/wenxinworkshop/chat/completions",
        default_model: "ernie-4.0-turbo-8k",
        description: "稳定性强，企业级",
    },
    LlmProvider {
        name: "OpenAI / 自定义",
        endpoint: "",
        default_model: "gpt-4o-mini",
        description: "自行填写 endpoint 和 key",
    },
];

pub struct LlmProvider {
    pub name: &'static str,
    pub endpoint: &'static str,
    pub default_model: &'static str,
    pub description: &'static str,
}
```

- [ ] **Step 2: Build & commit**

```bash
cargo build && cargo test && git add -A && git commit -m "feat: add LLM provider presets (DeepSeek/Qwen/GLM/ERNIE/OpenAI)"
```

---

## Task 2: `fan-files init` 交互式向导

**Files:**
- Create: `crates/fan-files/src/commands/init.rs`
- Modify: `crates/fan-files/src/main.rs`
- Modify: `crates/fan-files/src/commands/mod.rs`

- [ ] **Step 1: 创建 init.rs**

```rust
use fan_core::config::{Config, LLM_PROVIDERS};
use std::io::{self, Write};

pub fn run(config: &Config) {
    println!();
    println!("  ╔══════════════════════════════════════╗");
    println!("  ║   Fan-Files 初始化配置向导          ║");
    println!("  ╚══════════════════════════════════════╝");
    println!();
    let mut new_config: fan_core::config::Config = config.clone();

    // ── Step 1: Scan directories ──
    println!("  ▸ 步骤 1/3：扫描目录");
    println!();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let mut dirs: Vec<String> = new_config.scan.include.clone();
    if dirs.is_empty() {
        dirs.push(home.clone());
    }

    loop {
        println!("  当前扫描目录:");
        for (i, d) in dirs.iter().enumerate() {
            println!("    [{}] {}", i + 1, d);
        }
        println!();
        println!("  [a] 添加目录");
        println!("  [d] 删除目录");
        println!("  [Enter] 完成，进入下一步");
        print!("  请输入: ");
        io::stdout().flush().ok();
        let mut input = String::new();
        io::stdin().read_line(&mut input).ok();
        let input = input.trim();

        match input {
            "a" | "A" => {
                print!("  请输入目录路径: ");
                io::stdout().flush().ok();
                let mut path = String::new();
                io::stdin().read_line(&mut path).ok();
                let path = path.trim().to_string();
                if !path.is_empty() && !dirs.contains(&path) {
                    dirs.push(path);
                }
            }
            "d" | "D" => {
                print!("  输入要删除的序号: ");
                io::stdout().flush().ok();
                let mut n = String::new();
                io::stdin().read_line(&mut n).ok();
                if let Ok(idx) = n.trim().parse::<usize>() {
                    if idx > 0 && idx <= dirs.len() {
                        dirs.remove(idx - 1);
                    }
                }
            }
            "" => break,
            _ => println!("  无效输入"),
        }
    }
    new_config.scan.include = dirs.clone();
    new_config.watch.include = dirs;
    println!();

    // ── Step 2: LLM ──
    println!("  ▸ 步骤 2/3：LLM 元数据推断");
    println!();
    println!("  LLM 可自动识别项目、物种、实验类型。请选择:");
    for (i, p) in LLM_PROVIDERS.iter().enumerate() {
        println!("  [{}] {} — {}", i + 1, p.name, p.description);
    }
    println!("  [s] 暂时跳过");
    print!("  请输入: ");
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();

    if let Ok(idx) = input.trim().parse::<usize>() {
        if idx > 0 && idx <= LLM_PROVIDERS.len() {
            let provider = &LLM_PROVIDERS[idx - 1];
            new_config.llm.endpoint = provider.endpoint.to_string();
            new_config.llm.model = provider.default_model.to_string();

            print!("  API Key: ");
            io::stdout().flush().ok();
            let mut key = String::new();
            io::stdin().read_line(&mut key).ok();
            new_config.llm.api_key = key.trim().to_string();

            // Test connection
            println!("  测试连接...");
            let client = fan_core::llm::LlmClient::new(new_config.llm.clone());
            match client.infer_candidates("test") {
                Ok(_) => println!("  ✅ 连接成功"),
                Err(e) => println!("  ⚠️ 连接失败: {}（配置已保存，可稍后修改）", e),
            }
        }
    }
    println!();

    // ── Step 3: Start scan ──
    println!("  ▸ 步骤 3/3：开始扫描");
    println!();
    println!("  配置已保存到 ~/.fan-files/config.toml");
    println!("  现在开始扫描和推断？");
    println!("  [1] 后台运行（推荐）");
    println!("  [2] 前台运行");
    println!("  [3] 稍后手动 'fan-files daemon'");
    print!("  请输入: ");
    io::stdout().flush().ok();
    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();

    // Save config
    let config_path = fan_core::config::dirs_fan().join("config.toml");
    std::fs::write(&config_path, toml::to_string_pretty(&new_config).unwrap()).ok();

    match input.trim() {
        "1" => {
            println!("  已启动后台扫描...");
            println!("  'fan-files status' 查看进度");
            // Spawn daemon in background
            std::process::Command::new("fan-files")
                .arg("daemon")
                .arg("--background")
                .spawn()
                .ok();
        }
        "2" => {
            println!("  启动扫描...");
            crate::commands::daemon::run(&new_config);
        }
        _ => {
            println!("  完成。运行 'fan-files daemon' 开始扫描。");
        }
    }
}
```

- [ ] **Step 2: 添加 CLI 命令到 main.rs**

Read `crates/fan-files/src/main.rs`，在 Commands enum 添加：

```rust
    /// Interactive setup wizard
    Init,
```

match 分支添加：

```rust
        Commands::Init => commands::init::run(&config),
```

`crates/fan-files/src/commands/mod.rs` 添加 `pub mod init;`

- [ ] **Step 3: 添加 toml 依赖到 fan-files Cargo.toml**

Read `crates/fan-files/Cargo.toml`，确保 `toml` 在 dependencies 中。

- [ ] **Step 4: Build & test**

```bash
cargo build && cargo test && ./target/debug/fan-files init
```

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: add fan-files init interactive setup wizard"
```

---

## Task 3: Daemon 自动 Infer 增强 + 状态反馈

**Files:**
- Modify: `crates/fan-files/src/commands/daemon.rs`

- [ ] **Step 1: 增强 daemon 中的 infer 触发逻辑**

Read `crates/fan-files/src/commands/daemon.rs`。当前已有 LLM 触发，添加：

1. 首次扫描后 infer 时，打印 "正在推断项目元数据..."
2. 增量变化超过 10 个文件时，自动重新推断
3. 在 `run()` 函数内维护一个 `new_file_count` 计数器

在 `run()` 中，watch 循环里对每次索引成功的文件计数：

```rust
let mut new_files_since_infer: u64 = 0;
```

每次 `index_file` 成功后 `new_files_since_infer += 1`。当 >= 10 时触发 infer。

- [ ] **Step 2: 更新 status 命令显示元数据覆盖率**

Read `crates/fan-files/src/commands/status.rs`

在现有输出中追加元数据覆盖率：

```rust
let total = status.indexed_files;
let with_meta = sqlite_query_count_with_bio_metadata(&index);
let pct = if total > 0 { (with_meta as f64 / total as f64) * 100.0 } else { 0.0 };
println!("Metadata coverage: {:.0}% ({}/{})", pct, with_meta, total);
if pct < 50.0 {
    println!("  ⚠ Metadata coverage is low. Run 'fan-files infer' for better search results.");
}
```

需要在 SqliteStore 添加这个方法：

```rust
pub fn count_with_bio_metadata(&self) -> rusqlite::Result<u64> {
    let conn = self.conn.lock().unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM files WHERE bio_metadata_json IS NOT NULL AND deleted=0",
        [],
        |r| r.get::<_, i64>(0),
    )
    .map(|c| c as u64)
}
```

- [ ] **Step 3: Build & test**

```bash
cargo build && cargo test && ./target/debug/fan-files status
```

Expected: 显示 `Metadata coverage: XX%`

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat: auto-infer on 10+ new files + metadata coverage in status"
```

---

## Task 4: 搜索元数据覆盖率提示

**Files:**
- Modify: `crates/fan-files/src/commands/search.rs`

- [ ] **Step 1: 在 search 结果中加入覆盖率提示**

Read `crates/fan-files/src/commands/search.rs`，在 `run()` 函数的 JSON/文本输出前添加：

```rust
    // Check metadata coverage
    let total = index.sqlite.status().unwrap().indexed_files;
    let with_meta = index.sqlite.count_with_bio_metadata().unwrap_or(0);
    let coverage_pct = if total > 0 { (with_meta as f64 / total as f64) * 100.0 } else { 0.0 };

    if coverage_pct < 50.0 && !json {
        println!("⚠ Metadata coverage is low ({:.0}%). Run 'fan-files infer' for better results.", coverage_pct);
        println!();
    }
```

- [ ] **Step 2: Build & test**

```bash
cargo build && cargo test
```

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat: show metadata coverage warning in search when <50%"
```

---

## Task 5: 端到端验证

- [ ] **Step 1: 测试 init 命令**

```bash
cargo build --release
./target/release/fan-files init
# 模拟走一遍配置流程
```

- [ ] **Step 2: 测试 status 覆盖率**

```bash
./target/release/fan-files status
```

Expected: 显示 Metadata coverage 百分比

- [ ] **Step 3: 测试搜索**

```bash
./target/release/fan-files search "RNA-seq"
```

Expected: 低覆盖率时显示 ⚠ 提示

- [ ] **Step 4: Commit & push**

```bash
cargo test && git add -A && git commit -m "chore: usability improvements with init wizard + auto-infer + search hints" && git push
```
