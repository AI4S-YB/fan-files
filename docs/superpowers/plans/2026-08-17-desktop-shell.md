# fan-files 桌面壳 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 `feat/desktop-app` 分支上实现 Tauri 桌面壳：四页（首页仪表盘/数据集表格/搜索/设置）+ 托盘常驻，通过子进程 sidecar 复用现有 fan-files CLI 与 fan-files-share HTTP API。

**Architecture:** Tauri 2 应用（React+TS 前端，Rust 后端薄层）打包两个 sidecar 二进制（`fan-files`、`fan-files-share`）。GUI 启动时拉起 share（默认端口 17951，冲突换随机），扫描 = spawn `fan-files discover` 并流式读 stderr 做进度；浏览/搜索走 share 的本地 HTTP API。引擎侧唯一改动 = 给 share 新增 `/api/v1/search`（复用 Tantivy）。

**Tech Stack:** Rust（workspace 现有 crates + Tauri 2）、React 18 + TypeScript + Vite、Vitest、axum、Tantivy、GitHub Actions（三平台）。

**规格:** `docs/superpowers/specs/2026-08-17-desktop-shell-design.md`（9 项已确认决策、页面/架构/错误处理/测试策略，实施前必读）。

---

## Phase 0 — 引擎地基（在现有 workspace 内，纯 Rust）

### Task 1: share 加载 Tantivy 索引（AppState 扩展）

**Files:**
- Modify: `crates/fan-files-share/src/state.rs`
- Modify: `crates/fan-files-share/src/main.rs`

- [ ] **Step 1: 修改 AppState 结构**

`crates/fan-files-share/src/state.rs` 的 AppState 改为：

```rust
use crate::{
    config::Settings,
    db::Database,
    models::{Facets, Stats},
};
use fan_core::index::tantivy::TantivyIndex;
use std::{path::PathBuf, sync::Mutex, time::Instant};

pub struct Cache<T> {
    pub loaded: Instant,
    pub value: T,
}
pub struct AppState {
    pub db: Database,
    pub settings: Settings,
    /// Full-text index, shared data dir with the SQLite db
    /// (`<data_dir>/tantivy`). None when the index does not exist yet.
    pub tantivy: Option<TantivyIndex>,
    pub stats: Mutex<Option<Cache<Stats>>>,
    pub facets: Mutex<Option<Cache<Facets>>>,
}

impl AppState {
    pub fn new(settings: Settings) -> Result<Self, Box<dyn std::error::Error>> {
        let db = Database::open(&settings)?;
        // tantivy dir sits next to the sqlite database file
        let tantivy_dir: PathBuf = settings.database.parent().unwrap().join("tantivy");
        let tantivy = TantivyIndex::open(&tantivy_dir, true).ok();
        Ok(Self {
            db,
            settings,
            tantivy,
            stats: Mutex::new(None),
            facets: Mutex::new(None),
        })
    }
}
```

注意 `settings.database.parent()` 为 `Option`——`settings.database` 必为绝对路径，用 `.unwrap()` 前先确认 `Settings::load` 把它转成了绝对路径（读 `crates/fan-files-share/src/config.rs` 的 `Settings::load`；若不是，加一行 `std::fs::canonicalize` 或拼 `current_dir`）。

- [ ] **Step 2: 编译验证**

Run: `cargo check -p fan-files-share`
Expected: 无错误（`TantivyIndex::open(path, true)` 以只读打开，索引不存在时返回 Err → `None`，不影响启动）

- [ ] **Step 3: Commit**

```bash
git add crates/fan-files-share/src/state.rs crates/fan-files-share/src/main.rs
git commit -m "feat(share): open Tantivy index alongside SQLite db (optional)"
```

### Task 2: share 新增 `/api/v1/search` 端点（TDD）

**Files:**
- Modify: `crates/fan-files-share/src/db/mod.rs`（新增 `search_datasets`）
- Modify: `crates/fan-files-share/src/api/mod.rs`（新路由 + handler）
- Modify: `crates/fan-files-share/src/models.rs`（SearchQuery / DatasetSummary）

- [ ] **Step 1: 写失败测试（db 层 search_datasets）**

