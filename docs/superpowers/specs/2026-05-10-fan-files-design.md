# Fan-Files: 智能文件元数据检索引擎

## 概述

Fan-Files 是一个部署在服务器上的文件元数据管理和检索引擎。它扫描服务器上的所有文件，提取文件元数据和生物学业务上下文，通过三层检索（语义、全文、结构化）让 Claude Code 等 AI 工具能够用自然语言找到正确的数据文件，并主动向用户推荐可协同分析的相关数据。

**核心价值：让 Claude Code 具备超越当前工作目录的"全服务器视野"。**

## 部署形态

- 单二进制文件 `fan-files`
- Linux: systemd service，开机自启
- macOS: LaunchAgent
- 配置: `~/.fan-files/config.toml`
- 数据: `~/.fan-files/data/`（SQLite + Tantivy 索引 + 向量）
- 插件: `~/.fan-files/plugins/*.wasm`

## 架构

```
┌─────────────────────────────────────────────────┐
│            fan-files 单二进制                     │
│                                                   │
│  ┌─────────────────────────────────────────────┐ │
│  │            Core Engine                      │ │
│  │  ┌──────────┐ ┌──────────┐ ┌────────────┐  │ │
│  │  │ Scanner  │ │ Watcher  │ │ Scheduler  │  │ │
│  │  │(walkdir) │ │(inotify/ │ │ (定期任务)  │  │ │
│  │  │          │ │ FSEvents)│ │            │  │ │
│  │  └────┬─────┘ └────┬─────┘ └─────┬──────┘  │ │
│  │       └─────────────┼─────────────┘         │ │
│  │                     ▼                       │ │
│  │            ┌────────────────┐               │ │
│  │            │  Index Engine  │               │ │
│  │            │ SQLite+Tantivy │               │ │
│  │            │    + ONNX      │               │ │
│  │            └───────┬────────┘               │ │
│  └────────────────────┼───────────────────────┘ │
│                       │                          │
│  ┌────────────────────┼───────────────────────┐ │
│  │            Plugin Engine                   │ │
│  │  Layer 1: Format Detector (文件格式识别)    │ │
│  │  Layer 2: Context Interpreter (业务推断)    │ │
│  │  *.wasm 动态加载                           │ │
│  └────────────────────┬───────────────────────┘ │
│                       │                          │
│  ┌────────────────────┼───────────────────────┐ │
│  │              CLI Interface                  │ │
│  │    fan-files search/suggest/list/...       │ │
│  │    输出 JSON，供 Claude Code 直接调用       │ │
│  └─────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────┘
```

## 组件设计

### 1. Scanner（文件扫描器）

**职责：** 遍历目录树，发现文件，提取基础元数据。

- 递归遍历配置的 include 目录
- 收集: path, size, mtime, hash (SHA256), magic bytes (前 512 字节), MIME type
- 过滤: 跳过 exclude 规则匹配的文件和目录
- 增量扫描: 对比 mtime，跳过未变化的文件

### 2. Watcher（文件变更监控）

**职责：** 实时监控文件变更，增量更新索引。

三层机制：
- **Layer 1: 实时事件** — Linux inotify / macOS FSEvents，1 秒去重窗口
- **Layer 2: 变更队列** — 默认 5 秒或积满 100 个事件后批量处理
- **Layer 3: 增量更新** — 新文件走完整扫描管线，修改文件重新解析，删除文件软删除保留 30 天

兜底：每日凌晨 3 点全量对账，修复遗漏。

### 3. Plugin Engine（插件引擎）

双层设计，解决"同一个 BAM 文件可能是不同组学数据"的问题。

#### Layer 1: Format Detector（文件级）

识别文件的物理格式。内置支持 FASTQ, FASTA, BAM, CRAM, VCF, BED, GFF, HDF5, CSV/TSV 等。

```rust
trait FormatDetector {
    /// 返回优先级 (0-100)，越大越优先
    fn priority() -> u8 { 0 }
    fn can_handle(path: &str, magic: &[u8]) -> bool;
    fn detect(path: &str) -> Result<FormatInfo>;
}

struct BioMetadata {
    assay_type: Option<String>,       // "RNA-seq", "ChIP-seq" ...
    species: Option<String>,          // "human", "mouse" ...
    tissue: Option<String>,
    genome_build: Option<String>,     // "hg38", "mm10" ...
    project: Option<String>,
    extra: HashMap<String, String>,   // 扩展字段
}
```

#### Layer 2: Context Interpreter（上下文级）

基于文件内容和目录上下文推断生物学实验类型、物种、组织等信息。

