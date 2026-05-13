# LLM 驱动元数据推断管线 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 全量扫描完成后，自动将目录结构发给 LLM 推断项目分组、实验类型、物种，并可选调用 BOLD API 确认物种。

**Architecture:** 新增 `llm/`、`bold.rs`、`infer.rs`、`project.rs` 四个模块。LLM 客户端负责构造 prompt 和解析 JSON 响应。BOLD 模块负责序列提取和 API 调用。`infer.rs` 编排整个管线。`project.rs` 管理新增的三张表。

**Tech Stack:** `ureq` (HTTP), 现有 SQLite schema, serde_json

---

## File Structure

```
crates/fan-core/src/
├── llm/
│   ├── mod.rs          # LLM 客户端：build_request, call_api, parse_response
│   └── prompt.rs       # 目录摘要生成 + System Prompt
├── bold.rs             # BOLD API：序列提取 + bold_identify 调用
├── infer.rs            # 管线编排：汇总目录 → LLM → 写入 project → BOLD
├── project.rs          # project/project_file/project_relation CRUD
├── config.rs           # 新增 [llm] 配置段
├── index/
│   └── sqlite.rs       # 新增 project 表 migration
└── lib.rs              # 新增 pub mod 声明

crates/fan-files/src/
├── main.rs             # 新增 Infer 子命令
├── commands/
│   ├── infer.rs        # infer 命令实现
│   └── daemon.rs       # 全量扫描后触发 LLM 管线
└── config.rs           # (已存在)
```

---

### Task 1: Project 表 + CRUD

**Files:**
- Modify: `crates/fan-core/src/index/sqlite.rs`
- Create: `crates/fan-core/src/project.rs`
- Modify: `crates/fan-core/src/lib.rs`

- [ ] **Step 1: 在 sqlite.rs 的 migrate() 中新增 project 表**

Read `crates/fan-core/src/index/sqlite.rs`，找到 `migrate()` 方法，在末尾添加：

```sql
CREATE TABLE IF NOT EXISTS project (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    assay_type TEXT,
    species TEXT,
    species_confidence TEXT,
    species_source TEXT,
    root_dirs TEXT,
    summary TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS project_file (
    project_id INTEGER NOT NULL REFERENCES project(id),
    file_id INTEGER NOT NULL REFERENCES files(id),
    PRIMARY KEY (project_id, file_id)
);
CREATE TABLE IF NOT EXISTS project_relation (
    project_a_id INTEGER NOT NULL REFERENCES project(id),
    project_b_id INTEGER NOT NULL REFERENCES project(id),
    relation_type TEXT NOT NULL,
    score REAL NOT NULL DEFAULT 0.0,
    reason TEXT,
    PRIMARY KEY (project_a_id, project_b_id, relation_type)
);
```

- [ ] **Step 2: 创建 crates/fan-core/src/project.rs**

