# 公共数据集成 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** `fan-files search` 自动 ATTACH 外部植物 SRA 数据库，合并本地和公共搜索结果。

**Architecture:** config.rs 新增 `public_data` 段，search.rs 中 ATTACH 外部 DB 并用 UNION 查询，结果带 `source` 字段区分。

---

## Task 1: 配置 + ATTACH 基础设施

**Files:**
- Modify: `crates/fan-core/src/config.rs`
- Modify: `crates/fan-core/src/index/sqlite.rs`

- [ ] **Step 1: 在 config.rs 添加 PublicDataConfig**

Read `crates/fan-core/src/config.rs`，在 `Config` struct 和 `LlmConfig` 之间添加：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicDataConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub db_path: String,
}

impl Default for PublicDataConfig {
    fn default() -> Self {
        Self { enabled: false, db_path: String::new() }
    }
}
```

在 `Config` struct 添加 `#[serde(default)] pub public_data: PublicDataConfig,`

在 `Config::default()` 添加 `public_data: PublicDataConfig::default(),`

- [ ] **Step 2: 在 SqliteStore 添加 ATTACH 和公共搜索方法**

Read `crates/fan-core/src/index/sqlite.rs`，添加：

```rust
/// Attach external public database and search it
pub fn search_public(
    &self,
    db_path: &str,
    query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<(String, String, String)>> {
    let conn = self.conn.lock().unwrap();
    // ATTACH if not already attached
    conn.execute_batch(&format!("ATTACH DATABASE '{}' AS public_sra", db_path))?;
    
    let pattern = format!("%{}%", query);
    let mut stmt = conn.prepare(
        "SELECT accession, organism_name, project_title
         FROM public_sra.sra_entries
         WHERE organism_name LIKE ?1 OR project_title LIKE ?1
         LIMIT ?2"
    )?;
    let rows = stmt.query_map(params![pattern, limit as i64], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    rows.collect()
}
```

- [ ] **Step 3: Build & commit**

```bash
cargo build && cargo test && git add -A && git commit -m "feat: add public data config + ATTACH search infrastructure"
```

---

## Task 2: search 命令集成公共数据

**Files:**
- Modify: `crates/fan-files/src/commands/search.rs`

- [ ] **Step 1: 在 search.rs 尾部添加公共搜索逻辑**

Read `crates/fan-files/src/commands/search.rs`，在 `cosine_similarity` 函数之后添加：

```rust
fn search_public(config: &Config, query: &str) -> Vec<SearchResult> {
    let public_cfg = &config.public_data;
    if !public_cfg.enabled || public_cfg.db_path.is_empty() {
        return vec![];
    }

    let data_dir = fan_core::config::dirs_fan().join("data");
    let sqlite = match fan_core::index::sqlite::SqliteStore::open(&data_dir) {
        Ok(s) => s,
        Err(_) => return vec![],
    };

    match sqlite.search_public(&public_cfg.db_path, query, 20) {
        Ok(rows) => rows.into_iter().map(|(acc, org, title)| {
            SearchResult {
                path: format!("[public] {}", acc),
                score: 1.0,
                file_type: Some("SRA".into()),
                assay_type: None,
                species: Some(org),
                tags: vec![],
                summary: title,
                source: DataSource::Public { origin: "plant_sra".into() },
            }
        }).collect(),
        Err(_) => vec![],
    }
}
```

在 `run()` 函数中，构建完本地 results 后，追加公共结果：

```rust
    // 5. Search public data
    let public_results = search_public(config, query);
    results.extend(public_results);
    
    // Re-sort by score
    results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
```

- [ ] **Step 2: Build & test**

```bash
cargo build && cargo test
```

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat: merge public SRA data into search results"
```

---

## Task 3: 端到端验证

- [ ] **Step 1: 配置并测试**

```bash
# 编辑 ~/.fan-files/config.toml，添加：
# [public_data]
# enabled = true
# db_path = "/Users/kentnf/projects/data/plant-sra-metadata/plant_sra_search.db"

cargo build --release
./target/release/fan-files search "Oryza sativa"
./target/release/fan-files search "apple"
```

预期：每个搜索同时返回本地和公共结果。

- [ ] **Step 2: 测试无公共数据库时不报错**

```bash
# 注释掉或删除 public_data 配置后
./target/release/fan-files search "apple"
```

预期：只返本地结果，不报错。

- [ ] **Step 3: Commit & push**

```bash
cargo test && git add -A && git commit -m "chore: public data integration with end-to-end verification" && git push
```
