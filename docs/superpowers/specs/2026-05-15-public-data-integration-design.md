# 公共数据集成 — 植物 SRA 元数据

## 概述

将清洗后的植物 SRA 元数据 SQLite 数据库（686 万条、8.2GB）通过 ATTACH 方式连接到 fan-files 索引。`fan-files search` 同时查询本地私有数据和公共 SRA 数据，合并返回。

## 配置

```toml
# ~/.fan-files/config.toml
[public_data]
db_path = "/path/to/plant_sra_search.db"
enabled = true
```

`enabled = false` 或不配置 `db_path` 时，公共数据搜索不生效。

## 架构

```
fan-files search "apple RNA-seq"
  │
  ├── 本地 (Tantivy + SQLite)
  │     source = "local"
  │
  ├── 公共 (ATTACH plant_sra_search.db AS public)
  │     source = "public"
  │     查询 sra_entries 表（organism_name, project_title, accession, bioproject）
  │
  └── 合并返回
```

## 搜索逻辑

1. 打开本地 SQLite
2. 如果配置了 `public_data.enabled`，`ATTACH DATABASE` 外部 SRA DB
3. 本地搜索走现有逻辑（Tantivy + Embedding + SQLite 降级）
4. 公共搜索用外部 DB 的 LIKE 或 FTS 查询 `organism_name` 和 `project_title`
5. 结果用 `source` 字段区分：`local` / `public`
6. 公共结果格式：accession, organism_name, project_title, bioproject, source="public"

## CLI 行为

- `fan-files search "apple"` → 自动合并本地+公共结果
- `fan-files public info PRJNAxxx` → 查询单条 SRA 记录详情（预留）
- `fan-files projects` → 只显示本地项目（不变）

## Skill 行为

搜索到公共数据时，Skill 在结果中标注 `[public]`，引导用户决定是否将公共数据和本地数据一起分析。

## 错误处理

- 外部 DB 文件不存在 → 只返回本地结果，不报错
- ATTACH 失败 → 只返回本地结果，记录 warning
- 公共数据查询超时（>5s）→ 只返回本地结果