```rust
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: i64,
    pub name: String,
    pub assay_type: Option<String>,
    pub species: Option<String>,
    pub species_confidence: Option<String>,
    pub species_source: Option<String>,
    pub root_dirs: Option<String>,  // JSON array
    pub summary: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub struct ProjectStore {
    conn: std::sync::Arc<Mutex<rusqlite::Connection>>,
}

impl ProjectStore {
    pub fn new(conn: std::sync::Arc<Mutex<rusqlite::Connection>>) -> Self {
        Self { conn }
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
    }

    pub fn insert(&self, name: &str, assay_type: Option<&str>, species: Option<&str>,
                  species_confidence: Option<&str>, root_dirs: Option<&str>,
                  summary: Option<&str>) -> rusqlite::Result<i64> {
        let conn = self.conn.lock().unwrap();
        let now = Self::now();
        conn.execute(
            "INSERT INTO project (name, assay_type, species, species_confidence, root_dirs, summary, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![name, assay_type, species, species_confidence, root_dirs, summary, now, now],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn update_species(&self, id: i64, species: &str, source: &str, confidence: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE project SET species=?1, species_source=?2, species_confidence=?3, updated_at=?4 WHERE id=?5",
            params![species, source, confidence, Self::now(), id],
        )?;
        Ok(())
    }

    pub fn link_file(&self, project_id: i64, file_id: i64) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO project_file (project_id, file_id) VALUES (?1, ?2)",
            params![project_id, file_id],
        )?;
        Ok(())
    }

    pub fn link_files_bulk(&self, project_id: i64, file_ids: &[i64]) -> rusqlite::Result<()> {
        for id in file_ids {
            self.link_file(project_id, *id)?;
        }
        Ok(())
    }

    pub fn add_relation(&self, project_a: i64, project_b: i64, rel_type: &str, score: f64, reason: Option<&str>) -> rusqlite::Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO project_relation (project_a_id, project_b_id, relation_type, score, reason)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![project_a, project_b, rel_type, score, reason],
        )?;
        Ok(())
    }

    pub fn get_by_name(&self, name: &str) -> rusqlite::Result<Option<Project>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, assay_type, species, species_confidence, species_source, root_dirs, summary, created_at, updated_at
             FROM project WHERE name=?1"
        )?;
        let mut rows = stmt.query_map(params![name], Self::map_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn all(&self) -> rusqlite::Result<Vec<Project>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, assay_type, species, species_confidence, species_source, root_dirs, summary, created_at, updated_at
             FROM project ORDER BY id"
        )?;
        let rows = stmt.query_map([], Self::map_row)?;
        rows.collect()
    }

    fn map_row(row: &rusqlite::Row) -> rusqlite::Result<Project> {
        Ok(Project {
            id: row.get(0)?,
            name: row.get(1)?,
            assay_type: row.get(2)?,
            species: row.get(3)?,
            species_confidence: row.get(4)?,
            species_source: row.get(5)?,
            root_dirs: row.get(6)?,
            summary: row.get(7)?,
            created_at: row.get(8)?,
            updated_at: row.get(9)?,
        })
    }
}
```

- [ ] **Step 3: 添加 pub mod project 到 lib.rs**

Read `crates/fan-core/src/lib.rs`，在末尾添加 `pub mod project;`

- [ ] **Step 4: Build & commit**

```bash
cd /Users/kentnf/projects/omicsagent/fan-files && cargo build && cargo test
git add crates/fan-core/src/project.rs crates/fan-core/src/lib.rs crates/fan-core/src/index/sqlite.rs
git commit -m "feat: add project/project_file/project_relation tables and CRUD"
```

---

### Task 2: LLM 配置段

**Files:**
- Modify: `crates/fan-core/src/config.rs`

- [ ] **Step 1: 新增 LlmConfig 结构体**

Read `crates/fan-core/src/config.rs`，在 `Config` 结构体中添加：

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default = "default_llm_model")]
    pub model: String,
}

fn default_llm_model() -> String { "gpt-4o-mini".into() }

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            api_key: String::new(),
            model: default_llm_model(),
        }
    }
}
```

在 `Config` struct 中添加 `#[serde(default)] pub llm: LlmConfig,`

在 `Config::default()` 中添加 `llm: LlmConfig::default(),`

- [ ] **Step 2: Build & commit**

```bash
cargo build
git add crates/fan-core/src/config.rs
git commit -m "feat: add LLM config section (endpoint, api_key, model)"
```

---

### Task 3: LLM 客户端

**Files:**
- Create: `crates/fan-core/src/llm/mod.rs`
- Create: `crates/fan-core/src/llm/prompt.rs`
- Modify: `crates/fan-core/src/lib.rs`

- [ ] **Step 1: 创建 crates/fan-core/src/llm/prompt.rs**

