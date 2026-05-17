# 交互式元数据完善 — 实现计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task.

**Goal:** infer 完成后自动生成待完善清单，Claude Code 以选择题方式引导用户补全项目元数据。

**Architecture:** 新增 `pending` CLI 命令 + `projects update` 子命令。pending_review.json 存储待完善清单。infer.rs 末尾自动生成候选选项。Skill 文件增加启动检查逻辑。

---

## Task 1: pending_review.json 读写模块

**Files:**
- Create: `crates/fan-core/src/review.rs`
- Modify: `crates/fan-core/src/lib.rs`

- [ ] **Step 1: 创建 crates/fan-core/src/review.rs**

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingItem {
    pub project: String,
    pub field: String,
    pub current_value: Option<String>,
    pub confidence: Option<String>,
    pub candidates: Vec<String>,
    pub timestamp: i64,
}

pub struct ReviewStore {
    path: PathBuf,
}

impl ReviewStore {
    pub fn new() -> Self {
        Self {
            path: crate::config::dirs_fan().join("pending_review.json"),
        }
    }

    pub fn load(&self) -> Result<Vec<PendingItem>, Box<dyn std::error::Error>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let data = std::fs::read_to_string(&self.path)?;
        Ok(serde_json::from_str(&data)?)
    }

    pub fn save(&self, items: &[PendingItem]) -> Result<(), Box<dyn std::error::Error>> {
        let data = serde_json::to_string_pretty(items)?;
        std::fs::write(&self.path, data)?;
        Ok(())
    }

    pub fn clear(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.save(&[])
    }

    fn now() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64
    }
}
```

- [ ] **Step 2: 添加 pub mod review 到 lib.rs**

Read `crates/fan-core/src/lib.rs`，添加 `pub mod review;`

- [ ] **Step 3: 在 infer.rs 末尾生成 pending items**

Read `crates/fan-core/src/infer.rs`，在 `run_inference()` 的 `Ok(...)` 返回前添加：

```rust
    // 8. Generate pending review items for low/medium confidence projects
    let mut pending_items: Vec<crate::review::PendingItem> = Vec::new();
    for proj in &output.projects {
        let needs_review = proj.species_confidence.as_deref() == Some("low")
            || proj.species_confidence.as_deref() == Some("medium");
        if needs_review {
            let candidates = generate_candidates(llm_client, proj);
            pending_items.push(crate::review::PendingItem {
                project: proj.name.clone(),
                field: "species".into(),
                current_value: proj.species.clone(),
                confidence: proj.species_confidence.clone(),
                candidates,
                timestamp: crate::review::ReviewStore::now(),
            });
        }
    }
    if !pending_items.is_empty() {
        let store = crate::review::ReviewStore::new();
        store.save(&pending_items)?;
        info!("Saved {} pending review items to {}", pending_items.len(), store.path.display());
    }
```

Add helper function:

```rust
fn generate_candidates(llm_client: &LlmClient, proj: &crate::llm::prompt::LlmProject) -> Vec<String> {
    if !llm_client.is_configured() {
        return vec!["unknown_species".into()];
    }
    let prompt = format!(
        "数据项目 '{}' 的物种未知。根据项目名和上下文，\
         列出最可能的 4 个物种名，以逗号分隔，只返回物种名。\n\
         项目名: {}\n描述: {}",
        proj.name,
        proj.name,
        proj.summary.as_deref().unwrap_or("")
    );
    match llm_client.infer_candidates(&prompt) {
        Ok(candidates) => candidates,
        Err(_) => vec!["unknown_species".into()],
    }
}
```

- [ ] **Step 4: 在 LlmClient 添加 infer_candidates 方法**

Read `crates/fan-core/src/llm/mod.rs`，添加：

```rust
    /// Simple LLM call that returns a list of candidate strings
    pub fn infer_candidates(&self, prompt: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let body = serde_json::json!({
            "model": self.config.model,
            "messages": [
                {"role": "user", "content": prompt}
            ],
            "temperature": 0.3,
            "max_tokens": 50
        });
        let response = ureq::post(&self.config.endpoint)
            .set("Authorization", &format!("Bearer {}", self.config.api_key))
            .set("Content-Type", "application/json")
            .send_json(&body)
            .map_err(|e| format!("LLM API call failed: {}", e))?;
        let json: serde_json::Value = response.into_json()?;
        let content = json["choices"][0]["message"]["content"]
            .as_str().ok_or("No content")?;
        Ok(content.split(',').map(|s| s.trim().to_string()).collect())
    }
```

- [ ] **Step 5: Build & test**

```bash
cargo build && cargo test
```

All tests must pass.

- [ ] **Step 6: Commit**

```bash
git add -A && git commit -m "feat: add pending_review.json store + LLM candidate generation in infer"
```

---

## Task 2: `fan-files pending` + `fan-files projects update` 命令

**Files:**
- Modify: `crates/fan-files/src/main.rs`
- Modify: `crates/fan-files/src/commands/mod.rs`
- Modify: `crates/fan-files/src/commands/projects.rs`

- [ ] **Step 1: 在 projects.rs 添加 update 和 pending 支持**

Read `crates/fan-files/src/commands/projects.rs`，在 `run()` 函数添加 `update` 处理，新增 `pending` 入口。

更新 `run()` 的签名和处理逻辑以支持子命令。

- [ ] **Step 2: 添加 Pending 和 Projects 子命令到 main.rs**

Read `crates/fan-files/src/main.rs`，添加：

```rust
    /// Show or clear pending review items
    Pending {
        #[command(subcommand)]
        action: Option<PendingAction>,
    },
```

和 `PendingAction` enum：

```rust
#[derive(Subcommand)]
enum PendingAction {
    /// Clear all pending review items
    Clear,
}
```

Projects 子命令添加 Update：

```rust
    Projects {
        #[command(subcommand)]
        action: Option<ProjectAction>,
    },
#[derive(Subcommand)]
enum ProjectAction {
    /// Show project details
    Show { name: String },
    /// Update project metadata
    Update {
        name: String,
        #[arg(long)]
        species: Option<String>,
        #[arg(long, value_name = "high|medium|low")]
        confidence: Option<String>,
        #[arg(long)]
        assay_type: Option<String>,
        #[arg(long)]
        candidates: Option<String>,
    },
}
```

- [ ] **Step 3: Build & test**

```bash
cargo build && cargo test
./target/debug/fan-files pending
./target/debug/fan-files projects update --help
```

- [ ] **Step 4: Commit**

```bash
git add -A && git commit -m "feat: add fan-files pending + projects update commands"
```

---

## Task 3: Skill 文件更新 + 端到端验证

**Files:**
- Modify: `skills/fan-files.md`
- Modify: `CLAUDE.md`

- [ ] **Step 1: 更新 skills/fan-files.md**

在 skill 文件末尾添加：

```markdown
## 元数据完善

### 启动时检查
每次对话开始时，运行：
```
fan-files pending --json
```
如果返回非空数组，主动告知用户：
"有 N 个项目的信息需要完善。要现在处理吗？"

### 交互流程
用户同意后，逐个展示：
1. 项目名和不确定字段
2. 候选选项（A/B/C/D）
3. 用户选择或输入
4. 调用 `fan-files projects update <name> --species "xxx" --confidence high`

### 完成后
```
fan-files pending clear
```
```

- [ ] **Step 2: 更新 CLAUDE.md**

在 CLAUDE.md 添加类似指引。

- [ ] **Step 3: Release build + test + commit**

```bash
cargo build --release && cargo test && \
./target/release/fan-files pending && \
git add -A && git commit -m "chore: update skill for interactive metadata review" && \
git push
```
