# LLM 驱动的元数据推断管线

## 概述

在全量文件扫描完成后，将目录结构和代表性文件发送给 LLM（OpenAI 兼容接口），由 LLM 自主决定如何将目录合并为数据项目、推断实验类型和物种、推荐用于 BLAST 物种鉴定的序列文件。

## 流程

```
daemon 全量扫描完成
    │
    ├── 1. 从 SQLite 汇总目录树 + 代表性文件 → 压缩为"目录摘要"文本
    │
    ├── 2. 目录摘要 + System Prompt → LLM API
    │     如果目录数超过上下文窗口 → 分批发送，每批保留项目间关联的上下文
    │
    ├── 3. LLM 返回 JSON: 项目列表、每项目包含哪些目录、assay_type、species、
    │     项目间 relations
    │
    ├── 4. 对每个 species_confidence 为 low/medium 的项目 → 自动选择序列文件 → 
    │     提取前 500bp → BOLD API → 物种确认
    │
    └── 5. 写入 project / project_file / project_relation 表
```

## 新增表结构

```sql
-- LLM 推断出的数据项目
CREATE TABLE project (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    assay_type TEXT,
    species TEXT,
    species_confidence TEXT,   -- "high" / "medium" / "low"
    species_source TEXT,       -- "llm" / "bold_api"
    root_dirs TEXT,            -- JSON: ["dir1", "dir2"]
    summary TEXT,
    created_at INTEGER,
    updated_at INTEGER
);

-- 项目与文件的关联
CREATE TABLE project_file (
    project_id INTEGER REFERENCES project(id),
    file_id INTEGER REFERENCES files(id),
    PRIMARY KEY (project_id, file_id)
);

-- 项目间的关联
CREATE TABLE project_relation (
    project_a_id INTEGER REFERENCES project(id),
    project_b_id INTEGER REFERENCES project(id),
    relation_type TEXT,
    score REAL,
    reason TEXT,
    PRIMARY KEY (project_a_id, project_b_id, relation_type)
);
```

## LLM 接口

### 配置

```toml
# ~/.fan-files/config.toml
[llm]
endpoint = "https://api.openai.com/v1/chat/completions"
api_key = "sk-..."
model = "gpt-4o-mini"
# 或 DeepSeek: endpoint = "https://api.deepseek.com/v1/chat/completions"
```

### 输入：目录摘要

从 SQLite 提取，自动生成。每条目录包含：

- 路径、文件数
- 代表性文件名（最多 8 个，优先显示生信格式文件）
- 子目录（缩进嵌套，最多 3 层）

### 输出：JSON

```json
{
  "projects": [
    {
      "name": "SMT2024_genome",
      "dirs": ["test_data/genome/SMT2024/", "test_data/genome/smt2024_seq/"],
      "assay_type": "genome_annotation",
      "species": "未识别植物（SMT2024）",
      "species_confidence": "low",
      "summary": "SMT2024 物种的参考基因组及功能注释"
    },
    {
      "name": "apple_rnaseq",
      "dirs": ["test_data/fastq/apple_rnaseq_test/"],
      "assay_type": "RNA-seq",
      "species": "apple",
      "species_confidence": "high",
      "summary": "苹果 RNA-seq 表达数据，3个条件×2重复，双端测序"
    }
  ],
  "relations": [
    {
      "project_a": "SMT2024_genome",
      "project_b": "dataset01_genome",
      "relation": "可能为同一物种不同版本",
      "score": 0.7
    }
  ]
}
```

## BOLD 集成

LLM 返回项目列表后，对每个 species_confidence 为 "low" 或 "medium" 的项目：

1. 代码自动在项目目录下找最适合做物种鉴定的序列文件：
   - 优先选择 `*.fa.gz`、`*.fasta`、`*.fa`（基因组/转录组序列）
   - 优先选最大的文件（含最多序列信息）
   - 跳过功能注释文件（GO/KEGG 等 JSON）
2. 读取文件前 1000 字节
3. 如果 `.gz` → 解压读取
4. 提取第一个 `>` 之后的序列（FASTA），截取前 500bp
5. 调用 BOLD API 物种鉴定
6. 如果 BOLD 返回高置信度 → 更新 `project.species` 和 `species_source = "bold_api"`
7. BOLD 失败或低置信度 → 保留 LLM 推断，`species_source = "llm"`

## 触发方式

### daemon 自动触发

全量扫描完成后，自动执行 LLM 推理管线。

### 手动触发

```bash
fan-files infer                    # 重跑全部项目
fan-files infer --project SMT2024  # 只跑指定项目
fan-files infer --dry-run          # 只生成 prompt 不调 API
```

## 错误处理

- LLM API 调用失败 → 跳过该批次，继续下一批
- LLM 返回 JSON 解析失败 → 记录原始响应到日志，跳过
- BOLD API 调用失败 → 仅标记 `species_source = "llm"`，不阻塞
- 所有错误不影响文件扫描和基础索引功能