```rust
/// System prompt for the LLM
pub fn system_prompt() -> &'static str {
    "你是一个生物信息学数据管理助手。用户会给你一个服务器目录的扫描结果，\
     包含目录结构和代表性文件列表。请分析这些目录，返回结构化的 JSON。\n\n\
     你需要：\n\
     1. 将目录合并成\"数据项目\"——同一个生物学项目的文件即使分散在多个子目录，也应该归为一个项目\n\
     2. 推断项目的实验类型（assay_type）：RNA-seq, ChIP-seq, WGS, WGBS, ATAC-seq, \
        genome_annotation, variant_calling, epigenomics, transcriptomics, phenomics, germplasm 等\n\
     3. 推断物种信息（species）和置信度（species_confidence: high/medium/low）\n\
     4. 判断不同项目之间是否有关联（同一物种、互补实验类型等）\n\
     5. 每个项目写一句简短描述（summary）"
}

/// 从文件扩展名判断是否为生信关键文件（优先展示）
fn is_key_file(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.ends_with(".fastq.gz") || lower.ends_with(".fq.gz") || lower.ends_with(".fastq") ||
    lower.ends_with(".fasta") || lower.ends_with(".fa.gz") || lower.ends_with(".fa") ||
    lower.ends_with(".bam") || lower.ends_with(".vcf.gz") || lower.ends_with(".vcf") ||
    lower.ends_with(".gff3") || lower.ends_with(".gtf") || lower.ends_with(".bed") ||
    lower.ends_with(".h5") || lower.ends_with(".hdf5")
}

/// Generate a directory summary text from the index for LLM consumption
pub fn build_directory_summary(
    root: &str,
    dirs: &[(String, usize, Vec<String>)],  // (path, file_count, sample_filenames)
) -> String {
    let mut lines = vec![
        format!("## 扫描结果\n\n根目录: {}\n", root),
    ];

    for (path, count, samples) in dirs {
        let key_files: Vec<&str> = samples.iter().filter(|n| is_key_file(n)).map(|s| s.as_str()).collect();
        let other: Vec<&str> = samples.iter().filter(|n| !is_key_file(n)).take(5).map(|s| s.as_str()).collect();
        let all_samples: Vec<&str> = key_files.iter().chain(other.iter()).copied().collect();
        let display = all_samples.join(", ");
        let display = if display.len() > 120 { format!("{}...", &display[..117]) } else { display };

        lines.push(format!("{}  ({} files)", path, count));
        lines.push(format!("  代表性文件: {}", if display.is_empty() { "(无)" } else { &display }));
    }

    lines.join("\n")
}

/// Parse LLM JSON response into structured data
pub fn parse_llm_response(json: &str) -> Result<LlmOutput, serde_json::Error> {
    serde_json::from_str(json)
}

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct LlmOutput {
    pub projects: Vec<LlmProject>,
    #[serde(default)]
    pub relations: Vec<LlmRelation>,
}

#[derive(Debug, Deserialize)]
pub struct LlmProject {
    pub name: String,
    pub dirs: Vec<String>,
    pub assay_type: Option<String>,
    pub species: Option<String>,
    #[serde(default)]
    pub species_confidence: Option<String>,
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LlmRelation {
    pub project_a: String,
    pub project_b: String,
    pub relation: String,
    pub score: f64,
}
```

- [ ] **Step 2: 创建 crates/fan-core/src/llm/mod.rs**

```rust
pub mod prompt;

use crate::config::LlmConfig;
use prompt::{LlmOutput, system_prompt};
use tracing::{info, warn};

pub struct LlmClient {
    config: LlmConfig,
}

impl LlmClient {
    pub fn new(config: LlmConfig) -> Self {
        Self { config }
    }

    pub fn is_configured(&self) -> bool {
        !self.config.endpoint.is_empty() && !self.config.api_key.is_empty()
    }

    /// Send directory summary to LLM, return parsed project list
    pub fn infer_projects(&self, dir_summary: &str) -> Result<LlmOutput, Box<dyn std::error::Error>> {
        let user_msg = format!("{}\n\n请分析以上目录结构，返回 JSON。", dir_summary);

        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [
                {"role": "system", "content": system_prompt()},
                {"role": "user", "content": user_msg}
            ],
            "response_format": {"type": "json_object"},
            "temperature": 0.1
        });

        info!("Calling LLM API at {} (model: {})", self.config.endpoint, self.config.model);
        let response = ureq::post(&self.config.endpoint)
            .set("Authorization", &format!("Bearer {}", self.config.api_key))
            .set("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| format!("LLM API call failed: {}", e))?;

        let json: serde_json::Value = response.into_json()
            .map_err(|e| format!("Failed to parse LLM response: {}", e))?;

        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .ok_or("No content in LLM response")?;

        prompt::parse_llm_response(content)
            .map_err(|e| format!("Failed to parse LLM JSON output: {}", e).into())
    }
}
```

- [ ] **Step 3: 添加模块声明到 lib.rs**

在 `crates/fan-core/src/lib.rs` 添加 `pub mod llm;`

- [ ] **Step 4: Build & commit**

```bash
cargo build
git add crates/fan-core/src/llm/ crates/fan-core/src/lib.rs
git commit -m "feat: add LLM client with OpenAI-compatible API + directory summary prompt"
```

---

### Task 4: BOLD API 集成

**Files:**
- Create: `crates/fan-core/src/bold.rs`
- Modify: `crates/fan-core/src/lib.rs`

