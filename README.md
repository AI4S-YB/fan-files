# Fan-Files

智能文件元数据检索引擎——让 AI 助手拥有全服务器数据视野。

## 是什么

部署在生物信息学服务器上的后台服务。自动扫描所有文件，通过 LLM 推断生物学业务上下文（实验类型、物种、数据项目），建立多层索引。支持多用户（公有 + 私有索引）、多服务器远程扫描、实时文件监控。

**核心场景：**
- "服务器上有什么数据？"
- "有没有水稻的 RNA-seq 数据？"
- "这些文件来自哪台服务器？"
- "列出所有参考基因组项目"

## 安装

### 一键安装（推荐）

```bash
curl -fsSL https://raw.githubusercontent.com/AI4S-YB/fan-files/main/install.sh | bash
```

### 从 GitHub Releases 下载预编译二进制

```bash
# macOS (Apple Silicon)
curl -fsSL https://github.com/AI4S-YB/fan-files/releases/latest/download/fan-files-aarch64-apple-darwin.tar.gz | tar -xz

# Linux (x86_64)
curl -fsSL https://github.com/AI4S-YB/fan-files/releases/latest/download/fan-files-x86_64-unknown-linux-gnu.tar.gz | tar -xz
```

## 初始配置

### 用户模式（私有索引）

```bash
fan-files init
```

交互式向导：选择扫描目录 → 添加远程服务器 → 选择 LLM → 输入 API Key → 开始扫描。

索引保存在 `~/.fan-files/`，仅用户自己可访问。

### 管理员模式（全局公有索引）

```bash
sudo fan-files init --global
```

配置公有数据目录，所有用户可查询。索引保存在 `/var/lib/fan-files/`。

## 使用

### 搜索与查询

```bash
fan-files search "rice RNA-seq"          # 自然语言搜索（私有 + 公有）
fan-files projects                        # 列出 LLM 推断的数据项目
fan-files projects <name>                 # 查看项目详情
fan-files projects update <name> --species "Oryza sativa" --confidence high
fan-files info /path/to/file.bam          # 文件详情（含来源服务器）
fan-files suggest /data/projects/xxx      # 数据推荐
fan-files status                          # 索引状态 + 元数据覆盖率 + per-server 统计
fan-files list                            # 列出所有文件
fan-files list --server dev-server        # 按来源服务器过滤
```

### 服务器管理

```bash
fan-files servers list                    # 列出已注册服务器
fan-files servers add <name>              # 交互式添加服务器（支持多扫描路径）
fan-files servers remove <name>           # 移除服务器
fan-files servers scan <name>             # 扫描单台服务器（缓存优先）
fan-files servers scan --agent <name>     # 使用 fan-agent 远程扫描
fan-files servers watch <name>            # 实时监控远程服务器文件变化
```

### 管理员命令

```bash
fan-files --global init                   # 初始化全局索引
fan-files --global daemon                 # 启动全局守护进程
fan-files --global status                 # 查看全局索引状态
fan-files --global search "genome"        # 搜索全局索引
```

## 全部命令

```
fan-files init               交互式配置向导
fan-files daemon             启动守护进程（扫描 + 监控 + 自动 infer）
fan-files infer              手动触发 LLM 元数据推断（分批处理）
fan-files search             自然语言搜索
fan-files projects           列出/查看/更新数据项目
fan-files pending            查看待完善元数据清单
fan-files list               按类型/标签/服务器列出文件
fan-files info               文件详情（含来源服务器）
fan-files suggest            数据推荐
fan-files status             索引状态 + per-server 统计
fan-files servers            服务器注册管理子命令
fan-files update             升级到最新版本（从 GitHub Releases 下载）
fan-files uninstall          卸载（可选保留数据）
```

## 架构

```
┌─ Mac mini / 服务器 ────────────────────────────┐
│  fan-files                                       │
│  ├── SQLite + Tantivy 本地索引                   │
│  ├── 多用户：公有层 + 私有层                     │
│  ├── 远程扫描：SSH + 缓存 + fan-agent            │
│  └── 搜索：自然语言 + 语义 + 全文                │
│         │                                         │
│         │ SSH / fan-agent                        │
│         ▼                                         │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐      │
│  │ dev-srv  │  │ ai-srv   │  │ gpu-h100 │ ...  │
│  │ 本地扫描 │  │ 本地扫描 │  │ 本地扫描 │      │
│  └──────────┘  └──────────┘  └──────────┘      │
└─────────────────────────────────────────────────┘
```

## 升级

```bash
fan-files update
```

自动检测平台、从 GitHub Releases 下载最新二进制、原地替换。启动时自动检查版本更新（24h 缓存）。

## 卸载

```bash
fan-files uninstall
```

| 选项 | 效果 |
|------|------|
| 仅卸载程序 | 删除二进制 + 源码 + skill，保留 `~/.fan-files/` |
| 完全卸载 | 删除上述全部 + `~/.fan-files/` + `/var/lib/fan-files/` |

## Claude Code 集成

安装脚本自动将 SKILL.md 安装到 `~/.claude/skills/`。重启 Claude Code 后，fan-files skill 自动激活。

也可通过 FAN Marketplace 安装：

```bash
fan install fan-files
```

## 技术栈

Rust · SQLite · Tantivy · Candle (ONNX) · ureq (rustls) · walkdir · notify · clap

## 协议

MIT