```rust
struct FileContext {
    file: PathBuf,
    siblings: Vec<PathBuf>,        // 同目录文件列表
    directory_tree: Vec<PathBuf>,  // 向上 3 层目录结构
    metadata_files: Vec<PathBuf>,  // 附近的设计表/README/metadata.*
    file_header: Vec<u8>,          // 文件头部采样
    format_tags: Vec<String>,      // Layer 1 输出
}

trait ContextInterpreter {
    /// 判断此文件属于该 Interpreter 领域的置信度
    fn score(context: &FileContext) -> f64;
    
    /// 提取生物学元数据
    fn extract(context: &FileContext) -> Result<BioMetadata>;
}
```

多个 Interpreter 可对同一文件打分，取最高分。也可以组合，一个文件同时带有多种标签。

#### 插件格式

- **WASM** — 跨平台，同一文件 Linux/macOS 通用；沙箱安全。
- 放在 `~/.fan-files/plugins/`，启动时自动发现加载。
- 内置插件随二进制发布，社区可贡献新格式/新组学的检测插件。

### 4. Index Engine（检索引擎）

三层检索，各司其职：

| 层 | 引擎 | 存储 | 用途 |
|----|------|------|------|
| 结构化 | SQLite | files + metadata + tags + relations | 精确过滤（物种=human, 类型=RNA-seq） |
| 全文 | Tantivy | path + filename + tags + metadata_text | 关键词模糊搜索 |
| 语义 | ONNX (all-MiniLM-L6-v2) | 384 维向量存 SQLite BLOB | 自然语言语义搜索 |

向量不引入专门向量数据库。百万级文件，384 维向量，SQLite 存储，暴力搜索毫秒级。

### 5. CLI Interface

精简设计——不用 MCP Server，Claude Code 直接通过 bash 调用 CLI 命令获取 JSON 结果。

```
fan-files search "人类的RNA-seq数据" [--json]  → JSON 结果列表
fan-files suggest /data/projects/xxx [--json]  → 相关数据推荐
fan-files list --category rnaseq [--json]      → 按类别列出
fan-files info /path/to/file.bam [--json]      → 单文件完整元数据
fan-files status                                → 索引状态
fan-files daemon                                → 启动守护进程
```

### 6. Skill（操作手册）

Claude Code Skill 文件指导 Claude 何时调用哪些 CLI 命令：

- 做生信分析前 → `fan-files search` 发现数据
- 选参考基因组 → `fan-files list --category genome` 列出可用
- 开始数据分析 → `fan-files suggest <current_dir>` 找协同数据
- 需要更多上下文 → `fan-files info <path>` 获取详细元数据

### 7. 数据推荐引擎

比较两份数据在生物学元数据维度上的匹配程度。

计算流程：
1. 提取用户当前目录/项目的元数据
2. 用多维度加权匹配检索服务器上所有数据集
3. 维度权重: 物种 0.3, 组织 0.2, 项目归属 0.3, 实验类型互补 0.15, 参考基因组 0.05
4. 返回 Top-N 推荐 + 理由

实验类型互补矩阵可配置，如 RNA-seq → 推荐 ChIP-seq/ATAC-seq（表达+调控联合分析）。

### 8. 公共数据接口（预留）

```rust
trait PublicDataSource {
    async fn search(&self, query: &str, filters: &Filters) -> Result<Vec<FileMetadata>>;
    async fn get_metadata(&self, id: &str) -> Result<FileMetadata>;
}
```

配置文件预留 `[public_data]` 段，待公共数据系统设计完成后实现。

## 配置

```toml
# ~/.fan-files/config.toml

[daemon]
socket = "~/.fan-files/fan.sock"

[scan]
include = ["/data", "/home/shared"]
exclude = ["/tmp", "*.tmp"]

[watch]
include = ["/data", "/home/shared/genomes"]
exclude = ["/tmp", "/data/archive", "*.tmp", ".*"]

[embedding]
model = "all-MiniLM-L6-v2"   # 内置默认，可选 external API

[plugins]
dir = "~/.fan-files/plugins"

[public_data]   # 预留
# type = "rest"
# endpoint = "https://..."

[retention]
deleted_keep_days = 30

[schedule]
full_sync = "03:00"
```

## 技术选型

- **语言:** Rust
- **文件遍历:** `walkdir`
- **文件监控:** `notify` (跨平台 inotify/FSEvents 封装)
- **结构化存储:** `rusqlite`
- **全文索引:** `tantivy`
- **向量推理:** `ort` (ONNX Runtime) 或 `candle`
- **WASM 运行时:** `wasmtime`
- **序列化:** `serde` + `serde_json`
- **异步运行时:** `tokio`