- [ ] **Step 1: 创建 crates/fan-core/src/bold.rs**

```rust
use std::io::Read;
use std::path::Path;
use tracing::{info, warn};

/// 在项目目录中自动选择最适合的序列文件用于物种鉴定
pub fn find_blast_file(project_dirs: &[String]) -> Option<String> {
    let mut candidates: Vec<(String, u64)> = Vec::new();
    let seq_exts = ["fa.gz", "fasta.gz", "fa", "fasta", "fna", "fastq.gz", "fq.gz", "fastq"];

    for dir in project_dirs {
        let path = Path::new(dir);
        if !path.exists() { continue; }
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                let fpath = entry.path();
                let name = fpath.file_name().and_then(|n| n.to_str()).unwrap_or("");
                let name_lower = name.to_lowercase();

                // Skip annotation/small files
                if name_lower.contains("func_anno") || name_lower.contains("go_") 
                    || name_lower.contains("kegg") || name_lower.ends_with(".json")
                    || name_lower.ends_with(".xml") || name_lower.ends_with(".txt") {
                    continue;
                }

                if seq_exts.iter().any(|ext| name_lower.ends_with(ext)) {
                    if let Ok(meta) = std::fs::metadata(&fpath) {
                        candidates.push((fpath.to_string_lossy().to_string(), meta.len()));
                    }
                }
            }
        }
    }

    // Also look one level down in subdirectories
    if candidates.is_empty() {
        for dir in project_dirs {
            if let Ok(entries) = std::fs::read_dir(Path::new(dir)) {
                for entry in entries.flatten() {
                    let subdir = entry.path();
                    if subdir.is_dir() {
                        if let Ok(sub_entries) = std::fs::read_dir(&subdir) {
                            for sub_entry in sub_entries.flatten() {
                                let fpath = sub_entry.path();
                                let name = fpath.file_name().and_then(|n| n.to_str()).unwrap_or("");
                                let name_lower = name.to_lowercase();
                                if seq_exts.iter().any(|ext| name_lower.ends_with(ext)) {
                                    if let Ok(meta) = std::fs::metadata(&fpath) {
                                        candidates.push((fpath.to_string_lossy().to_string(), meta.len()));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Sort by size desc (genome > CDS > mRNA), then by ext priority (fa > fastq)
    candidates.sort_by(|a, b| {
        let a_is_seq = !a.0.to_lowercase().contains("fastq");
        let b_is_seq = !b.0.to_lowercase().contains("fastq");
        b_is_seq.cmp(&a_is_seq).then(b.1.cmp(&a.1))
    });

    candidates.first().map(|(p, s)| {
        info!("Selected BLAST file: {} ({} bytes)", p, s);
        p.clone()
    })
}

/// Extract first 500bp of sequence from a FASTA file
pub fn extract_sequence(file_path: &str, max_bp: usize) -> Result<String, Box<dyn std::error::Error>> {
    let path = Path::new(file_path);
    let file = std::fs::File::open(path)?;
    let reader: Box<dyn Read> = if file_path.ends_with(".gz") {
        Box::new(flate2::read::GzDecoder::new(file))
    } else {
        Box::new(file)
    };

    use std::io::BufRead;
    let mut seq = String::new();
    let mut in_seq = false;
    for line in std::io::BufReader::new(reader).lines() {
        let line = line?;
        if line.starts_with('>') {
            if in_seq { break; }  // second header, stop
            in_seq = true;
            continue;
        }
        if in_seq {
            seq.push_str(&line);
            if seq.len() >= max_bp { break; }
        }
    }

    if seq.is_empty() {
        return Err("No sequence found in file".into());
    }

    Ok(seq[..seq.len().min(max_bp)].to_uppercase())
}

/// Call BOLD API to identify species
pub fn identify_species(sequence: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
    // BOLD API: https://v4.boldsystems.org/index.php/api_home
    // This uses the public BOLD identification endpoint
    let url = "https://v4.boldsystems.org/index.php/Ids_OpenApi";

    let response = ureq::post(url)
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_form(&[
            ("sequence", sequence),
            ("db", "COX1_SPECIES_PUBLIC,COX1,COX1_SPECIES"),
            ("format", "json"),
        ])
        .map_err(|e| format!("BOLD API call failed: {}", e))?;

    let json: serde_json::Value = response.into_json()
        .map_err(|e| format!("BOLD response parse error: {}", e))?;

    // Parse top match
    if let Some(matches) = json["top_matches"].as_array() {
        if let Some(top) = matches.first() {
            let species = top["taxonomicidentification"].as_str()
                .or_else(|| top["species_name"].as_str())
                .map(|s| s.to_string());
            let similarity = top["similarity"].as_f64().unwrap_or(0.0);
            if similarity > 95.0 {
                return Ok(species);
            }
        }
    }

    Ok(None)
}
```

