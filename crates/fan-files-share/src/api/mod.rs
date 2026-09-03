use crate::{
    error::AppError,
    models::*,
    state::{AppState, Cache},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use fan_core::index::tantivy::TantivyIndex;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tower_http::timeout::TimeoutLayer;

pub fn router(state: Arc<AppState>) -> Router {
    // 请求级超时按路由应用：普通端点保持 settings.request_timeout_ms（默认 5s）；
    // chat-search 不套该层——LLM 调用在处理器内自带上限（CHAT_LLM_TIMEOUT_SECS），
    // 慢模型/重试不会被请求级超时掐成 408（规格：LLM 失败 → 503 前端降级）
    let standard = Router::new()
        .route("/healthz", get(health))
        .route("/readyz", get(ready))
        .route("/api/v1/datasets", get(datasets))
        .route("/api/v1/datasets/{id}", get(dataset))
        .route("/api/v1/datasets/{id}/files", get(files))
        .route("/api/v1/facets", get(facets))
        .route("/api/v1/stats", get(stats))
        .route("/api/v1/search", get(search))
        .route_layer(TimeoutLayer::with_status_code(
            http::StatusCode::REQUEST_TIMEOUT,
            Duration::from_millis(state.settings.request_timeout_ms),
        ));
    Router::new()
        .route("/api/v1/chat-search", post(chat_search))
        .merge(standard)
        .with_state(state)
}

async fn health() -> Json<Envelope<Health>> {
    Json(Envelope {
        data: Health { status: "ok" },
    })
}
async fn ready(State(state): State<Arc<AppState>>) -> Result<Json<Envelope<Readiness>>, AppError> {
    let db = state.db.clone();
    let supported = state.settings.supported_schema_versions.clone();
    let version = blocking(move || db.readiness(&supported)).await?;
    Ok(Json(Envelope {
        data: Readiness {
            status: "ready",
            database: "ok",
            schema_version: version,
        },
    }))
}
async fn datasets(
    State(state): State<Arc<AppState>>,
    Query(query): Query<DatasetQuery>,
) -> Result<Json<PageEnvelope<DatasetSummary>>, AppError> {
    validate_dataset_order(&query)?;
    let limit = page_limit(query.limit, state.settings.max_page_size)?;
    let db = state.db.clone();
    let expose = state.settings.expose_absolute_paths;
    Ok(Json(
        blocking(move || db.datasets(&query, limit, expose)).await?,
    ))
}
fn validate_dataset_order(query: &DatasetQuery) -> Result<(), AppError> {
    // GUI-T4: 数据集页排序下拉支持 name（名称）/ file_count（文件数）排序。
    const SORTS: &[&str] = &["id", "relevance", "name", "file_count"];
    if query.sort.as_deref().is_some_and(|value| !SORTS.contains(&value)) {
        return Err(AppError::BadRequest(
            "sort must be id, relevance, name or file_count".into(),
        ));
    }
    if query.sort.as_deref() == Some("relevance")
        && query
            .q
            .as_deref()
            .is_none_or(|value| value.trim().is_empty())
    {
        return Err(AppError::BadRequest("sort=relevance requires q".into()));
    }
    if query.order.as_deref().is_some_and(|value| value != "asc") {
        return Err(AppError::BadRequest(
            "v1 cursor pagination supports order=asc only".into(),
        ));
    }
    Ok(())
}
async fn dataset(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
) -> Result<Json<Envelope<DatasetDetail>>, AppError> {
    let db = state.db.clone();
    let expose = state.settings.expose_absolute_paths;
    Ok(Json(Envelope {
        data: blocking(move || db.dataset(id, expose)).await?,
    }))
}
async fn files(
    State(state): State<Arc<AppState>>,
    Path(id): Path<i64>,
    Query(query): Query<FileQuery>,
) -> Result<Json<PageEnvelope<FileSummary>>, AppError> {
    let limit = page_limit(query.limit, state.settings.max_page_size)?;
    let db = state.db.clone();
    let expose = state.settings.expose_absolute_paths;
    Ok(Json(
        blocking(move || db.files(id, &query, limit, expose)).await?,
    ))
}
async fn facets(State(state): State<Arc<AppState>>) -> Result<Json<Envelope<Facets>>, AppError> {
    let ttl = Duration::from_secs(state.settings.stats_cache_seconds);
    if let Some(value) = cached(&state.facets, ttl) {
        return Ok(Json(Envelope { data: value }));
    }
    let db = state.db.clone();
    let value = blocking(move || db.facets()).await?;
    *state.facets.lock().map_err(|_| AppError::Internal)? = Some(Cache {
        loaded: Instant::now(),
        value: value.clone(),
    });
    Ok(Json(Envelope { data: value }))
}
async fn stats(State(state): State<Arc<AppState>>) -> Result<Json<Envelope<Stats>>, AppError> {
    let ttl = Duration::from_secs(state.settings.stats_cache_seconds);
    if let Some(value) = cached(&state.stats, ttl) {
        return Ok(Json(Envelope { data: value }));
    }
    let db = state.db.clone();
    let value = blocking(move || db.stats()).await?;
    *state.stats.lock().map_err(|_| AppError::Internal)? = Some(Cache {
        loaded: Instant::now(),
        value: value.clone(),
    });
    Ok(Json(Envelope { data: value }))
}
async fn search(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Envelope<Vec<DatasetSummary>>>, AppError> {
    let q = query.q.trim().to_string();
    if q.is_empty() {
        return Err(AppError::BadRequest("q is required".into()));
    }
    let expose = state.settings.expose_absolute_paths;
    let datasets = blocking(move || run_search(&state, &q, expose)).await?;
    Ok(Json(Envelope { data: datasets }))
}

/// Run a full-text search over the Tantivy index and map the hit file ids
/// back to their datasets. Returns an empty list when the index does not
/// exist (e.g. the desktop CLI has not built one yet).
fn run_search(state: &AppState, q: &str, expose_path: bool) -> Result<Vec<DatasetSummary>, AppError> {
    let file_ids = tantivy_file_ids(state, q)?;
    state.db.search_datasets(&file_ids, expose_path)
}

// ---------- POST /api/v1/chat-search（NR-T2：对话搜索） ----------

/// LLM 调用超时（秒）：慢模型/内部重试也在此上限内；超时按调用失败处理 → 503。
/// （不套普通端点的 5s 请求级超时——见 router() 注释）
const CHAT_LLM_TIMEOUT_SECS: u64 = 60;

/// 对话搜索：messages（多轮上下文）+ question → LLM 生成结构化查询 JSON →
/// 走现有 Tantivy 搜索逻辑 → 返回 query + results。
/// LLM 未配置/调用失败/超时 → 503（前端降级基础搜索）。
async fn chat_search(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatSearchRequest>,
) -> Result<Json<Envelope<ChatSearchResponse>>, AppError> {
    let question = request.question.trim().to_string();
    if question.is_empty() {
        return Err(AppError::BadRequest("question is required".into()));
    }
    // 阶段一：LLM 调用（同步网络请求）独立 spawn_blocking 并带上限
    let llm_state = state.clone();
    let llm_messages = request.messages;
    let llm_question = question.clone();
    let llm_future =
        tokio::task::spawn_blocking(move || run_chat_search_llm(&llm_state, &llm_messages, &llm_question));
    let query = tokio::time::timeout(Duration::from_secs(CHAT_LLM_TIMEOUT_SECS), llm_future)
        .await
        .map_err(|_| {
            tracing::warn!("chat-search llm call timed out");
            AppError::LlmUnavailable
        })?
        .map_err(|error| {
            tracing::error!(%error, "chat-search llm worker failed");
            AppError::Internal
        })??;
    // 阶段二：搜索（本地 tantivy + sqlite，毫秒级，沿用 blocking 封装）
    let expose = state.settings.expose_absolute_paths;
    let search_state = state.clone();
    let (query, results) = blocking(move || {
        // 关键词空白拼接 → Tantivy 查询串（多词 OR 匹配，与 GET /search 同一入口）
        let results = run_search(&search_state, &query.keywords.join(" "), expose)?;
        Ok((query, results))
    })
    .await?;
    Ok(Json(Envelope {
        data: ChatSearchResponse { query, results },
    }))
}

/// 对话搜索阶段一：拼 prompt（上下文 + 问题）→ LLM 生成查询 JSON → 解析。
/// LLM 未配置/调用失败/输出不合格 → LlmUnavailable(503)。
fn run_chat_search_llm(
    state: &AppState,
    messages: &[ChatMessage],
    question: &str,
) -> Result<ChatQuery, AppError> {
    if !state.llm.is_configured() {
        return Err(AppError::LlmUnavailable);
    }
    let prompt = build_chat_search_prompt(messages, question);
    let content = state
        .llm
        .chat(&[serde_json::json!({"role": "user", "content": prompt})])
        .map_err(|error| {
            tracing::error!(%error, "chat-search llm call failed");
            AppError::LlmUnavailable
        })?;
    parse_chat_query(&content)
}

/// 拼对话搜索 prompt：JSON schema 指令 + 对话历史（按角色标注）+ 当前问题。
/// 与现有 LLM 查询生成提示同风格（中文、强制 JSON 输出），但带多轮上下文，
/// 让模型解析"再找/也"等指代。
fn build_chat_search_prompt(messages: &[ChatMessage], question: &str) -> String {
    let mut prompt = String::from(
        "你是数据集搜索引擎的查询生成助手。请结合对话上下文理解当前问题，生成一个用于全文搜索的查询。\n\
         只输出 JSON，不要任何多余文字，格式：\n\
         {\"keywords\": [\"关键词1\", \"关键词2\"], \"type\": \"数据集类型\"}\n\
         - keywords：2~5 个搜索关键词（结合上下文解析当前问题中的指代，如\"再找\"\"也\"等）\n\
         - type：可选，数据集类型（如 genome / transcriptome / protein），问题未明确时省略\n",
    );
    if !messages.is_empty() {
        prompt.push_str("\n对话历史：\n");
        for message in messages {
            let role = if message.role == "user" { "用户" } else { "助手" };
            prompt.push_str(&format!("{role}: {}\n", message.content.trim()));
        }
    }
    prompt.push_str("\n当前问题：\n");
    prompt.push_str(question.trim());
    prompt
}

/// 解析 LLM 返回的查询 JSON（容忍 ```json 代码围栏）。
/// JSON 非法 / keywords 缺失或为空 → LLM 输出不合格 → 503。
fn parse_chat_query(content: &str) -> Result<ChatQuery, AppError> {
    let trimmed = content.trim();
    let inner = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim();
    let query: ChatQuery = serde_json::from_str(inner).map_err(|error| {
        tracing::error!(%error, content = %inner, "chat-search llm output not parseable");
        AppError::LlmUnavailable
    })?;
    if query.keywords.is_empty() {
        tracing::error!(content = %inner, "chat-search llm output has no keywords");
        return Err(AppError::LlmUnavailable);
    }
    Ok(query)
}

/// Lock the shared index, lazily opening it on first use so that a share
/// started before the CLI built the index picks it up without a restart.
/// Open failures are logged and retried on the next call; the open is
/// guarded by the existence of the `<data_dir>/tantivy` directory.
fn tantivy_file_ids(state: &AppState, q: &str) -> Result<Vec<i64>, AppError> {
    let mut index = state.tantivy.lock().map_err(|_| AppError::Internal)?;
    if index.is_none() {
        let data_dir = state
            .settings
            .database
            .parent()
            .map(std::path::Path::to_path_buf)
            .unwrap_or_default();
        if data_dir.join("tantivy").exists() {
            match TantivyIndex::open(&data_dir, true) {
                Ok(opened) => {
                    index.replace(opened);
                }
                Err(error) => tracing::warn!(%error, "tantivy open failed"),
            }
        }
    }
    let Some(index) = index.as_ref() else {
        return Ok(vec![]);
    };
    let hits = index.search(q, 200).map_err(|error| {
        tracing::error!(%error, "tantivy search failed");
        AppError::Internal
    })?;
    Ok(hits.into_iter().map(|(id, _)| id).collect())
}

fn cached<T: Clone>(cache: &std::sync::Mutex<Option<Cache<T>>>, ttl: Duration) -> Option<T> {
    cache
        .lock()
        .ok()?
        .as_ref()
        .filter(|item| item.loaded.elapsed() < ttl)
        .map(|item| item.value.clone())
}
fn page_limit(value: Option<u32>, max: u32) -> Result<u32, AppError> {
    let value = value.unwrap_or(50);
    if value == 0 || value > max {
        return Err(AppError::BadRequest(format!(
            "limit must be between 1 and {max}"
        )));
    }
    Ok(value)
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, AppError> + Send + 'static,
) -> Result<T, AppError> {
    tokio::task::spawn_blocking(work).await.map_err(|error| {
        tracing::error!(%error, "database worker failed");
        AppError::Internal
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use fan_core::{config::LlmConfig, index::tantivy::TantivyIndex};
    use serde_json::Value;
    use tower::ServiceExt;

    #[test]
    fn accepts_relevance_only_with_query() {
        let valid = DatasetQuery {
            q: Some("Oryza".into()),
            sort: Some("relevance".into()),
            ..Default::default()
        };
        assert!(validate_dataset_order(&valid).is_ok());

        let missing_query = DatasetQuery {
            sort: Some("relevance".into()),
            ..Default::default()
        };
        assert!(validate_dataset_order(&missing_query).is_err());
    }

    #[test]
    fn rejects_unknown_sort_and_oversized_page() {
        let invalid = DatasetQuery {
            sort: Some("size".into()),
            ..Default::default()
        };
        assert!(validate_dataset_order(&invalid).is_err());
        assert!(page_limit(Some(201), 200).is_err());
        assert_eq!(page_limit(None, 200).unwrap(), 50);
    }

    // GUI-T4: name/file_count 排序无需 q 即可用（与 relevance 不同）
    #[test]
    fn accepts_name_and_file_count_sort_without_query() {
        for sort in ["name", "file_count"] {
            let valid = DatasetQuery {
                sort: Some(sort.into()),
                ..Default::default()
            };
            assert!(
                validate_dataset_order(&valid).is_ok(),
                "sort={sort} should be accepted without q"
            );
        }
        let id_sort = DatasetQuery {
            sort: Some("id".into()),
            ..Default::default()
        };
        assert!(validate_dataset_order(&id_sort).is_ok());
    }

    fn api_fixture(db_path: &std::path::Path) -> Settings {
        Settings {
            database: db_path.to_path_buf(),
            pool_size: 2,
            ..Settings::default()
        }
    }

    fn create_db(path: &std::path::Path) {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute_batch(
            "PRAGMA user_version=4;
             CREATE TABLE dataset(id INTEGER PRIMARY KEY,name TEXT,path TEXT,dataset_type TEXT,species TEXT,species_confidence TEXT,summary TEXT,updated_at INTEGER);
             CREATE TABLE asset(id INTEGER PRIMARY KEY,dataset_id INTEGER,name TEXT,asset_type TEXT);
             CREATE TABLE files(id INTEGER PRIMARY KEY,path TEXT,size INTEGER,mime_type TEXT,source_server TEXT,deleted INTEGER,updated_at INTEGER);
             CREATE TABLE asset_file(asset_id INTEGER,file_id INTEGER,role TEXT);",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn search_endpoint_returns_empty_without_tantivy_index() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("index.db");
        create_db(&db_path);
        let state = AppState::new(api_fixture(&db_path)).unwrap();
        let app = router(Arc::new(state));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/search?q=genome")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn search_endpoint_returns_datasets() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("index.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "PRAGMA user_version=4;
             CREATE TABLE dataset(id INTEGER PRIMARY KEY,name TEXT,path TEXT,dataset_type TEXT,species TEXT,species_confidence TEXT,summary TEXT,updated_at INTEGER);
             CREATE TABLE asset(id INTEGER PRIMARY KEY,dataset_id INTEGER,name TEXT,asset_type TEXT);
             CREATE TABLE files(id INTEGER PRIMARY KEY,path TEXT,size INTEGER,mime_type TEXT,source_server TEXT,deleted INTEGER,updated_at INTEGER);
             CREATE TABLE asset_file(asset_id INTEGER,file_id INTEGER,role TEXT);
             INSERT INTO dataset VALUES(1,'Oryza_sativa_v1','/data/orders/Poales/Poaceae/Oryza_sativa/v1','genome','Oryza sativa',NULL,'rice reference genome',0);
             INSERT INTO asset VALUES(1,1,'assembly','assembly');
             INSERT INTO files VALUES(10,'/data/orders/Poales/Poaceae/Oryza_sativa/v1/genome.fa',123,'text/plain','local',0,0);
             INSERT INTO asset_file VALUES(1,10,'primary');",
        )
        .unwrap();
        drop(conn);
        // Build a tiny tantivy index next to the database, as the CLI would.
        let index = TantivyIndex::open(temp.path(), false).unwrap();
        index
            .index_file(
                10,
                std::path::Path::new("/data/orders/Poales/Poaceae/Oryza_sativa/v1/genome.fa"),
                "Oryza sativa reference genome assembly",
                &[],
            )
            .unwrap();
        index.commit().unwrap();
        drop(index);
        let state = AppState::new(api_fixture(&db_path)).unwrap();
        let app = router(Arc::new(state));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/search?q=genome")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["name"], "Oryza_sativa_v1");
        assert_eq!(data[0]["type"], "genome");
        assert_eq!(data[0]["file_count"], 1);
    }

    #[tokio::test]
    async fn search_endpoint_requires_nonempty_q() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("index.db");
        create_db(&db_path);
        let state = AppState::new(api_fixture(&db_path)).unwrap();
        let app = router(Arc::new(state));
        for uri in ["/api/v1/search", "/api/v1/search?q="] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::BAD_REQUEST, "uri: {uri}");
        }
    }

    // ---------- chat-search 端点（NR-T2：对话搜索） ----------

    fn llm_cfg(base: &str) -> LlmConfig {
        LlmConfig {
            endpoint: format!("{base}/v1/chat/completions"),
            api_key: "sk-test".into(),
            model: "gpt-4o-mini".into(),
            api_type: "openai".into(),
        }
    }

    /// 造 Oryza_sativa_v1 数据集（schema v4）+ 同名 tantivy 索引，搜索可命中。
    fn seed_oryza_dataset(dir: &std::path::Path) {
        let conn = rusqlite::Connection::open(dir.join("index.db")).unwrap();
        conn.execute_batch(
            "PRAGMA user_version=4;
             CREATE TABLE dataset(id INTEGER PRIMARY KEY,name TEXT,path TEXT,dataset_type TEXT,species TEXT,species_confidence TEXT,summary TEXT,updated_at INTEGER);
             CREATE TABLE asset(id INTEGER PRIMARY KEY,dataset_id INTEGER,name TEXT,asset_type TEXT);
             CREATE TABLE files(id INTEGER PRIMARY KEY,path TEXT,size INTEGER,mime_type TEXT,source_server TEXT,deleted INTEGER,updated_at INTEGER);
             CREATE TABLE asset_file(asset_id INTEGER,file_id INTEGER,role TEXT);
             INSERT INTO dataset VALUES(1,'Oryza_sativa_v1','/data/orders/Poales/Poaceae/Oryza_sativa/v1','genome','Oryza sativa',NULL,'rice reference genome',0);
             INSERT INTO asset VALUES(1,1,'assembly','assembly');
             INSERT INTO files VALUES(10,'/data/orders/Poales/Poaceae/Oryza_sativa/v1/genome.fa',123,'text/plain','local',0,0);
             INSERT INTO asset_file VALUES(1,10,'primary');",
        )
        .unwrap();
        drop(conn);
        let index = TantivyIndex::open(dir, false).unwrap();
        index
            .index_file(
                10,
                std::path::Path::new("/data/orders/Poales/Poaceae/Oryza_sativa/v1/genome.fa"),
                "Oryza sativa reference genome assembly",
                &[],
            )
            .unwrap();
        index.commit().unwrap();
        drop(index);
    }

    /// 一次性 mock LLM HTTP 服务器：捕获请求原文，返回固定 openai 格式响应。
    /// 与 fan-core llm 测试的 with_llm_server 同款模式；测试先发请求再 join 拿原始请求。
    fn spawn_llm_server(resp_body: &str) -> (String, std::thread::JoinHandle<String>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base = format!("http://{}", addr);
        let body_owned = resp_body.to_string();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut total = Vec::new();
            let mut tmp = [0u8; 8192];
            let mut clen: usize = 0;
            loop {
                let n = stream.read(&mut tmp).unwrap();
                if n == 0 {
                    break;
                }
                total.extend_from_slice(&tmp[..n]);
                if let Some(pos) = total.windows(4).position(|w| w == b"\r\n\r\n") {
                    if clen == 0 {
                        let head = String::from_utf8_lossy(&total[..pos]).to_string();
                        clen = head
                            .lines()
                            .find_map(|l| {
                                let lower = l.to_ascii_lowercase();
                                lower
                                    .strip_prefix("content-length:")
                                    .and_then(|v| v.trim().parse::<usize>().ok())
                            })
                            .unwrap_or(0);
                    }
                    if clen > 0 && total.len() >= pos + 4 + clen {
                        break;
                    }
                }
            }
            let req = String::from_utf8_lossy(&total).to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body_owned.len(),
                body_owned
            );
            stream.write_all(response.as_bytes()).unwrap();
            req
        });
        (base, handle)
    }

    fn chat_search_request(body: &serde_json::Value) -> http::Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/v1/chat-search")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap()
    }

    /// 对话搜索：LLM 生成查询 JSON → 走现有 Tantivy 搜索 → 返回 query + results。
    /// 同时断言 LLM 请求携带对话历史与当前问题（多轮上下文）。
    #[tokio::test]
    async fn chat_search_returns_llm_query_and_results() {
        let temp = tempfile::tempdir().unwrap();
        seed_oryza_dataset(temp.path());

        let (base, server) = spawn_llm_server(
            r#"{"choices":[{"message":{"content":"{\"keywords\":[\"genome\"],\"type\":\"genome\"}"}}]}"#,
        );
        let mut settings = api_fixture(&temp.path().join("index.db"));
        settings.llm = llm_cfg(&base);
        let state = AppState::new(settings).unwrap();
        let app = router(Arc::new(state));

        let body = serde_json::json!({
            "messages": [{"role": "user", "content": "帮我找水稻基因组数据"}],
            "question": "再帮我找转录组的"
        });
        let resp = app.oneshot(chat_search_request(&body)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json: Value =
            serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(json["data"]["query"]["keywords"][0], "genome");
        assert_eq!(json["data"]["query"]["type"], "genome");
        let data = json["data"]["results"].as_array().unwrap();
        assert_eq!(data.len(), 1);
        assert_eq!(data[0]["name"], "Oryza_sativa_v1");

        // 多轮上下文必须进 LLM 请求
        let req = server.join().unwrap();
        assert!(req.contains("帮我找水稻基因组数据"), "history missing: {req}");
        assert!(req.contains("再帮我找转录组的"), "question missing: {req}");
    }

    /// LLM 未配置（endpoint/api_key 为空）→ 503，前端据此降级基础搜索
    #[tokio::test]
    async fn chat_search_503_when_llm_not_configured() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("index.db");
        create_db(&db_path);
        let state = AppState::new(api_fixture(&db_path)).unwrap();
        let app = router(Arc::new(state));

        let body = serde_json::json!({"messages": [], "question": "水稻基因组"});
        let resp = app.oneshot(chat_search_request(&body)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let json: Value =
            serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(json["error"]["code"], "llm_unavailable");
    }

    /// LLM 返回不可解析内容 → 503
    #[tokio::test]
    async fn chat_search_503_when_llm_output_unparseable() {
        let temp = tempfile::tempdir().unwrap();
        seed_oryza_dataset(temp.path());

        let (base, _server) = spawn_llm_server(
            r#"{"choices":[{"message":{"content":"sorry, I cannot help with that"}}]}"#,
        );
        let mut settings = api_fixture(&temp.path().join("index.db"));
        settings.llm = llm_cfg(&base);
        let state = AppState::new(settings).unwrap();
        let app = router(Arc::new(state));

        let body = serde_json::json!({"messages": [], "question": "水稻基因组"});
        let resp = app.oneshot(chat_search_request(&body)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let json: Value =
            serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(json["error"]["code"], "llm_unavailable");
    }

    /// question 为空 → 400
    #[tokio::test]
    async fn chat_search_requires_question() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("index.db");
        create_db(&db_path);
        let state = AppState::new(api_fixture(&db_path)).unwrap();
        let app = router(Arc::new(state));

        for question in ["", "   "] {
            let body = serde_json::json!({"messages": [], "question": question});
            let resp = app
                .clone()
                .oneshot(chat_search_request(&body))
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::BAD_REQUEST,
                "question: {question:?}"
            );
        }
    }

    /// prompt 构造：对话历史按角色标注 + 当前问题；首轮无历史则无"对话历史"段
    #[test]
    fn chat_search_prompt_includes_history_and_question() {
        let messages = vec![
            ChatMessage {
                role: "user".into(),
                content: "帮我找水稻基因组".into(),
            },
            ChatMessage {
                role: "assistant".into(),
                content: "找到 3 个基因组数据集".into(),
            },
        ];
        let prompt = build_chat_search_prompt(&messages, "再找转录组的");
        assert!(prompt.contains("对话历史"), "prompt: {prompt}");
        assert!(prompt.contains("帮我找水稻基因组"), "prompt: {prompt}");
        assert!(prompt.contains("找到 3 个基因组数据集"), "prompt: {prompt}");
        assert!(prompt.contains("再找转录组的"), "prompt: {prompt}");
        // 首轮（无历史）：不出现"对话历史"段
        let first = build_chat_search_prompt(&[], "找水稻基因组");
        assert!(!first.contains("对话历史"), "prompt: {first}");
        assert!(first.contains("找水稻基因组"), "prompt: {first}");
    }

    /// 解析 LLM 查询 JSON：容忍代码围栏；keywords 缺失/为空 → Err(503)
    #[test]
    fn chat_search_query_parse_tolerates_fences_and_rejects_empty() {
        let parsed = parse_chat_query(
            "```json\n{\"keywords\":[\"水稻\",\"转录组\"],\"type\":\"transcriptome\"}\n```",
        )
        .unwrap();
        assert_eq!(parsed.keywords, vec!["水稻", "转录组"]);
        assert_eq!(parsed.dataset_type.as_deref(), Some("transcriptome"));
        assert!(matches!(
            parse_chat_query("not json"),
            Err(AppError::LlmUnavailable)
        ));
        assert!(matches!(
            parse_chat_query("{}"),
            Err(AppError::LlmUnavailable)
        ));
        assert!(matches!(
            parse_chat_query(r#"{"keywords":[]}"#),
            Err(AppError::LlmUnavailable)
        ));
    }
}
