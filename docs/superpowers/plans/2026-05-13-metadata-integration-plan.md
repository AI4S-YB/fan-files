# 元数据整合 + 项目查询 + 搜索增强 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 新增 `fan-files projects` 命令，LLM 结果回写文件级元数据，search 结果带项目信息。

**Architecture:** 新增 `projects` CLI 命令，在 `infer.rs` 管线末尾增加回写步骤，瘦身 `interpreter.rs`，增强 `search.rs` 输出。

---

## Task 1: `fan-files projects` 命令

**Files:**
- Create: `crates/fan-files/src/commands/projects.rs`
- Modify: `crates/fan-files/src/main.rs`
- Modify: `crates/fan-files/src/commands/mod.rs`

- [ ] **Step 1: 创建 projects.rs**

```rust
use fan_core::config::Config;
use fan_core::index::sqlite::SqliteStore;
use fan_core::project::ProjectStore;
use std::sync::Arc;

pub fn run(config: &Config, show_name: Option<&str>) {
    let data_dir = fan_core::config::dirs_fan().join("data");
    let sqlite = match SqliteStore::open(&data_dir) {
        Ok(s) => s,
        Err(e) => { eprintln!("Failed to open index: {}", e); return; }
    };
    let store = ProjectStore::new(Arc::clone(&sqlite.conn));

    match show_name {
        Some(name) => show(&store, name),
        None => list(&store),
    }
}

fn list(store: &ProjectStore) {
    match store.all() {
        Ok(projects) => {
            if projects.is_empty() {
                println!("No projects found. Run 'fan-files infer' first.");
                return;
            }
            for p in &projects {
                let species = p.species.as_deref().unwrap_or("?");
                let conf = p.species_confidence.as_deref().unwrap_or("?");
                let assay = p.assay_type.as_deref().unwrap_or("?");
                // Count files via project_file join
                let file_count = store.file_count(p.id).unwrap_or(0);
                println!(
                    "{:<25} {:<20} {:<15} ({})  {} files",
                    truncate(&p.name, 25),
                    assay,
                    format!("{} ({})", species, conf),
                    p.species_source.as_deref().unwrap_or("llm"),
                    file_count,
                );
            }
        }
        Err(e) => eprintln!("Error: {}", e),
    }
}

fn show(store: &ProjectStore, name: &str) {
    match store.get_by_name(name) {
        Ok(Some(p)) => {
            println!("Project: {}", p.name);
            if let Some(ref at) = p.assay_type { println!("  Assay:       {}", at); }
            println!(
                "  Species:     {} (confidence: {}, source: {})",
                p.species.as_deref().unwrap_or("?"),
                p.species_confidence.as_deref().unwrap_or("?"),
                p.species_source.as_deref().unwrap_or("llm"),
            );
            if let Some(ref dirs) = p.root_dirs {
                if let Ok(parsed) = serde_json::from_str::<Vec<String>>(dirs) {
                    println!("  Directories:");
                    for d in &parsed { println!("    {}", d); }
                }
            }
            println!("  Files:       {}", store.file_count(p.id).unwrap_or(0));
            if let Some(ref s) = p.summary { println!("  Summary:     {}", s); }

            // Show relations
            if let Ok(rels) = store.get_relations(p.id) {
                if !rels.is_empty() {
                    println!("  Relations:");
                    for (other_name, rel_type, score) in &rels {
                        println!("    → {} ({}, score: {:.1})", other_name, rel_type, score);
                    }
                }
            }
        }
        Ok(None) => eprintln!("Project '{}' not found.", name),
        Err(e) => eprintln!("Error: {}", e),
    }
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() > max { &s[..max-3] } else { s }
}
```

- [ ] **Step 2: 给 ProjectStore 添加辅助方法**

Read `crates/fan-core/src/project.rs`，添加：

```rust
pub fn file_count(&self, project_id: i64) -> rusqlite::Result<usize> {
    let conn = self.conn.lock().unwrap();
    conn.query_row(
        "SELECT COUNT(*) FROM project_file WHERE project_id=?1",
        params![project_id],
        |r| r.get::<_, i64>(0),
    )
    .map(|c| c as usize)
}

pub fn get_relations(&self, project_id: i64) -> rusqlite::Result<Vec<(String, String, f64)>> {
    let conn = self.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT CASE WHEN pr.project_a_id=?1 THEN p2.name ELSE p1.name END as other_name,
                pr.relation_type, pr.score
         FROM project_relation pr
         JOIN project p1 ON p1.id=pr.project_a_id
         JOIN project p2 ON p2.id=pr.project_b_id
         WHERE pr.project_a_id=?1 OR pr.project_b_id=?1"
    )?;
    let rows = stmt.query_map(params![project_id], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?))
    })?;
    rows.collect()
}
```

- [ ] **Step 3: 添加 CLI 命令到 main.rs**

Read `crates/fan-files/src/main.rs`，在 Commands enum 中添加：

```rust
    /// List or show LLM-inferred projects
    Projects {
        /// Show details for a specific project
        show: Option<String>,
    },
```

在 match 分支：

```rust
        Commands::Projects { show } => commands::projects::run(&config, show.as_deref()),
```

在 `crates/fan-files/src/commands/mod.rs` 添加 `pub mod projects;`

- [ ] **Step 4: Build & test**

```bash
cargo build && cargo test && ./target/debug/fan-files projects
```

- [ ] **Step 5: Commit**

```bash
git add -A && git commit -m "feat: add fan-files projects command (list + show)"
```

---

## Task 2: LLM 结果回写文件级元数据

**Files:**
- Modify: `crates/fan-core/src/infer.rs`

- [ ] **Step 1: 在 run_inference() 末尾添加回写逻辑**

Read `crates/fan-core/src/infer.rs`，在写入 project_relation 之后、`Ok(...)` 之前，添加：