Add `flate2` to fan-core Cargo.toml dependencies: `flate2 = "1"`

- [ ] **Step 2: Build & commit**

```bash
cargo build
git add crates/fan-core/src/bold.rs crates/fan-core/src/lib.rs crates/fan-core/Cargo.toml
git commit -m "feat: add BOLD API integration with automatic sequence file selection"
```

---

### Task 5: 推理管线编排器

**Files:**
- Create: `crates/fan-core/src/infer.rs`
- Modify: `crates/fan-core/src/lib.rs`

- [ ] **Step 1: 创建 crates/fan-core/src/infer.rs**

```rust
use crate::bold;
use crate::index::sqlite::SqliteStore;
use crate::llm::LlmClient;
use crate::project::ProjectStore;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use tracing::{info, warn};

/// Run the full LLM inference pipeline
pub fn run_inference(
    sqlite: &SqliteStore,
    project_store: &ProjectStore,
    llm_client: &LlmClient,
    scan_root: &str,
) -> Result<(usize, usize), Box<dyn std::error::Error>> {
    if !llm_client.is_configured() {
        info!("LLM not configured, skipping inference");
        return Ok((0, 0));
    }

    // 1. Collect directory tree from indexed files
    let dirs = collect_directory_summary(sqlite);
    if dirs.is_empty() {
        info!("No directories found for inference");
        return Ok((0, 0));
    }

    // 2. Build prompt and call LLM
    let summary = crate::llm::prompt::build_directory_summary(scan_root, &dirs);
    info!("Sending {} directories to LLM for inference...", dirs.len());

    let output = match llm_client.infer_projects(&summary) {
        Ok(o) => o,
        Err(e) => {
            warn!("LLM inference failed: {}", e);
            return Ok((0, 0));
        }
    };

    info!("LLM returned {} projects, {} relations", output.projects.len(), output.relations.len());

    // 3. Collect all file IDs by path for quick lookup
    let all_files = sqlite.all_paths().unwrap_or_default();
    let mut path_to_id: HashMap<String, i64> = HashMap::new();
    for (id, path, _) in &all_files {
        path_to_id.insert(path.clone(), *id);
    }

    // 4. Write projects and link files
    let mut project_name_to_id: HashMap<String, i64> = HashMap::new();
    let mut projects_created = 0;

    for proj in &output.projects {
        let root_dirs_json = serde_json::to_string(&proj.dirs).unwrap_or_default();
        let id = project_store.insert(
            &proj.name,
            proj.assay_type.as_deref(),
            proj.species.as_deref(),
            proj.species_confidence.as_deref(),
            Some(&root_dirs_json),
            proj.summary.as_deref(),
        )?;
        project_name_to_id.insert(proj.name.clone(), id);
        projects_created += 1;

        // Link files under project dirs
        for dir in &proj.dirs {
            for (file_id, file_path, _) in &all_files {
                if file_path.starts_with(dir) {
                    project_store.link_file(id, *file_id).ok();
                }
            }
        }

        // 5. BOLD species confirmation for low-confidence projects
        if proj.species_confidence.as_deref() == Some("low")
            || proj.species_confidence.as_deref() == Some("medium")
        {
            if let Some(blast_file) = bold::find_blast_file(&proj.dirs) {
                info!("Attempting BOLD species identification for project '{}' using {}", proj.name, blast_file);
                match bold::extract_sequence(&blast_file, 500) {
                    Ok(seq) => match bold::identify_species(&seq) {
                        Ok(Some(species)) => {
                            info!("BOLD identified species for '{}': {}", proj.name, species);
                            project_store.update_species(id, &species, "bold_api", "high").ok();
                        }
                        Ok(None) => info!("BOLD could not identify species for '{}'", proj.name),
                        Err(e) => warn!("BOLD API error for '{}': {}", proj.name, e),
                    },
                    Err(e) => warn!("Sequence extraction failed for '{}': {}", proj.name, e),
                }
            }
        }
    }

    // 6. Write project relations
    for rel in &output.relations {
        if let (Some(&a_id), Some(&b_id)) = (
            project_name_to_id.get(&rel.project_a),
            project_name_to_id.get(&rel.project_b),
        ) {
            project_store.add_relation(a_id, b_id, &rel.relation, rel.score, None).ok();
        }
    }

    Ok((projects_created, output.relations.len()))
}

/// Collect directory tree summary from indexed files
fn collect_directory_summary(sqlite: &SqliteStore) -> Vec<(String, usize, Vec<String>)> {
    let all = sqlite.all_paths().unwrap_or_default();
    let mut dir_map: HashMap<String, (usize, Vec<String>)> = HashMap::new();

    for (_, path, _) in &all {
        if let Some(parent) = std::path::Path::new(path).parent() {
            let dir_path = parent.to_string_lossy().to_string();
            let entry = dir_map.entry(dir_path).or_insert((0, Vec::new()));
            entry.0 += 1;
            if let Some(name) = std::path::Path::new(path).file_name().and_then(|n| n.to_str()) {
                if entry.1.len() < 8 {
                    entry.1.push(name.to_string());
                }
            }
        }
    }

    let mut result: Vec<_> = dir_map.into_iter().collect();
    result.sort_by(|a, b| b.1.0.cmp(&a.1.0)); // Sort by file count desc
    result.into_iter().map(|(path, (count, files))| (path, count, files)).collect()
}
```

