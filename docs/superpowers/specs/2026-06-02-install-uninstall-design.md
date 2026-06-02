# fan-files 安装/卸载/升级

## 概述

用 install.sh 脚本实现跨平台一键安装，fan-files 内置 update 和 uninstall 子命令。

## 安装：install.sh

用户无需安装 Rust 知识：

```bash
curl -fsSL https://raw.githubusercontent.com/AI4S-YB/fan-files/main/install.sh | bash
```

install.sh 流程：

1. 检测 OS（`uname -s`），macOS 和 Linux 各自适配
2. 检查 Rust：`which cargo || curl https://sh.rustup.rs -sSf | sh -s -- -y`
3. 检查 git：`which git || (macOS: xcode-select --install; Linux: apt install git -y)`
4. git clone 到 `/tmp/fan-files-install`
5. `cargo build --release`
6. `sudo cp target/release/fan-files /usr/local/bin/fan-files`
7. 清理临时文件
8. 输出：`✅ fan-files 安装完成！运行 'fan-files init' 开始配置。`

## 升级：`fan-files update`

```bash
fan-files update
```

1. 找到源码目录（编译时嵌入，或者在 `~/.fan-files/install_source` 记录）
2. `git -C <source_dir> pull`
3. `cargo build --release`
4. `sudo cp target/release/fan-files /usr/local/bin/fan-files`
5. 输出：`✅ fan-files 升级到 vX.Y.Z`

如果用户是从预编译二进制安装的（没有源码目录），提示：
```
fan-files 由预编译二进制安装，无法自动升级。
重新运行安装脚本获取最新版本：
  curl -fsSL https://raw.githubusercontent.com/AI4S-YB/fan-files/main/install.sh | bash
```

## 卸载：`fan-files uninstall`

```
$ fan-files uninstall

  ⚠ 即将卸载 fan-files

  [1] 仅卸载程序
      删除: /usr/local/bin/fan-files + 源码目录
      保留: ~/.fan-files/ (数据库、配置、模型、插件)

  [2] 完全卸载
      删除: 程序 + 源码 + ~/.fan-files/ 全部数据

  [q] 取消
```

| 选项 | 删除 | 保留 | 重装后 |
|------|------|------|--------|
| 仅卸载 | 二进制 + 源码 + skill | `~/.fan-files/` | 直接可用，数据全在 |
| 完全卸载 | 全部 | 无 | 需要重新 `fan-files init` |

## Skill 同步

### 安装时

install.sh 最后一步：将 SKILL.md 安装到全局 Claude Code skills 目录。

```bash
# 安装 Claude Code Skill
mkdir -p ~/.claude/skills
cp SKILL.md ~/.claude/skills/fan-files.md
echo "✅ Claude Code Skill 已安装到 ~/.claude/skills/fan-files.md"
```

如果用户已通过 fan-marketplace 安装，跳过这一步（检测 `~/.claude/skills/fan-files.md` 是否已存在）。

### 升级时

`fan-files update` 中，git pull 后也更新 skill：

```bash
cp SKILL.md ~/.claude/skills/fan-files.md
```

### 卸载时

| 选项 | skill 处理 |
|------|-----------|
| 仅卸载 | 删除 `~/.claude/skills/fan-files.md` |
| 完全卸载 | 删除 `~/.claude/skills/fan-files.md` |

用户也可以独立卸载 skill 但保留程序：`rm ~/.claude/skills/fan-files.md`