```rust
    // 7. Back-sync LLM metadata to file-level bio_metadata
    let mut files_updated = 0;
    for proj in &output.projects {
        if let Some(&proj_id) = project_name_to_id.get(&proj.name) {
            let assay_val = proj.assay_type.clone().unwrap_or_default();
            let species_val = proj.species.clone().unwrap_or_default();
            for dir in &proj.dirs {
                for (file_id, file_path, _) in &all_files {
                    if file_path.starts_with(dir) {
                        // Read existing metadata
                        if let Ok(Some(mut entry)) = sqlite.get_by_id(*file_id) {
                            let mut meta = entry.bio_metadata.unwrap_or_default();
                            if !assay_val.is_empty() {
                                meta.assay_type = Some(assay_val.clone());
                            }
                            if !species_val.is_empty() {
                                meta.species = Some(species_val.clone());
                            }
                            meta.project = Some(proj.name.clone());
                            if let Err(e) = sqlite.update_bio_metadata(*file_id, &meta) {
                                warn!("Failed to back-sync metadata for {}: {}", file_path, e);
                            } else {
                                files_updated += 1;
                            }
                        }
                    }
                }
            }
        }
    }
    info!("Back-synced LLM metadata to {} files", files_updated);
```

需要添加 `use fan_plugin_sdk::BioMetadata;` 到 infer.rs 的 imports。

- [ ] **Step 2: Build & test**

```bash
cargo build && cargo test
```

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat: back-sync LLM project metadata to file-level bio_metadata_json"
```

---

## Task 3: 内置解释器瘦身

**Files:**
- Modify: `crates/fan-core/src/interpreter.rs`
- Modify: `crates/fan-core/tests/interpreter_test.rs`

- [ ] **Step 1: 移除物种/实验类型推断，只保留标签**

读取所有解释器的 `extract()` 方法，移除 `meta.assay_type = ...` 和 `meta.species = ...` 的赋值。只保留 `meta.tags` 和 `meta.project`。

具体改动：
- `FastqInterpreter::extract()` — 移除 `assay_type` / `species` 设置，保留 `tags: ["paired-end"/"single-end"]`
- `BamInterpreter::extract()` — 同理
- `VcfInterpreter::extract()` — 同理
- `AnnotationInterpreter::extract()` — 保留 `assay_type = "annotation"`，移除 species
- `GenericInterpreter::extract()` — 只保留 project 提取，移除 assay_type/species

- [ ] **Step 2: 更新测试断言**

移除测试中对 `assay_type` / `species` 的断言，只验证 tags。例如 `test_fastq_rnaseq_detection` 改为：

```rust
assert!(meta.tags.contains(&"paired-end".to_string()));
// 不再断言 assay_type == "RNA-seq" 和 species == "human"
```

- [ ] **Step 3: Build & test**

```bash
cargo build && cargo test
```

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "refactor: slim down built-in interpreter to only do format detection + tags"
```

---

## Task 4: search 结果带项目信息

**Files:**
- Modify: `crates/fan-files/src/commands/search.rs`

- [ ] **Step 1: 在 search.rs 中 JOIN 项目信息**

Read `crates/fan-files/src/commands/search.rs`，在构建 SearchResult 时添加项目信息查询。

在构建 results 的循环中，对每个 entry.path 查 project_file 表找所属项目：

```rust
// After building SearchResult, enrich with project info
let project_info = get_project_for_path(&index.sqlite, &entry.path);
if let Some(proj_name) = project_info {
    r.summary = format!("{} [project: {}]", r.summary, proj_name);
}
```

添加辅助函数：

```rust
fn get_project_for_path(sqlite: &SqliteStore, file_path: &PathBuf) -> Option<String> {
    use rusqlite::params;
    let conn = sqlite.conn.lock().unwrap();
    let mut stmt = conn.prepare(
        "SELECT p.name FROM project p
         JOIN project_file pf ON p.id = pf.project_id
         JOIN files f ON f.id = pf.file_id
         WHERE f.path = ?1 LIMIT 1"
    ).ok()?;
    stmt.query_row(params![file_path.to_string_lossy().to_string()], |row| row.get(0)).ok()
}
```

需要在 search.rs 中添加 `use fan_core::index::sqlite::SqliteStore;` 和 `use std::path::PathBuf;`。

- [ ] **Step 2: Build & test**

```bash
cargo build && cargo test && ./target/debug/fan-files search "RNA-seq"
```

- [ ] **Step 3: Commit**

```bash
git add -A && git commit -m "feat: enrich search results with project name from project_file table"
```

---

## Task 5: Release & 端到端验证

- [ ] **Step 1: 清理旧数据并重跑推理**

```bash
sqlite3 ~/.fan-files/data/index.db "DELETE FROM project_file; DELETE FROM project_relation; DELETE FROM project;"
./target/debug/fan-files infer
```

- [ ] **Step 2: 验证 projects 命令**

```bash
./target/debug/fan-files projects
./target/debug/fan-files projects show apple_rnaseq_test
```

- [ ] **Step 3: 验证回写**

```bash
sqlite3 ~/.fan-files/data/index.db "SELECT COUNT(*) FROM files WHERE bio_metadata_json IS NOT NULL AND bio_metadata_json != '' AND deleted=0;"
```

预期：从 48 变为接近 287。

- [ ] **Step 4: 验证搜索**

```bash
./target/debug/fan-files search "RNA-seq"
```

预期：结果带 `[project: xxx]`。

- [ ] **Step 5: Build release, run tests, commit & push**

```bash
cargo build --release && cargo test && git add -A && git commit -m "chore: finalize metadata integration with tests and release build" && git push
```

All tests must pass. Binary to `/usr/local/bin/fan-files`.