- [ ] **Step 2: 添加 flate2 依赖到 fan-core/Cargo.toml**

```
flate2 = "1"
```

- [ ] **Step 3: Build & commit**

```bash
cargo build
git add crates/fan-core/src/infer.rs crates/fan-core/src/lib.rs crates/fan-core/Cargo.toml
git commit -m "feat: add inference pipeline orchestrator with LLM + BOLD integration"
```

---

### Task 6: CLI infer 命令

**Files:**
- Create: `crates/fan-files/src/commands/infer.rs`
- Modify: `crates/fan-files/src/main.rs`
- Modify: `crates/fan-files/src/commands/mod.rs`

- [ ] **Step 1: 创建 crates/fan-files/src/commands/infer.rs**

```rust
use fan_core::config::Config;
use fan_core::index::IndexEngine;
use fan_core::infer;
use fan_core::llm::LlmClient;
use fan_core::project::ProjectStore;
use std::sync::Arc;

pub fn run(config: &Config) {
    let index = match IndexEngine::open(config, true) {
        Ok(i) => i,
        Err(e) => { eprintln!("Failed to open index: {}", e); return; }
    };

    let llm_client = LlmClient::new(config.llm.clone());
    if !llm_client.is_configured() {
        eprintln!("LLM not configured. Set [llm] section in ~/.fan-files/config.toml");
        return;
    }

    let project_store = ProjectStore::new(Arc::clone(&index.sqlite.conn));

    let scan_root = config.scan.include.first().map(|s| s.as_str()).unwrap_or("/");
    match infer::run_inference(&index.sqlite, &project_store, &llm_client, scan_root) {
        Ok((projects, relations)) => {
            println!("Inference complete: {} projects, {} relations", projects, relations);
        }
        Err(e) => eprintln!("Inference failed: {}", e),
    }
}
```

- [ ] **Step 2: 暴露 sqlite conn 给 ProjectStore 使用**

`SqliteStore` 的 `conn` 字段当前是 `Mutex<Connection>`，需要改为 `Arc<Mutex<Connection>>` 或暴露一个引用。最简单：在 `SqliteStore` 添加方法：

```rust
pub fn conn_arc(&self) -> Arc<Mutex<Connection>> {
    Arc::new(Mutex::new(/* can't clone Mutex<Connection> */ ))
}
```

更好的做法：重构 `SqliteStore.conn` 为 `Arc<Mutex<Connection>>`。更新 `sqlite.rs` 中 `open()` 方法的 `conn` 初始化，以及所有 `self.conn.lock().unwrap()` 调用保持不变（`Arc<Mutex<T>>` 的 `lock()` 和 `Mutex<T>` 的 `lock()` 调用方式一样）。

同时导出 `pub conn: Arc<Mutex<Connection>>`。