在 `crates/fan-files-share/src/db/mod.rs` 底部测试模块加（沿用文件内现有测试风格；若文件无测试模块则新建）：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn fixture() -> (tempfile::TempDir, rusqlite::Connection) {
        let dir = tempdir().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join("index.db")).unwrap();
        conn.execute_batch(
            "CREATE TABLE dataset (id INTEGER PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
             dataset_type TEXT, species TEXT, species_confidence TEXT, species_source TEXT,
             summary TEXT, indexed_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             CREATE TABLE files (id INTEGER PRIMARY KEY, path TEXT NOT NULL);
             CREATE TABLE asset (id INTEGER PRIMARY KEY, dataset_id INTEGER NOT NULL REFERENCES dataset(id),
             name TEXT, asset_type TEXT, indexed_at INTEGER NOT NULL);
             CREATE TABLE asset_file (asset_id INTEGER NOT NULL REFERENCES asset(id),
             file_id INTEGER NOT NULL REFERENCES files(id), role TEXT);",
        ).unwrap();
        (dir, conn)
    }

    #[test]
    fn search_datasets_maps_file_ids_to_datasets() {
        let (_dir, conn) = fixture();
        conn.execute("INSERT INTO dataset (id,name,path,dataset_type,species,indexed_at,updated_at)
                      VALUES (1,'Oryza_sativa_v1','/data/orders/Poales/Poaceae/Oryza_sativa/v1','genome','Oryza sativa',0,0)").unwrap();
        conn.execute("INSERT INTO files (id,path) VALUES (10,'/data/orders/Poales/Poaceae/Oryza_sativa/v1/genome.fa')").unwrap();
        conn.execute("INSERT INTO asset (id,dataset_id,name,asset_type,indexed_at) VALUES (1,1,'assembly','assembly',0)").unwrap();
        conn.execute("INSERT INTO asset_file (asset_id,file_id,role) VALUES (1,10,'primary')").unwrap();
        let db = Database { conn: std::sync::Mutex::new(conn) }; // 按现有 Database 字段构造
        let rows = db.search_datasets(&[10]).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name, "Oryza_sativa_v1");
        assert_eq!(rows[0].dataset_type.as_deref(), Some("genome"));
    }

    #[test]
    fn search_datasets_returns_empty_for_unknown_ids() {
        let (_dir, conn) = fixture();
        let db = Database { conn: std::sync::Mutex::new(conn) };
        assert!(db.search_datasets(&[999]).unwrap().is_empty());
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cargo test -p fan-files-share --lib search_datasets`
Expected: FAIL（`search_datasets` 不存在 / 编译错误）

- [ ] **Step 3: 实现 db 层 `search_datasets`**

在 `crates/fan-files-share/src/db/mod.rs` 的 `impl Database` 中加（按现有 `datasets()` 的返回结构复用一个轻量摘要结构，加到 `models.rs`）：

```rust
/// Map Tantivy-hit file ids to the datasets that contain those files,
/// deduplicated per dataset, with the file_count within that dataset.
pub fn search_datasets(&self, file_ids: &[i64]) -> Result<Vec<DatasetSummary>, AppError> {
    if file_ids.is_empty() {
        return Ok(vec![]);
    }
    let conn = self.conn.lock().map_err(|_| AppError::internal("db lock"))?;
    let placeholders = vec!["?"; file_ids.len()].join(",");
    let sql = format!(
        "SELECT d.id, d.name, d.dataset_type, d.species, d.path, COUNT(DISTINCT f.id) AS file_count
         FROM files f
         JOIN asset_file af ON af.file_id = f.id
         JOIN asset a ON a.id = af.asset_id
         JOIN dataset d ON d.id = a.dataset_id
         WHERE f.id IN ({placeholders})
         GROUP BY d.id
         ORDER BY file_count DESC"
    );
    let mut stmt = conn.prepare(&sql).map_err(AppError::db)?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(file_ids.iter()), |r| {
            Ok(DatasetSummary {
                id: r.get(0)?,
                name: r.get(1)?,
                dataset_type: r.get(2)?,
                species: r.get(3)?,
                path: r.get(4)?,
                file_count: r.get(5)?,
            })
        })
        .map_err(AppError::db)?
        .filter_map(|r| r.ok())
        .collect();
    Ok(rows)
}
```

`models.rs` 加：

```rust
#[derive(Debug, Serialize)]
pub struct DatasetSummary {
    pub id: i64,
    pub name: String,
    pub dataset_type: Option<String>,
    pub species: Option<String>,
    pub path: String,
    pub file_count: i64,
}
```

（按 `models.rs` 现有 serde 派生风格；`AppError::internal/db` 按 `error.rs` 现有构造器，没有就加两个小构造器。）

- [ ] **Step 4: 跑测试确认通过**

Run: `cargo test -p fan-files-share --lib search_datasets`
Expected: 2 passed

- [ ] **Step 5: 写失败测试（handler 层）**

`crates/fan-files-share/src/api/mod.rs` 测试模块（按现有 `accepts_relevance_only_with_query` 的测试搭建方式）：

```rust
#[tokio::test]
async fn search_endpoint_returns_datasets() {
    let settings = Settings::test_fixture(); // 若无此方法：用临时目录构造 Settings
    let state = AppState::new(settings.clone()).unwrap();
    // 塞 1 个 dataset + 1 个 file + 1 个 asset_file（直接 sqlite）
    {
        let conn = state.db.conn.lock().unwrap();
        conn.execute("INSERT INTO dataset (id,name,path,dataset_type,species,indexed_at,updated_at)
                      VALUES (1,'Oryza_sativa_v1','/data/orders/…/v1','genome','Oryza sativa',0,0)", []).unwrap();
        conn.execute("INSERT INTO files (id,path) VALUES (10,'/data/orders/…/v1/genome.fa')", []).unwrap();
        conn.execute("INSERT INTO asset (id,dataset_id,name,asset_type,indexed_at) VALUES (1,1,'assembly','assembly',0)", []).unwrap();
        conn.execute("INSERT INTO asset_file (asset_id,file_id,role) VALUES (1,10,'primary')", []).unwrap();
    }
    let app = router(Arc::new(state));
    let resp = app.oneshot(
        Request::builder().uri("/api/v1/search?q=genome").body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    // body 解析为 Envelope<Vec<DatasetSummary>>，断言 len == 1
}
```

若 AppState 没有 tantivy 索引（测试环境无索引）→ handler 应返回空数组而非报错——**这正是 handler 的第一条行为**：`state.tantivy` 为 None 时返回 `Ok(vec![])`。

- [ ] **Step 6: 跑测试确认失败**

Run: `cargo test -p fan-files-share --lib search_endpoint`
Expected: FAIL（无 `/api/v1/search` 路由）

- [ ] **Step 7: 实现 handler 与路由**

`api/mod.rs`：

```rust
use crate::db::Database;

async fn search(
    State(state): State<Arc<AppState>>,
    Query(q): Query<SearchQuery>,
) -> Result<Json<Envelope<Vec<DatasetSummary>>>, AppError> {
    let Some(tantivy) = &state.tantivy else {
        return Ok(Json(Envelope::ok(vec![])));
    };
    let q = q.q.trim().to_string();
    if q.is_empty() {
        return Err(AppError::bad_request("q is required"));
    }
    let hits = tantivy.search(&q, 200).map_err(|e| AppError::internal(&e.to_string()))?;
    let file_ids: Vec<i64> = hits.iter().map(|(id, _)| *id).collect();
    let datasets = state.db.search_datasets(&file_ids)?;
    Ok(Json(Envelope::ok(datasets)))
}
```

路由注册（`router()` 函数里）：

```rust
.route("/api/v1/search", get(search))
```

`models.rs` 加 `SearchQuery { pub q: String }`（serde Deserialize）。

- [ ] **Step 8: 跑测试确认通过**

Run: `cargo test -p fan-files-share --lib`
Expected: 全绿

- [ ] **Step 9: Commit**

```bash
git add crates/fan-files-share/src/
git commit -m "feat(share): add GET /api/v1/search (Tantivy full-text over datasets)"
```

### Task 3: sidecar 二进制与端口约定（GUI 依赖的地基验证）

**Files:**
- 无代码改动，验证 + 文档

- [ ] **Step 1: 验证 release 二进制与版本输出**

Run:
```bash
cargo build --release -p fan-files -p fan-files-share
ls target/release/fan-files target/release/fan-files-share
./target/release/fan-files --version   # 期望: fan-files 0.2.0
```
Expected: 两个二进制存在；版本输出稳定（GUI 设置页显示它）。

- [ ] **Step 2: 验证 share 端口绑定方式**

Read: `crates/fan-files-share/src/config.rs` 的 `Args`（bind 参数形式）。
Run: `./target/release/fan-files-share --help`
Expected: 存在 bind 地址参数（GUI 用它指定 17951 或随机端口；若没有 `--bind`，本任务补一个 `--bind <addr>` 参数，改动照 config.rs 的 Args 结构加字段）。

- [ ] **Step 3: Commit（若改了 Args）**

```bash
git add crates/fan-files-share/src/config.rs crates/fan-files-share/src/main.rs
git commit -m "feat(share): allow --bind address override for GUI embedding"
```

---

## Phase 1 — Tauri 脚手架

### Task 4: 创建 Tauri 应用骨架

**Files:**
- Create: `apps/desktop/`（Tauri 2 + React + TS + Vite）

- [ ] **Step 1: 脚手架**

Run:
```bash
mkdir -p apps/desktop && cd apps/desktop
npm create tauri-app@latest . -- --template react-ts --manager npm --yes
npm install
```
Expected: `apps/desktop/src/`（React 模板）、`apps/desktop/src-tauri/`（Rust 工程）生成。

- [ ] **Step 2: 装前端依赖**

Run: `npm install && npm install -D vitest @testing-library/react @testing-library/jest-dom jsdom`
Expected: package.json 增加 devDependencies。

- [ ] **Step 3: 最小可用验证**

Run: `npm run tauri dev`（Mac 上会弹出空窗口）
Expected: 窗口打开、模板页可见；`ctrl-c` 退出。

- [ ] **Step 4: Commit**

```bash
git add apps/desktop
git commit -m "chore(desktop): scaffold Tauri 2 + React/TS app"
```

### Task 5: Rust 后端命令层（config 读写 + 引擎状态）

**Files:**
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/Cargo.toml`（serde/serde_json/tauri-plugin-opener/tauri-plugin-shell? 按需）
- Test: `apps/desktop/src-tauri/src/lib.rs`（单元测试模块）

- [ ] **Step 1: 写失败测试（config 序列化）**

`lib.rs` 加测试模块：

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dto_roundtrip_keeps_fields() {
        let dto = FanConfig {
            threads: Some(10),
            include: vec!["/data/kentnf/orders".into()],
            exclude: vec!["*.tmp".into()],
            endpoint: "http://182.92.166.143:3200/v1/chat/completions".into(),
            api_key: "sk-test".into(),
            model: "DSv4-flash".into(),
        };
        let s = serde_json::to_string(&dto).unwrap();
        let back: FanConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(back.include, dto.include);
        assert_eq!(back.model, dto.model);
    }
}
```

- [ ] **Step 2: 跑测试确认失败**

Run: `cd apps/desktop/src-tauri && cargo test --lib`
Expected: FAIL（FanConfig 未定义）

- [ ] **Step 3: 实现 config DTO 与读写命令**

`lib.rs`：

```rust
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// GUI 视角的 ~/.fan-files/config.toml（与 CLI 共享同一文件）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FanConfig {
    pub threads: Option<usize>,
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
}

