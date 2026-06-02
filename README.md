# Fan-Files

智能文件元数据检索引擎——让 Claude Code 拥有全服务器视野。

## 是什么

部署在生物信息学服务器上的后台服务。自动扫描服务器所有文件，通过 LLM 推断生物学业务上下文（实验类型、物种、数据项目），建立多层索引。Claude Code 能以自然语言检索文件、发现数据项目、找到可协同分析的相关数据。

**核心场景：**
- "服务器上有什么数据？"
- "有没有苹果的 RNA-seq 数据？"
- "SMT2024 项目的物种是什么？"
- "列出所有参考基因组"

## 安装

### 一键安装（推荐）

```bash
curl -fsSL https://raw.githubusercontent.com/AI4S-YB/fan-files/main/install.sh | bash
```

自动安装 Rust（如未装）、编译、部署到 `/usr/local/bin/fan-files`，并安装 Claude Code Skill。

### 初始配置

```bash
fan-files init
```

交互式向导：选择扫描目录 → 选择 LLM（DeepSeek/Qwen/GLM/ERNIE/OpenAI）→ 输入 API Key → 开始扫描。

完成后自动运行 `daemon`（扫描+监控）和 `infer`（LLM 元数据推断）。

## 使用

```bash
fan-files search "apple RNA-seq"      # 自然语言搜索（本地 + 公共库）
fan-files projects                    # 列出 LLM 推断的数据项目
fan-files projects SMT2024_genome     # 查看项目详情
fan-files projects update <name> --species "Apple" --confidence high  # 修正元数据
fan-files pending                     # 查看待完善项目
fan-files info /path/to/file.bam      # 文件详情
fan-files suggest /data/projects/xxx  # 数据推荐
fan-files status                      # 索引状态 + 元数据覆盖率
```

### 全部命令

```
fan-files init         交互式配置向导
fan-files daemon       启动守护进程（扫描 + 监控 + 自动 infer）
fan-files infer        手动触发 LLM 元数据推断
fan-files search       自然语言搜索
fan-files projects     列出/查看/更新数据项目
fan-files pending      查看待完善清单
fan-files list         按类型/标签列出文件
fan-files info         文件详情
fan-files suggest      数据推荐
fan-files status       索引状态 + 覆盖率
fan-files update       升级到最新版本
fan-files uninstall    卸载（可选保留数据）
```

## 升级

```bash
fan-files update
```

自动 git pull、重新编译、更新二进制和 Skill。

## 卸载

```bash
fan-files uninstall
```

| 选项 | 效果 |
|------|------|
| 仅卸载程序 | 删除二进制 + 源码 + skill，保留 `~/.fan-files/`（数据库、配置、模型） |
| 完全卸载 | 删除上述全部 + `~/.fan-files/` |

## Claude Code 集成

安装脚本自动将 SKILL.md 安装到 `~/.claude/skills/`。重启 Claude Code 后，fan-files skill 自动激活。

也可通过 FAN Marketplace 安装：

```bash
fan install fan-files
```

## 技术栈

Rust · SQLite · Tantivy · Candle (ONNX) · wasmtime · notify

## 协议

MIT