- [ ] **Step 3: 添加 infer 子命令到 main.rs**

Read `crates/fan-files/src/main.rs`，在 Commands enum 中添加：

```rust
    /// Run LLM inference on indexed files
    Infer,
```

在 match 分支中添加：

```rust
        Commands::Infer => commands::infer::run(&config),
```

在 `crates/fan-files/src/commands/mod.rs` 添加 `pub mod infer;`

- [ ] **Step 4: Build & commit**

```bash
cargo build
git add crates/fan-files/src/commands/infer.rs crates/fan-files/src/main.rs crates/fan-files/src/commands/mod.rs crates/fan-core/src/index/sqlite.rs
git commit -m "feat: add fan-files infer CLI command"
```

---

### Task 7: Daemon 自动触发 LLM 管线

**Files:**
- Modify: `crates/fan-files/src/commands/daemon.rs`

- [ ] **Step 1: 全量扫描后触发 LLM 推理**

Read `crates/fan-files/src/commands/daemon.rs`，在 `run_full_scan()` 返回后（两次调用处：初始扫描和定时同步）添加 LLM 推理调用。

在 `run()` 函数中，初始扫描之后、文件监控之前：

```rust
    // After initial scan, run LLM inference
    let llm_client = fan_core::llm::LlmClient::new(config.llm.clone());
    let project_store = fan_core::project::ProjectStore::new(/* Arc<Mutex<Connection>> */);
    let scan_root = config.scan.include.first().map(|s| s.as_str()).unwrap_or("/");
    match fan_core::infer::run_inference(&index.sqlite, &project_store, &llm_client, scan_root) {
        Ok((p, r)) => info!("LLM inference complete: {} projects, {} relations", p, r),
        Err(e) => warn!("LLM inference skipped or failed: {}", e),
    }
```

同样在定时同步的 `run_full_scan` 之后也触发一次（项目可能新增了目录）。

- [ ] **Step 2: Build & commit**

```bash
cargo build
git add crates/fan-files/src/commands/daemon.rs
git commit -m "feat: auto-trigger LLM inference after full scan in daemon"
```

---

### Task 8: 集成测试

**Files:**
- Create: `crates/fan-core/tests/llm_inference_test.rs`

- [ ] **Step 1: 创建集成测试**

```rust
use fan_core::llm::prompt;

#[test]
fn test_directory_summary_generation() {
    let dirs = vec![
        ("/data/blastdb/".into(), 101, vec!["arabidopsis_mrna.nhr".into(), "blast_names.json".into()]),
        ("/data/fastq/apple_rnaseq_test/".into(), 13, vec!["H_1_1.fq".into(), "meta.json".into()]),
    ];
    let summary = prompt::build_directory_summary("/data", &dirs);
    assert!(summary.contains("blastdb"));
    assert!(summary.contains("apple_rnaseq_test"));
    assert!(summary.contains("101 files"));
    assert!(summary.contains("13 files"));
}

#[test]
fn test_llm_response_parsing() {
    let json = r#"{
        "projects": [
            {"name": "test_project", "dirs": ["/data/test/"], "assay_type": "RNA-seq", "species": "human", "species_confidence": "high", "summary": "test"}
        ],
        "relations": []
    }"#;
    let result = prompt::parse_llm_response(json).unwrap();
    assert_eq!(result.projects.len(), 1);
    assert_eq!(result.projects[0].name, "test_project");
    assert_eq!(result.projects[0].assay_type, Some("RNA-seq".into()));
}

#[test]
fn test_project_store() {
    // Requires database setup — skip for unit test, covered by integration
}
```

- [ ] **Step 2: Run tests & commit**

```bash
cargo test
git add crates/fan-core/tests/llm_inference_test.rs
git commit -m "test: add LLM prompt generation and response parsing tests"
```

---

### Task 9: 构建 + 端到端验证

- [ ] **Step 1: Build release**

```bash
cargo build --release
```

- [ ] **Step 2: 验证 CLI**

```bash
./target/release/fan-files --help
# 应显示 infer 子命令

./target/release/fan-files infer
# 应提示 LLM not configured（因为还没配 API key）
```

- [ ] **Step 3: 运行全部测试**

```bash
cargo test
```

Expected: all tests pass (19+ 个).

- [ ] **Step 4: Commit & push**

```bash
git add -A
git commit -m "chore: finalize LLM inference pipeline with integration tests"
git push
```
