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
