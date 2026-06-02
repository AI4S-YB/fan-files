# fan-files 安装/卸载/升级 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `install.sh` 一键安装（含 skill），`fan-files update` 自升级，`fan-files uninstall` 两步卸载。

**Architecture:** Bash 安装脚本 + 两个 Rust CLI 子命令。skill 同步贯穿全部三个操作。

---

## Task 1: `install.sh` 安装脚本

**Files:**
- Create: `install.sh`

- [ ] **Step 1: 创建 install.sh**

```bash
#!/bin/bash
set -e

echo "╔══════════════════════════════════════╗"
echo "║   Fan-Files 安装脚本                ║"
echo "╚══════════════════════════════════════╝"
echo ""

OS=$(uname -s)
REPO="https://github.com/AI4S-YB/fan-files.git"
INSTALL_DIR="/tmp/fan-files-install"

# 1. Check dependencies
echo "▸ 检查依赖..."
if ! command -v cargo &>/dev/null; then
    echo "  Rust 未安装，正在安装..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env"
fi

if ! command -v git &>/dev/null; then
    echo "  git 未安装。请手动安装 git 后重试。"
    exit 1
fi

echo "  ✅ Rust: $(rustc --version)"
echo "  ✅ Git:  $(git --version)"

# 2. Clone and build
echo ""
echo "▸ 下载源码..."
rm -rf "$INSTALL_DIR"
git clone --depth 1 "$REPO" "$INSTALL_DIR"

echo ""
echo "▸ 编译 (约 2-5 分钟)..."
cd "$INSTALL_DIR"
cargo build --release

# 3. Install binary
echo ""
echo "▸ 安装..."
sudo cp target/release/fan-files /usr/local/bin/fan-files

# Record install source for future updates
mkdir -p "$HOME/.fan-files"
echo "$INSTALL_DIR" > "$HOME/.fan-files/install_source"

# 4. Install Claude Code Skill
echo ""
echo "▸ 安装 Claude Code Skill..."
mkdir -p "$HOME/.claude/skills"
if [ -f "SKILL.md" ]; then
    cp SKILL.md "$HOME/.claude/skills/fan-files.md"
    echo "  ✅ Skill 已安装到 ~/.claude/skills/fan-files.md"
else
    echo "  ⚠ SKILL.md 未找到，跳过 skill 安装"
fi

echo ""
echo "╔══════════════════════════════════════╗"
echo "║  ✅ fan-files 安装完成！            ║"
echo "║                                    ║"
echo "║  运行 'fan-files init' 开始配置    ║"
echo "╚══════════════════════════════════════╝"
```

- [ ] **Step 2: 测试脚本语法**

```bash
bash -n install.sh && echo "syntax ok"
```

- [ ] **Step 3: Commit**

```bash
git add install.sh && git commit -m "feat: add one-liner install.sh with Rust auto-install + skill sync"
```

---

## Task 2: `fan-files update` 命令

**Files:**
- Create: `crates/fan-files/src/commands/update.rs`
- Modify: `crates/fan-files/src/main.rs`
- Modify: `crates/fan-files/src/commands/mod.rs`

- [ ] **Step 1: 创建 update.rs**

```rust
use std::process::Command;

pub fn run() {
    // Read install source path
    let home = std::env::var("HOME").unwrap_or_default();
    let source_file = format!("{}/.fan-files/install_source", home);
    let source_dir = std::fs::read_to_string(&source_file).unwrap_or_default();
    let source_dir = source_dir.trim();

    if source_dir.is_empty() {
        eprintln!("无法找到源码目录（可能由包管理器安装）。");
        eprintln!("重新运行安装脚本获取最新版本：");
        eprintln!("  curl -fsSL https://raw.githubusercontent.com/AI4S-YB/fan-files/main/install.sh | bash");
        return;
    }

    println!("▸ 更新源码: {}", source_dir);
    let pull = Command::new("git")
        .args(["-C", source_dir, "pull"])
        .status();

    if pull.is_err() || !pull.unwrap().success() {
        eprintln!("git pull 失败，请检查网络或手动更新");
        return;
    }

    println!("▸ 重新编译...");
    let build = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(source_dir)
        .status();

    if build.is_err() || !build.unwrap().success() {
        eprintln!("编译失败");
        return;
    }

    println!("▸ 安装...");
    let mut bin_path = std::path::PathBuf::from(source_dir);
    bin_path.push("target/release/fan-files");
    let install = Command::new("sudo")
        .args(["cp", bin_path.to_str().unwrap(), "/usr/local/bin/fan-files"])
        .status();

    if install.is_ok() && install.unwrap().success() {
        // Update skill
        let skill_src = format!("{}/SKILL.md", source_dir);
        let skill_dst = format!("{}/.claude/skills/fan-files.md", home);
        std::fs::copy(&skill_src, &skill_dst).ok();
        println!("✅ fan-files 已升级到最新版本，skill 已同步更新");
    } else {
        eprintln!("安装失败");
    }
}
```

