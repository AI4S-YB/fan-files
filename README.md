# Fan-Files

智能文件元数据检索引擎——让 Claude Code 拥有全服务器视野。

## 是什么

部署在生物信息学服务器上的后台服务。自动扫描服务器所有文件，推断生物学业务上下文（实验类型、物种、参考基因组等），建立多层索引。当用户用 Claude Code 分析数据时，能以自然语言检索文件、发现可协同分析的相关数据。

**核心场景：**
- "服务器上有没有肺癌的 RNA-seq 数据？"
- "我正在分析 RNA-seq，服务器上有匹配的 ChIP-seq 可以联合分析"
- "列出所有人类参考基因组"
- "这个 BAM 文件是什么实验类型的？"

## 安装

### 从源码编译

```bash
git clone git@github.com:AI4S-YB/fan-files.git
cd fan-files
cargo build --release
sudo cp target/release/fan-files /usr/local/bin/
```

### 初始化配置

```bash
mkdir -p ~/.fan-files
fan-files generate-skill  # 生成 Claude Code Skill（可选）
```

编辑 `~/.fan-files/config.toml`，设置扫描目录：

```toml
[scan]
include = ["/data"]
exclude = ["/tmp", "*.tmp"]

[watch]
include = ["/data"]
```

## 使用

### 启动守护进程

```bash
fan-files daemon
```

首次启动会全量扫描配置的目录，之后持续监控文件变化。

### 检索文件

```bash
# 自然语言搜索
fan-files search "人类的RNA-seq数据"

# 列出某类数据
fan-files list --category rnaseq

# 按标签筛选
fan-files list --tag human

# JSON 输出（供 Claude Code 调用）
fan-files search "参考基因组 hg38" --json
```

### 数据推荐

```bash
fan-files suggest /data/projects/your_project
```

### 查看文件详情

```bash
fan-files info /path/to/file.bam
```

### 查看索引状态

```bash
fan-files status
```

### 配合 Claude Code 使用

将生成的 `skill/fan-files.md` 放到 Claude Code 的 skills 目录，Claude 会自动在分析数据时调用上述命令。

## 技术栈

Rust · SQLite · Tantivy · fastembed (ONNX) · wasmtime · notify

## 协议

MIT
