# fan-files 元数据整合 + 项目查询 + 搜索增强

## 概述

三项改进：(1) 新增 `fan-files projects` 命令查看 LLM 推断结果；(2) LLM 推断结果回写到文件级元数据，替代内置解释器的物种/实验类型判断；(3) search 结果附带项目信息。

## 1. `fan-files projects` 命令

### 子命令

```
fan-files projects              # 列出所有项目
fan-files projects show <name>  # 查看单个项目详情
```

### 输出

`fan-files projects` 列表：

```
  SMT2024_genome        genome_annotation   SMT2024 (high)    78 files
  apple_rnaseq_test     RNA-seq             apple (high)      13 files
  blastdb               genome_annotation   Rose/Arabidopsis  101 files
  dataset01_genome      genome_annotation   unknown (low)     47 files
  epigenomics           WGBS                unknown (low)     12 files
  ...
```

`fan-files projects show SMT2024_genome` 详情：

```
Project: SMT2024_genome
  Assay:       genome_annotation
  Species:     SMT2024 (confidence: high, source: llm)
  Directories: /data/test_data/genome/SMT2024/
               /data/test_data/genome/SMT2024/func_anno/
               /data/test_data/genome/SMT2024/func_anno/itak/
  Files:       78
  Summary:     Genome assembly and functional annotation for SMT2024.
  Relations:
    → dataset01_genome (similar_assay, score: 0.7)
```

## 2. LLM 结果回写文件级元数据

### 改动点

`infer.rs` 中 `run_inference()` 完成后，对每个 project：

1. 遍历 project 下的所有文件
2. 用 project 的 `species` / `assay_type` 更新 `files.bio_metadata_json`
3. 不覆盖已有的 `tags`（如 `paired-end`），只在 `species` / `assay_type` / `project` 字段上覆盖

### 内置解释器改动

`interpreter.rs` 中各解释器不再推断 `species` 和 `assay_type`，只做：
- 格式检测（FASTQ → tags: ["paired-end", "single-end"]）
- 结构推断（目录里有 design.csv → tags: ["has_metadata_file"]）

物种和实验类型的判断完全交给 LLM。

## 3. search 结果带项目信息

### 改动点

`search.rs` 中，对每个搜索结果：

1. 查出该文件所属的 project（通过 `project_file` 表 JOIN）
2. 在 `SearchResult.summary` 中追加 `[project: SMT2024_genome]`
3. 搜 `RNA-seq` 时，在 Tantivy 查询中额外搜索 `metadata` 字段，匹配到 project 的 `assay_type` 字段

### 示例

```
$ fan-files search "RNA-seq"

0.95  /data/test_data/fastq/apple_rnaseq_test/H_1_1.fq  RNA-seq  apple
      [project: apple_rnaseq_test] Apple RNA-seq fastq files
0.87  /data/test_data/transcriptome/01.phytohormones_RNAseq.h5  transcriptomics  unknown
      [project: transcriptome] Phytohormone RNA-seq expression data
```