- [ ] **Step 2: 添加 CLI 到 main.rs**

Read `crates/fan-files/src/main.rs`，在 Commands enum 添加：

```rust
    /// Update fan-files to the latest version
    Update,
```

match 分支：

```rust
        Commands::Update => commands::update::run(),
```

`crates/fan-files/src/commands/mod.rs` 添加 `pub mod update;`

- [ ] **Step 3: Build & commit**

```bash
cargo build && git add -A && git commit -m "feat: add fan-files update self-upgrade command"
```

---

## Task 3: `fan-files uninstall` 命令

**Files:**
- Create: `crates/fan-files/src/commands/uninstall.rs`
- Modify: `crates/fan-files/src/main.rs`
- Modify: `crates/fan-files/src/commands/mod.rs`

- [ ] **Step 1: 创建 uninstall.rs**

```rust
use std::io::{self, Write};
use std::process::Command;

pub fn run() {
    println!();
    println!("  ⚠ 即将卸载 fan-files");
    println!();
    println!("  [1] 仅卸载程序");
    println!("      删除: /usr/local/bin/fan-files + 源码 + skill");
    println!("      保留: ~/.fan-files/ (数据库、配置、模型、插件)");
    println!();
    println!("  [2] 完全卸载");
    println!("      删除: 程序 + 源码 + skill + ~/.fan-files/ 全部数据");
    println!();
    println!("  [q] 取消");
    println!();
    print!("  请输入: ");
    io::stdout().flush().ok();

    let mut input = String::new();
    io::stdin().read_line(&mut input).ok();

    let home = std::env::var("HOME").unwrap_or_default();
    let source_file = format!("{}/.fan-files/install_source", home);
    let source_dir = std::fs::read_to_string(&source_file).unwrap_or_default().trim().to_string();

    match input.trim() {
        "1" => {
            // Remove binary
            println!("  删除 /usr/local/bin/fan-files...");
            let _ = Command::new("sudo").args(["rm", "/usr/local/bin/fan-files"]).status();

            // Remove source
            if !source_dir.is_empty() {
                println!("  删除源码: {}", source_dir);
                let _ = std::fs::remove_dir_all(&source_dir);
            }

            // Remove skill
            let skill_path = format!("{}/.claude/skills/fan-files.md", home);
            if std::path::Path::new(&skill_path).exists() {
                println!("  删除 Claude Code Skill...");
                let _ = std::fs::remove_file(&skill_path);
            }

            println!();
            println!("  ✅ fan-files 程序已卸载");
            println!("  数据保留在 ~/.fan-files/，重新安装后可直接使用");
        }
        "2" => {
            // Full uninstall
            println!("  删除 /usr/local/bin/fan-files...");
            let _ = Command::new("sudo").args(["rm", "/usr/local/bin/fan-files"]).status();

            if !source_dir.is_empty() {
                println!("  删除源码: {}", source_dir);
                let _ = std::fs::remove_dir_all(&source_dir);
            }

            let skill_path = format!("{}/.claude/skills/fan-files.md", home);
            if std::path::Path::new(&skill_path).exists() {
                println!("  删除 Claude Code Skill...");
                let _ = std::fs::remove_file(&skill_path);
            }

            let fan_dir = format!("{}/.fan-files", home);
            if std::path::Path::new(&fan_dir).exists() {
                println!("  删除 ~/.fan-files/ (数据库、配置、模型)...");
                let _ = std::fs::remove_dir_all(&fan_dir);
            }

            println!();
            println!("  ✅ fan-files 已完全卸载");
        }
        _ => {
            println!("  已取消");
        }
    }
}
```

- [ ] **Step 2: 添加 CLI 到 main.rs**

Read `crates/fan-files/src/main.rs`，在 Commands enum 添加：

```rust
    /// Uninstall fan-files
    Uninstall,
```

match 分支：

```rust
        Commands::Uninstall => commands::uninstall::run(),
```

`crates/fan-files/src/commands/mod.rs` 添加 `pub mod uninstall;`

- [ ] **Step 3: Build & commit**

```bash
cargo build && git add -A && git commit -m "feat: add fan-files uninstall command with partial/full options"
```

---

## Task 4: 端到端验证

- [ ] **Step 1: Build release**

```bash
cargo build --release && cargo test
```

All 21 tests must pass.

- [ ] **Step 2: 测试 install.sh**

```bash
bash -n install.sh && echo "syntax ok"
```

- [ ] **Step 3: 测试 CLI**

```bash
./target/release/fan-files --help | grep -E "update|uninstall|init"
```

Expected: `update`, `uninstall`, `init` 三个命令出现。

- [ ] **Step 4: Commit & push**

```bash
git add -A && git commit -m "chore: finalize install/uninstall/update with skill sync" && git push
```