fn config_path() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".fan-files/config.toml")
}

#[tauri::command]
pub fn read_config() -> Result<FanConfig, String> {
    let raw = std::fs::read_to_string(config_path()).map_err(|e| e.to_string())?;
    let v: toml::Value = toml::from_str(&raw).map_err(|e| e.to_string())?;
    Ok(FanConfig {
        threads: v.get("threads").and_then(|t| t.as_integer()).map(|t| t as usize),
        include: v.get("scan").and_then(|s| s.get("include"))
            .and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        exclude: v.get("scan").and_then(|s| s.get("exclude"))
            .and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        endpoint: v.get("llm").and_then(|l| l.get("endpoint")).and_then(|x| x.as_str()).unwrap_or("").into(),
        api_key: v.get("llm").and_then(|l| l.get("api_key")).and_then(|x| x.as_str()).unwrap_or("").into(),
        model: v.get("llm").and_then(|l| l.get("model")).and_then(|x| x.as_str()).unwrap_or("").into(),
    })
}

#[tauri::command]
pub fn write_config(cfg: FanConfig) -> Result<(), String> {
    let v = toml::toml! {
        threads = cfg.threads.map(|t| t as i64)
        [scan]
        include = cfg.include
        exclude = cfg.exclude
        [llm]
        endpoint = cfg.endpoint
        api_key = cfg.api_key
        model = cfg.model
    };
    std::fs::create_dir_all(config_path().parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(config_path(), toml::to_string_pretty(&v).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn fan_home() -> Result<String, String> {
    Ok(config_path().parent().unwrap().to_string_lossy().to_string())
}
```

Cargo.toml 加：`toml = "0.8"`、`serde = { version = "1", features = ["derive"] }`、`serde_json = "1"`。

- [ ] **Step 4: 跑测试确认通过**

Run: `cd apps/desktop/src-tauri && cargo test --lib`
Expected: 1 passed

- [ ] **Step 5: 注册命令与 opener 插件**

`lib.rs` 的 `run()`（模板已有 `tauri::Builder`）：

```rust
tauri::Builder::default()
    .plugin(tauri_plugin_opener::init())
    .invoke_handler(tauri::generate_handler![read_config, write_config, fan_home])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
```

Cargo.toml 加 `tauri-plugin-opener = "2"`。

- [ ] **Step 6: Commit**

```bash
git add apps/desktop/src-tauri
git commit -m "feat(desktop): config read/write commands sharing CLI config.toml"
```

### Task 6: sidecar 打包配置

**Files:**
- Modify: `apps/desktop/src-tauri/tauri.conf.json`
- Create: `apps/desktop/src-tauri/build.rs`（若无则改模板现有）

- [ ] **Step 1: 声明 externalBin**

`tauri.conf.json` 加：

```json
"bundle": {
  "externalBin": [
    "binaries/fan-files",
    "binaries/fan-files-share"
  ]
}
```

（模板可能已有 `bundle` 节——合并而非新建；targets 保持三平台默认。）

- [ ] **Step 2: build.rs 复制 workspace 产物**

`build.rs`（src-tauri 下，模板已有则替换内容）：

```rust
fn main() {
    tauri_build::build();
    // copy workspace release binaries for sidecar bundling
    let out = std::path::Path::new("binaries");
    let _ = std::fs::create_dir_all(out);
    let workspace_target = std::path::Path::new("../../target/release");
    for bin in ["fan-files", "fan-files-share"] {
        let src = workspace_target.join(bin);
        let dst = out.join(bin);
        if src.exists() {
            let _ = std::fs::copy(&src, &dst);
        }
    }
}
```

- [ ] **Step 3: 验证打包含二进制**

Run: `npm run tauri build -- --debug`（先 debug 省时）
Expected: 产物 bundle 目录内出现 `fan-files` 与 `fan-files-share`。

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src-tauri
git commit -m "feat(desktop): bundle fan-files + share as sidecar binaries"
```

---

## Phase 2 — 前端四页 + 托盘

> 前端文件均位于 `apps/desktop/src/`。所有 fetch 走 `http://127.0.0.1:{port}/api/v1/...`，端口来自后端命令 `get_share_port`（Task 16 实现，先以常量 17951 占位，Task 16 替换）。

### Task 7: 应用骨架 + 侧边栏 + 页面路由

**Files:**
- Modify: `apps/desktop/src/App.tsx`
- Create: `apps/desktop/src/pages/HomePage.tsx` `DatasetsPage.tsx` `SearchPage.tsx` `SettingsPage.tsx`（先占位）
- Create: `apps/desktop/src/components/Sidebar.tsx`

- [ ] **Step 1: 写失败测试（侧边栏渲染与切换）**

`apps/desktop/src/App.test.tsx`：

```tsx
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect } from "vitest";
import App from "./App";

describe("App shell", () => {
  it("renders sidebar with four entries", () => {
    render(<App />);
    for (const label of ["首页", "数据集", "搜索", "设置"]) {
      expect(screen.getByText(label)).toBeInTheDocument();
    }
  });
  it("switches page on sidebar click", () => {
    render(<App />);
    fireEvent.click(screen.getByText("数据集"));
    expect(screen.getByText("数据集页面（占位）")).toBeInTheDocument();
  });
});
```

（`vitest` 配置：vite.config.ts 加 `test: { environment: "jsdom", setupFiles: "./src/setupTests.ts" }`；`setupTests.ts` 引入 `@testing-library/jest-dom`。）

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test`
Expected: FAIL

- [ ] **Step 3: 实现**

`Sidebar.tsx`：

```tsx
export type Page = "home" | "datasets" | "search" | "settings";
const items: { key: Page; icon: string; label: string }[] = [
  { key: "home", icon: "🏠", label: "首页" },
  { key: "datasets", icon: "📁", label: "数据集" },
  { key: "search", icon: "🔍", label: "搜索" },
  { key: "settings", icon: "⚙️", label: "设置" },
];
export function Sidebar({ page, onSelect }: { page: Page; onSelect: (p: Page) => void }) {
  return (
    <nav className="sidebar">
      <div className="sidebar-logo">🌱 fan-files</div>
      {items.map((it) => (
        <button key={it.key} className={page === it.key ? "side-item active" : "side-item"}
                onClick={() => onSelect(it.key)}>
          <span>{it.icon}</span> {it.label}
        </button>
      ))}
      <div className="sidebar-version">v0.2.0</div>
    </nav>
  );
}
```

`App.tsx`：

```tsx
import { useState } from "react";
import { Sidebar, Page } from "./components/Sidebar";
import HomePage from "./pages/HomePage";
import DatasetsPage from "./pages/DatasetsPage";
import SearchPage from "./pages/SearchPage";
import SettingsPage from "./pages/SettingsPage";

export default function App() {
  const [page, setPage] = useState<Page>("home");
  return (
    <div className="app">
      <Sidebar page={page} onSelect={setPage} />
      <main className="content">
        {page === "home" && <HomePage />}
        {page === "datasets" && <DatasetsPage />}
        {page === "search" && <SearchPage />}
        {page === "settings" && <SettingsPage />}
      </main>
    </div>
  );
}
```

占位页（其余三页）渲染 `"数据集页面（占位）"` 等文本；`App.css` 加 `.app{display:flex} .sidebar{width:180px;...} .content{flex:1}` 基础样式。

- [ ] **Step 4: 跑测试确认通过**

Run: `npm test`
Expected: 2 passed

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src
git commit -m "feat(desktop): app shell with sidebar navigation and page placeholders"
```

### Task 8: API 客户端与类型

**Files:**
- Create: `apps/desktop/src/api.ts`
- Test: `apps/desktop/src/api.test.ts`

- [ ] **Step 1: 写失败测试**

```tsx
import { describe, it, expect, vi } from "vitest";
import { fetchStats, fetchDatasets, searchDatasets, fetchDatasetDetail } from "./api";

describe("api client", () => {
  it("fetchStats calls /api/v1/stats", async () => {
    const m = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ data: { directories: 1 } }) });
    vi.stubGlobal("fetch", m);
    await fetchStats();
    expect(m).toHaveBeenCalledWith(expect.stringContaining("/api/v1/stats"));
    vi.unstubAllGlobals();
  });
  it("searchDatasets encodes q", async () => {
    const m = vi.fn().mockResolvedValue({ ok: true, json: async () => ({ data: [] }) });
    vi.stubGlobal("fetch", m);
    await searchDatasets("水稻 基因组");
    expect(m).toHaveBeenCalledWith(expect.stringContaining(encodeURIComponent("水稻 基因组")));
    vi.unstubAllGlobals();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test`
Expected: FAIL（模块不存在）

- [ ] **Step 3: 实现**

```tsx
const BASE = "http://127.0.0.1:17951"; // Task 16 换成后端提供的实际端口

export interface Stats { directories: number; files: number; datasets: number; }
export interface DatasetRow { id: number; name: string; dataset_type: string | null;
  species: string | null; path: string; file_count: number; }
export interface DatasetDetail { id: number; name: string; dataset_type: string | null;
  species: string | null; path: string; assets: { name: string; asset_type: string; files: { path: string; role: string }[] }[]; }

async function get<T>(path: string): Promise<T> {
  const r = await fetch(`${BASE}${path}`);
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  const body = await r.json();
  return body.data as T; // Envelope 包装
}

export const fetchStats = () => get<Stats>("/api/v1/stats");
export const fetchDatasets = (q: { page?: number; size?: number; type?: string }) =>
  get<DatasetRow[]>(`/api/v1/datasets?${new URLSearchParams(Object.entries(q).filter(([,v]) => v != null) as [string,string][]).toString()}`);
export const searchDatasets = (q: string) => get<DatasetRow[]>(`/api/v1/search?q=${encodeURIComponent(q)}`);
export const fetchDatasetDetail = (id: number) => get<DatasetDetail>(`/api/v1/datasets/${id}`);
```

（share 实际响应结构以 `models.rs` 的 `Envelope`/分页字段为准——实现时读 `api/mod.rs` 的 `datasets` handler 返回体，对齐字段名；若列表是 `{items,total}` 包装，这里相应调整类型。）

- [ ] **Step 4: 跑测试确认通过**

Run: `npm test`
Expected: 2 passed

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/api.ts apps/desktop/src/api.test.ts
git commit -m "feat(desktop): typed api client for share endpoints"
```

### Task 9: 首页仪表盘（统计卡 + 扫描按钮 + 进度/日志 + 空状态）

**Files:**
- Modify: `apps/desktop/src/pages/HomePage.tsx`
- Create: `apps/desktop/src/components/ScanPanel.tsx`
- Test: `apps/desktop/src/pages/HomePage.test.tsx`

- [ ] **Step 1: 写失败测试**

```tsx
import { render, screen } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import HomePage from "./HomePage";

describe("HomePage", () => {
  it("shows empty-state CTA when no directories configured", async () => {
    vi.stubGlobal("__TAURI_INTERNALS__", { invoke: vi.fn(async (cmd: string) => {
      if (cmd === "read_config") return { include: [], exclude: [], endpoint: "", api_key: "", model: "" };
      return null;
    })});
    vi.mock("../api", () => ({ fetchStats: vi.fn(async () => ({ directories: 0, files: 0, datasets: 0 })) }));
    render(<HomePage />);
    expect(await screen.findByText("选择目录开始扫描")).toBeInTheDocument();
  });
  it("renders stats cards when indexed", async () => {
    vi.mock("../api", () => ({ fetchStats: vi.fn(async () => ({ directories: 6399, files: 109796, datasets: 1453 })) }));
    render(<HomePage />);
    expect(await screen.findByText("6,399")).toBeInTheDocument();
    expect(screen.getByText("1,453")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test`
Expected: FAIL

- [ ] **Step 3: 实现**

`HomePage.tsx`：

```tsx
import { useEffect, useState } from "react";
import { fetchStats, Stats } from "../api";
import ScanPanel from "../components/ScanPanel";

export default function HomePage() {
  const [stats, setStats] = useState<Stats | null>(null);
  const [configured, setConfigured] = useState(true);
  useEffect(() => {
    fetchStats().then(setStats).catch(() => setStats(null));
    // read_config 判断是否配置过目录（Task 16 前用 invoke 占位）
  }, []);
  return (
    <div className="page">
      <h2>首页</h2>
      {!configured ? (
        <div className="empty-cta">
          <p>先告诉 fan-files 你的数据在哪里</p>
          <button className="primary">📁 选择目录开始扫描</button>
        </div>
      ) : (
        <>
          <div className="stat-cards">
            <div className="stat-card"><b>{stats ? stats.directories.toLocaleString() : "—"}</b><span>目录</span></div>
            <div className="stat-card"><b>{stats ? stats.files.toLocaleString() : "—"}</b><span>文件</span></div>
            <div className="stat-card"><b>{stats ? stats.datasets.toLocaleString() : "—"}</b><span>数据集</span></div>
          </div>
          <ScanPanel />
        </>
      )}
    </div>
  );
}
```

`ScanPanel.tsx`（扫描触发与进度；Task 17 接真事件，先可运行形态）：

```tsx
import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

export default function ScanPanel() {
  const [running, setRunning] = useState(false);
  const [lines, setLines] = useState<string[]>([]);
  async function scan() {
    setRunning(true); setLines([]);
    await invoke("scan_now"); // Task 17 改为事件流监听
    setRunning(false);
  }
  return (
    <div className="scan-panel">
      <button className="primary" disabled={running} onClick={scan}>
        {running ? "扫描中…" : "🔄 重新扫描"}
      </button>
      {lines.length > 0 && <pre className="scan-log">{lines.join("\n")}</pre>}
    </div>
  );
}
```

（"选择目录"按钮点击行为 = 打开系统目录选择器 → `write_config` 写 include → 触发扫描；目录选择器插件在 Task 13 设置页一并接入，这里先放按钮。）

- [ ] **Step 4: 跑测试确认通过**

Run: `npm test`
Expected: 2 passed

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/pages/HomePage.tsx apps/desktop/src/components/ScanPanel.tsx apps/desktop/src/pages/HomePage.test.tsx
git commit -m "feat(desktop): dashboard page with stats cards, scan button and empty state"
```

### Task 10: 数据集表格（分页/筛选/排序）

**Files:**
- Create: `apps/desktop/src/components/DataTable.tsx`
- Modify: `apps/desktop/src/pages/DatasetsPage.tsx`
- Test: `apps/desktop/src/components/DataTable.test.tsx`

- [ ] **Step 1: 写失败测试**

```tsx
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import DataTable from "./DataTable";
import { DatasetRow } from "../api";

const rows: DatasetRow[] = [
  { id: 1, name: "Oryza_sativa_v1", dataset_type: "genome", species: "Oryza sativa", path: "/a/b/v1", file_count: 25 },
  { id: 2, name: "tie_sRNA", dataset_type: "transcriptome", species: null, path: "/x/y", file_count: 3921 },
];

describe("DataTable", () => {
  it("renders rows with type badges", () => {
    render(<DataTable rows={rows} onSelect={() => {}} />);
    expect(screen.getByText("Oryza_sativa_v1")).toBeInTheDocument();
    expect(screen.getByText("genome")).toBeInTheDocument();
    expect(screen.getByText("transcriptome")).toBeInTheDocument();
  });
  it("fires onSelect on row click", () => {
    const onSelect = vi.fn();
    render(<DataTable rows={rows} onSelect={onSelect} />);
    fireEvent.click(screen.getByText("Oryza_sativa_v1"));
    expect(onSelect).toHaveBeenCalledWith(rows[0]);
  });
  it("empty state", () => {
    render(<DataTable rows={[]} onSelect={() => {}} />);
    expect(screen.getByText("还没有数据集 — 去首页开始扫描")).toBeInTheDocument();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test`
Expected: FAIL

- [ ] **Step 3: 实现**

```tsx
import { DatasetRow } from "../api";

export default function DataTable({ rows, onSelect }: { rows: DatasetRow[]; onSelect: (r: DatasetRow) => void }) {
  if (rows.length === 0) return <div className="empty">还没有数据集 — 去首页开始扫描</div>;
  return (
    <table className="data-table">
      <thead><tr><th>名称</th><th>类型</th><th>物种</th><th>文件</th><th>路径</th></tr></thead>
      <tbody>
        {rows.map((r) => (
          <tr key={r.id} onClick={() => onSelect(r)}>
            <td>{r.name}</td>
            <td><span className={`badge badge-${r.dataset_type ?? "other"}`}>{r.dataset_type ?? "—"}</span></td>
            <td>{r.species ?? "—"}</td>
            <td>{r.file_count.toLocaleString()}</td>
            <td className="mono">{r.path}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
```

`DatasetsPage.tsx`：

```tsx
import { useEffect, useState } from "react";
import { fetchDatasets, fetchDatasetDetail, DatasetRow, DatasetDetail } from "../api";
import DataTable from "../components/DataTable";

export default function DatasetsPage() {
  const [rows, setRows] = useState<DatasetRow[]>([]);
  const [page, setPage] = useState(0);
  const [type, setType] = useState<string | undefined>();
  const [detail, setDetail] = useState<DatasetDetail | null>(null);
  useEffect(() => { fetchDatasets({ page, size: 50, type }).then(setRows).catch(() => setRows([])); }, [page, type]);
  async function openDetail(r: DatasetRow) { setDetail(await fetchDatasetDetail(r.id)); }
  return (
    <div className="page">
      <h2>数据集</h2>
      <div className="filters">
        {["genome", "transcriptome", "variant", "other"].map((t) => (
          <button key={t} className={type === t ? "chip active" : "chip"} onClick={() => setType(type === t ? undefined : t)}>{t}</button>
        ))}
      </div>
      <DataTable rows={rows} onSelect={openDetail} />
      <div className="pager">
        <button disabled={page === 0} onClick={() => setPage(page - 1)}>上一页</button>
        <button onClick={() => setPage(page + 1)}>下一页</button>
      </div>
      {detail && (
        <div className="modal" onClick={() => setDetail(null)}>
          <div className="modal-body" onClick={(e) => e.stopPropagation()}>
            <h3>{detail.name}</h3>
            <p>物种: {detail.species ?? "—"} · 路径: {detail.path}</p>
            <h4>资产</h4>
            <ul>{detail.assets.map((a) => <li key={a.name}>{a.name}（{a.asset_type}）· {a.files.length} 文件</li>)}</ul>
            <div className="modal-actions">
              <button disabled title="即将推出">📤 共享</button>
              <button onClick={() => { /* Task 17: invoke("open_path", {path: detail.path}) */ }}>📂 打开目录</button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
```

（筛选 chip 的取值以 share `/facets` 返回的真实类型为准——实现时 fetch facets 替换硬编码数组。）

- [ ] **Step 4: 跑测试确认通过**

Run: `npm test`
Expected: 3 passed

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/components/DataTable.tsx apps/desktop/src/pages/DatasetsPage.tsx apps/desktop/src/components/DataTable.test.tsx
git commit -m "feat(desktop): datasets table with pagination, filter and detail modal"
```

### Task 11: 搜索页

**Files:**
- Modify: `apps/desktop/src/pages/SearchPage.tsx`
- Test: `apps/desktop/src/pages/SearchPage.test.tsx`

- [ ] **Step 1: 写失败测试**

```tsx
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import SearchPage from "./SearchPage";

describe("SearchPage", () => {
  it("searches on submit and renders results", async () => {
    vi.mock("../api", () => ({
      searchDatasets: vi.fn(async () => [{ id: 1, name: "Oryza_sativa_v1", dataset_type: "genome", species: "Oryza sativa", path: "/a/v1", file_count: 3 }]),
    }));
    render(<SearchPage />);
    fireEvent.change(screen.getByPlaceholderText("搜索你的数据（如：水稻基因组）…"), { target: { value: "水稻" } });
    fireEvent.submit(screen.getByRole("searchbox").closest("form")!);
    await waitFor(() => expect(screen.getByText("Oryza_sativa_v1")).toBeInTheDocument());
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test`
Expected: FAIL

- [ ] **Step 3: 实现**

```tsx
import { useState } from "react";
import { searchDatasets, DatasetRow } from "../api";
import DataTable from "../components/DataTable";

export default function SearchPage() {
  const [q, setQ] = useState("");
  const [rows, setRows] = useState<DatasetRow[] | null>(null);
  async function submit(e: React.FormEvent) {
    e.preventDefault();
    if (!q.trim()) return;
    setRows(await searchDatasets(q));
  }
  return (
    <div className="page">
      <h2>搜索</h2>
      <form role="search" onSubmit={submit}>
        <input role="searchbox" className="search-box" value={q} onChange={(e) => setQ(e.target.value)}
               placeholder="搜索你的数据（如：水稻基因组）…" />
        <button type="submit" className="primary">搜索</button>
      </form>
      {rows === null ? (
        <div className="empty">输入关键词或自然语言描述，搜索你的数据集</div>
      ) : (
        <DataTable rows={rows} onSelect={() => {}} />
      )}
    </div>
  );
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `npm test`
Expected: 1 passed

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/pages/SearchPage.tsx apps/desktop/src/pages/SearchPage.test.tsx
git commit -m "feat(desktop): search page wired to /api/v1/search"
```

### Task 12: 设置页（目录 + 模型 + 关于）

**Files:**
- Modify: `apps/desktop/src/pages/SettingsPage.tsx`
- Test: `apps/desktop/src/pages/SettingsPage.test.tsx`

- [ ] **Step 1: 写失败测试**

```tsx
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import SettingsPage from "./SettingsPage";

describe("SettingsPage", () => {
  it("loads config into fields", async () => {
    vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async (cmd: string) =>
      cmd === "read_config" ? { include: ["/data/x"], exclude: [], endpoint: "http://e", api_key: "k", model: "m" } : null) }));
    render(<SettingsPage />);
    await waitFor(() => expect(screen.getByDisplayValue("/data/x")).toBeInTheDocument());
  });
  it("saves config on button click", async () => {
    const invoke = vi.fn(async () => null);
    vi.mock("@tauri-apps/api/core", () => ({ invoke }));
    render(<SettingsPage />);
    await waitFor(() => expect(screen.getByRole("button", { name: "保存配置" })).toBeInTheDocument());
    fireEvent.click(screen.getByRole("button", { name: "保存配置" }));
    expect(invoke).toHaveBeenCalledWith("write_config", expect.objectContaining({ include: [] }));
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test`
Expected: FAIL

- [ ] **Step 3: 实现**

```tsx
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Cfg { include: string[]; exclude: string[]; endpoint: string; api_key: string; model: string; }

export default function SettingsPage() {
  const [cfg, setCfg] = useState<Cfg | null>(null);
  useEffect(() => { invoke<Cfg>("read_config").then(setCfg).catch(() => setCfg({ include: [], exclude: [], endpoint: "", api_key: "", model: "" })); }, []);
  if (!cfg) return <div className="page">加载中…</div>;
  function patch(p: Partial<Cfg>) { setCfg({ ...cfg!, ...p }); }
  async function save() { await invoke("write_config", { cfg: cfg! }); alert("已保存"); }
  return (
    <div className="page">
      <h2>设置</h2>
      <section>
        <h3>数据目录</h3>
        <ul>{cfg.include.map((d, i) => (
          <li key={i}>{d} <button onClick={() => patch({ include: cfg.include.filter((_, j) => j !== i) })}>移除</button></li>
        ))}</ul>
        <button className="primary" onClick={() => {/* 目录选择器：Task 13 */}}>📁 添加目录</button>
      </section>
      <section>
        <h3>模型配置</h3>
        <label>Endpoint <input value={cfg.endpoint} onChange={(e) => patch({ endpoint: e.target.value })} /></label>
        <label>API Key <input type="password" value={cfg.api_key} onChange={(e) => patch({ api_key: e.target.value })} /></label>
        <label>Model <input value={cfg.model} onChange={(e) => patch({ model: e.target.value })} /></label>
        <button onClick={async () => { const ok = await invoke<boolean>("test_connection", { cfg: cfg! }); alert(ok ? "连接成功" : "连接失败"); }}>测试连接</button>
      </section>
      <section>
        <h3>账号与崖州湾试用</h3>
        <p className="muted">即将推出</p>
      </section>
      <section>
        <h3>关于</h3>
        <p>版本 v0.2.0 · <button onClick={() => invoke("check_update")}>检查更新</button></p>
      </section>
      <button className="primary" onClick={save}>保存配置</button>
    </div>
  );
}
```

- [ ] **Step 4: 跑测试确认通过**

Run: `npm test`
Expected: 2 passed

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/pages/SettingsPage.tsx apps/desktop/src/pages/SettingsPage.test.tsx
git commit -m "feat(desktop): settings page for directories, model config and about"
```

### Task 13: 目录选择器 + 打开目录 + 测试连接 + 检查更新（后端补齐）

**Files:**
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/Cargo.toml`
- Modify: `apps/desktop/src/pages/SettingsPage.tsx`（接 `pick_directory`）

- [ ] **Step 1: 实现后端命令**

Cargo.toml 加 `tauri-plugin-dialog = "2"`。`lib.rs`：

```rust
#[tauri::command]
pub async fn pick_directory(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = app.dialog().file().blocking_pick_folder();
    Ok(picked.map(|p| p.to_string()))
}

#[tauri::command]
pub async fn test_connection(cfg: FanConfig) -> Result<bool, String> {
    let body = serde_json::json!({
        "model": cfg.model,
        "messages": [{"role": "user", "content": "reply OK"}],
        "max_tokens": 10
    });
    let resp = reqwest::Client::new()
        .post(&cfg.endpoint)
        .header("Authorization", format!("Bearer {}", cfg.api_key))
        .header("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(30))
        .json(&body)
        .send().await
        .map_err(|e| e.to_string())?;
    Ok(resp.status().is_success())
}

#[tauri::command]
pub async fn open_path(app: tauri::AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn check_update() -> Result<String, String> {
    // spawn `fan-files update` sidecar, 返回其 stdout 末行
    let out = tokio::process::Command::new(sidecar_bin("fan-files"))
        .arg("update")
        .output().await
        .map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
```

Cargo.toml 加 `reqwest = { version = "0.12", features = ["json"] }`、`tokio = { version = "1", features = ["process", "macros", "rt-multi-thread"] }`；`sidecar_bin` 助手（Task 16 实现，先返回 PATH 中的 `fan-files`）。

注册：`invoke_handler` 加 `pick_directory, test_connection, open_path, check_update`。

- [ ] **Step 2: 设置页接目录选择**

`SettingsPage.tsx` 的"添加目录"按钮：

```tsx
onClick={async () => {
  const dir = await invoke<string | null>("pick_directory");
  if (dir && !cfg.include.includes(dir)) patch({ include: [...cfg.include, dir] });
}}
```

- [ ] **Step 3: 编译 + 手动冒烟**

Run: `cd apps/desktop/src-tauri && cargo check`
Expected: 无错误。
Run: `npm run tauri dev`，手动验证：设置页点"添加目录"弹系统目录框、"打开目录"（数据集详情弹层）唤起 Finder。

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src-tauri apps/desktop/src/pages/SettingsPage.tsx
git commit -m "feat(desktop): directory picker, open-path, test-connection and update commands"
```

### Task 14: 托盘常驻

**Files:**
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src-tauri/Cargo.toml`（`tauri = { features = ["tray-icon"] }`）

- [ ] **Step 1: 实现托盘与关闭行为**

```rust
use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconEvent},
    Manager,
};

// run() 内：
tauri::Builder::default()
    .setup(|app| {
        let open = MenuItem::with_id(app, "open", "打开窗口", true, None::<&str>)?;
        let scan = MenuItem::with_id(app, "scan", "立即扫描", true, None::<&str>)?;
        let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&open, &scan, &quit])?;
        TrayIconBuilder::new()
            .menu(&menu)
            .on_menu_event(|app, event| match event.id.as_ref() {
                "open" => { let w = app.get_webview_window("main").unwrap(); w.show(); w.set_focus().ok(); }
                "scan" => { /* Task 17: 触发扫描 */ }
                "quit" => app.exit(0),
                _ => {}
            })
            .build(app)?;
        Ok(())
    })
    // Windows 关闭按钮收托盘；macOS/Linux 保持退出
    .on_window_event(|window, event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            #[cfg(target_os = "windows")]
            { api.prevent_close(); window.hide().ok(); }
        }
    })
```

- [ ] **Step 2: 编译验证**

Run: `cd apps/desktop/src-tauri && cargo check`
Expected: 无错误。

- [ ] **Step 3: Commit**

```bash
git add apps/desktop/src-tauri
git commit -m "feat(desktop): tray icon with open/scan/quit, close-to-tray on Windows"
```

### Task 15: 全局错误横幅（引擎失联）

**Files:**
- Create: `apps/desktop/src/components/EngineBanner.tsx`
- Modify: `apps/desktop/src/App.tsx`
- Test: `apps/desktop/src/components/EngineBanner.test.tsx`

- [ ] **Step 1: 写失败测试**

```tsx
import { render, screen, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi } from "vitest";
import EngineBanner from "./EngineBanner";

describe("EngineBanner", () => {
  it("renders error and retry button", () => {
    render(<EngineBanner error="引擎未运行" onRetry={() => {}} />);
    expect(screen.getByText("引擎未运行")).toBeInTheDocument();
    expect(screen.getByText("重试")).toBeInTheDocument();
  });
  it("fires onRetry", () => {
    const onRetry = vi.fn();
    render(<EngineBanner error="x" onRetry={onRetry} />);
    fireEvent.click(screen.getByText("重试"));
    expect(onRetry).toHaveBeenCalled();
  });
});
```

- [ ] **Step 2: 跑测试确认失败**

Run: `npm test`
Expected: FAIL

- [ ] **Step 3: 实现**

```tsx
export default function EngineBanner({ error, onRetry }: { error: string | null; onRetry: () => void }) {
  if (!error) return null;
  return (
    <div className="engine-banner">
      <span>⚠️ {error}</span>
      <button onClick={onRetry}>重试</button>
    </div>
  );
}
```

`App.tsx` 挂载：`const [engineError, setEngineError] = useState<string | null>(null);`，content 顶部渲染 `<EngineBanner error={engineError} onRetry={() => setEngineError(null)} />`（Task 16 由生命周期管理器设置错误）。

- [ ] **Step 4: 跑测试确认通过**

Run: `npm test`
Expected: 2 passed

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src/components/EngineBanner.tsx apps/desktop/src/components/EngineBanner.test.tsx apps/desktop/src/App.tsx
git commit -m "feat(desktop): global engine error banner with retry"
```

---

## Phase 3 — 编排（sidecar 生命周期 + 扫描）

### Task 16: sidecar 生命周期管理器（share 常驻 + 端口回退 + 健康检查）

**Files:**
- Create: `apps/desktop/src-tauri/src/engine.rs`
- Modify: `apps/desktop/src-tauri/src/lib.rs`（setup 时启动，command 暴露端口）
- Test: `apps/desktop/src-tauri/src/engine.rs`（端口回退逻辑单测）

- [ ] **Step 1: 写失败测试（端口探测逻辑）**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_port_rolls_on_conflict() {
        assert_ne!(next_port(17951, true), 17951);
        assert_eq!(next_port(17951, false), 17951);
    }
}
```

（`next_port(base, conflict)`：冲突时用随机高位端口，如 20000 + (rand % 20000)。）

- [ ] **Step 2: 跑测试确认失败**

Run: `cd apps/desktop/src-tauri && cargo test --lib`
Expected: FAIL

- [ ] **Step 3: 实现 engine.rs**

```rust
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Mutex;
use std::process::Stdio;
use std::time::Duration;
use tauri::Manager;

pub static SHARE_PORT: AtomicU16 = AtomicU16::new(17951);

pub struct Engine { pub share: Mutex<Option<std::process::Child>> }

pub fn sidecar_bin(name: &str) -> std::path::PathBuf {
    // dev: workspace target/release；打包后: tauri sidecar 相对路径
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    let dev = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release").join(&exe);
    if dev.exists() { dev } else { std::path::PathBuf::from(&exe) }
}

pub fn next_port(base: u16, conflict: bool) -> u16 {
    if !conflict { base } else { 20000 + (std::process::id() as u16 % 15000) }
}

fn is_port_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// 启动 share（默认 17951，冲突换随机），返回实际端口；失败返回 Err。
pub fn start_share(engine: &Engine) -> Result<u16, String> {
    let base = SHARE_PORT.load(Ordering::SeqCst);
    let port = next_port(base, !is_port_free(base));
    SHARE_PORT.store(port, Ordering::SeqCst);
    let child = std::process::Command::new(sidecar_bin("fan-files-share"))
        .arg("--bind").arg(format!("127.0.0.1:{port}"))
        .stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().map_err(|e| e.to_string())?;
    *engine.share.lock().unwrap() = Some(child);
    Ok(port)
}

pub async fn wait_healthy(port: u16) -> bool {
    for _ in 0..20 {
        if let Ok(r) = reqwest::get(format!("http://127.0.0.1:{port}/healthz")).await {
            if r.status().is_success() { return true; }
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    false
}
```

`lib.rs` setup：创建 `Engine`，`start_share` + `wait_healthy`，失败则放 `engine_error` 状态（App 状态用 `tauri::State<EngineStatus>`：`Mutex<Option<String>>`）。commands：

```rust
#[tauri::command]
pub fn get_share_port() -> u16 { SHARE_PORT.load(Ordering::SeqCst) }

#[tauri::command]
pub async fn retry_engine(engine: tauri::State<'_, Engine>) -> Result<u16, String> {
    let port = start_share(&engine)?;
    if wait_healthy(port).await { Ok(port) } else { Err("engine unhealthy".into()) }
}
```

前端 `api.ts` 的 BASE 改为 `http://127.0.0.1:${await invoke("get_share_port")}`（模块初始化时取一次；banner 重试后刷新）。

- [ ] **Step 4: 跑测试确认通过 + 编译**

Run: `cd apps/desktop/src-tauri && cargo test --lib && cargo check`
Expected: 1 passed，无错误

- [ ] **Step 5: Commit**

```bash
git add apps/desktop/src-tauri/src/engine.rs apps/desktop/src-tauri/src/lib.rs apps/desktop/src/api.ts
git commit -m "feat(desktop): share sidecar lifecycle manager with port fallback and health check"
```

### Task 17: 扫描编排（spawn discover + 进度事件 + 互斥 + 定时扫描）

**Files:**
- Modify: `apps/desktop/src-tauri/src/lib.rs`
- Modify: `apps/desktop/src/pages/HomePage.tsx`（监听进度事件）
- Modify: `apps/desktop/src/components/ScanPanel.tsx`

- [ ] **Step 1: 实现后端扫描编排**

```rust
use std::sync::atomic::{AtomicBool, Ordering};

static SCANNING: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub async fn scan_now(app: tauri::AppHandle) -> Result<(), String> {
    if SCANNING.swap(true, Ordering::SeqCst) {
        return Err("already scanning".into());
    }
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut child = match tokio::process::Command::new(sidecar_bin("fan-files"))
            .env("FAN_JSON_FORMAT", "1")
            .arg("discover")
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn() {
            Ok(c) => c,
            Err(e) => { handle.emit("scan://error", e.to_string()).ok(); SCANNING.store(false, Ordering::SeqCst); return; }
        };
        use tokio::io::AsyncBufReadExt;
        if let Some(stderr) = child.stderr.take() {
            let mut lines = tokio::io::BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                handle.emit("scan://progress", &line).ok();
            }
        }
        let status = child.wait().await;
        handle.emit("scan://done", status.map(|s| s.code().unwrap_or(0)).unwrap_or(-1)).ok();
        SCANNING.store(false, Ordering::SeqCst);
    });
    Ok(())
}

#[tauri::command]
pub fn scan_state() -> bool { SCANNING.load(Ordering::SeqCst) }
```

定时扫描（托盘常驻）：setup 里 `tauri::async_runtime::spawn` 一个循环 `loop { sleep(3600s); if !SCANNING { let _ = scan_now(app.clone()).await; } }`——用 `std::thread::sleep` 的独立线程 + `app.emit`，避免阻塞主循环。

- [ ] **Step 2: 前端接事件**

`ScanPanel.tsx`：

```tsx
import { listen } from "@tauri-apps/api/event";

// 组件内 useEffect：
useEffect(() => {
  const un1 = listen<string>("scan://progress", (e) => setLines((ls) => [...ls, e.payload]));
  const un2 = listen<number>("scan://done", () => setRunning(false));
  const un3 = listen<string>("scan://error", (e) => setLines((ls) => [...ls, e.payload]));
  return () => { un1.then((u) => u()); un2.then((u) => u()); un3.then((u) => u()); };
}, []);
```

scan() 改为 `invoke("scan_now")`，`scan_state` 用于初始 running 状态。

- [ ] **Step 3: 手动冒烟**

Run: `npm run tauri dev` → 点"重新扫描" → 观察进度行滚动（应与 CLI 输出一致）→ 完成后统计卡刷新。
Expected: 进度行含 "Bottom-Up: 6399 directories found" 等。

- [ ] **Step 4: Commit**

```bash
git add apps/desktop/src-tauri/src/lib.rs apps/desktop/src/components/ScanPanel.tsx apps/desktop/src/pages/HomePage.tsx
git commit -m "feat(desktop): scan orchestration with stderr streaming, mutex and hourly schedule"
```

### Task 18: 集成冒烟脚本

**Files:**
- Create: `apps/desktop/scripts/smoke.sh`

- [ ] **Step 1: 写冒烟脚本**

```bash
#!/usr/bin/env bash
# 起 share → 校验核心端点 → 退出
set -euo pipefail
PORT="${1:-17951}"
BIN="${2:-target/release/fan-files-share}"
"$BIN" --bind "127.0.0.1:$PORT" & PID=$!
trap 'kill $PID 2>/dev/null || true' EXIT
sleep 2
curl -sf "http://127.0.0.1:$PORT/healthz" | grep -q '"ok"'
curl -sf "http://127.0.0.1:$PORT/api/v1/stats"
curl -sf "http://127.0.0.1:$PORT/api/v1/datasets?size=5"
curl -sf "http://127.0.0.1:$PORT/api/v1/search?q=genome"
echo "SMOKE OK"
```

- [ ] **Step 2: 跑通**

Run: `bash apps/desktop/scripts/smoke.sh 17952 $(pwd)/target/release/fan-files-share`（在仓库根，先 `cargo build --release -p fan-files-share`）
Expected: 输出 SMOKE OK（需已有索引数据；无索引时 search 返回空数组仍为 200）

- [ ] **Step 3: Commit**

```bash
git add apps/desktop/scripts/smoke.sh
git commit -m "test(desktop): smoke script for share endpoints"
```

---

## Phase 4 — CI 与打包

### Task 19: GitHub Actions 三平台构建（含 Windows）

**Files:**
- Create: `.github/workflows/build-desktop.yml`

- [ ] **Step 1: 写 workflow**

```yaml
name: build-desktop
on:
  push:
    branches: [feat/desktop-app]
  workflow_dispatch:

jobs:
  build:
    strategy:
      fail-fast: false
      matrix:
        include:
          - platform: macos-latest
          - platform: ubuntu-22.04
          - platform: windows-latest
    runs-on: ${{ matrix.platform }}
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with: { node-version: 22 }
      - uses: dtolnay/rust-toolchain@stable
      - uses: swatinem/rust-cache@v2
        with: { workspaces: ". -> target" }
      - name: Linux deps
        if: matrix.platform == 'ubuntu-22.04'
        run: sudo apt-get update && sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf libssl-dev
      - name: Build sidecars
        run: cargo build --release -p fan-files -p fan-files-share
      - name: Install npm deps
        working-directory: apps/desktop
        run: npm install
      - name: Frontend tests
        working-directory: apps/desktop
        run: npm test -- --run
      - uses: tauri-apps/tauri-action@v0
        with:
          projectPath: apps/desktop
          args: --no-bundle-deps-check
```

（tauri-action 自动上传产物；Windows runner 上 sidecar 名为 `fan-files.exe`——build.rs 的复制逻辑按 `cfg!(windows)` 加 `.exe` 后缀，Task 6 的 build.rs 需同步修正：`let bin = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };`）

- [ ] **Step 2: 推送触发 CI 并检查三平台全绿**

Run: `git push origin feat/desktop-app`
Expected: Actions 三个 job 绿（首次 Windows 构建常需修 webview 依赖细节，按报错迭代）

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/build-desktop.yml apps/desktop/src-tauri/build.rs
git commit -m "ci(desktop): three-platform build incl. Windows via tauri-action"
```

### Task 20: 手动验收清单（Mac + Windows）

**Files:**
- Create: `apps/desktop/ACCEPTANCE.md`

- [ ] **Step 1: 写验收清单（含每项预期）**

```markdown
# 桌面壳 v1 验收清单

## Mac（开发机）
- [ ] 首次启动：首页显示"选择目录开始扫描"
- [ ] 选目录 → 自动写 config.toml → 扫描启动，进度行滚动
- [ ] 扫描完成：统计卡显示真实数字（与 CLI `fan-files datasets` 数量一致）
- [ ] 数据集页：表格分页/类型筛选/排序可用；行点击详情弹层显示资产
- [ ] 详情弹层：📤 共享灰置（tooltip 即将推出）；📂 打开目录唤起 Finder
- [ ] 搜索页：输入 "Oryza" 出结果；输入中文物种名出结果
- [ ] 设置页：改模型配置 → 保存 → 重新打开仍在；测试连接给出正确反馈
- [ ] 托盘：关闭窗口 → 托盘仍在；托盘菜单打开窗口/退出生效
- [ ] 与 CLI 共存：CLI `fan-files search xxx` 与 GUI 看到同一份索引

## Windows（CI 产物 / VM）
- [ ] 安装包安装、启动、首次流程同 Mac
- [ ] 点 X 收托盘（不退出）；托盘菜单三项生效
- [ ] 目录选择器/打开目录（资源管理器）正常
```

- [ ] **Step 2: Mac 全流程执行并勾选**

- [ ] **Step 3: Commit**

```bash
git add apps/desktop/ACCEPTANCE.md
git commit -m "docs(desktop): acceptance checklist"
```

---

## 执行顺序与依赖

```
Phase 0（T1→T2→T3）→ Phase 1（T4→T5→T6）→ Phase 2（T7…T15，按序）
→ Phase 3（T16→T17→T18）→ Phase 4（T19→T20）
```

T1-T3 独立于 GUI，可先行并单独发布（share 新端点随引擎走）。T7-T15 依赖 T4-T6 的脚手架。T16 依赖 T4-T6；T17 依赖 T16（scan_now 需要 sidecar 定位）。工作流：Mac mini 写代码 → bioinfo7 编译/测试 → bioinfo7 提交推送（`feat/desktop-app` 分支）。
