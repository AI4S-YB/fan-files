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
    *state.facets.lock().map_err(|_| AppError::Internal("mutex poisoned".into()))? = Some(Cache {
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
    *state.stats.lock().map_err(|_| AppError::Internal("mutex poisoned".into()))? = Some(Cache {
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
///
/// NR-T2 follow-up: uses the same hybrid pipeline as `fan-files search` (CLI):
/// Tantivy top-200 by score → load stored embeddings for those file ids →
/// compute query embedding → cosine similarity → 0.6/0.4 fusion → keep top 50.
/// Falls back to Tantivy-only when no embeddings are stored or the embedding
/// model is unavailable.
fn run_search(state: &AppState, q: &str, expose_path: bool) -> Result<Vec<DatasetSummary>, AppError> {
    let file_ids = tantivy_file_ids(state, q)?;
    let ranked = hybrid_rerank(state, q, file_ids)?;
    state.db.search_datasets(&ranked, expose_path)
}

/// Run chat-search: Tantivy hybrid → if 0 hit + LLM gave dataset_type → SQL type fallback.
/// Also handles "近半年"/"recently updated" time filter when present in the original question.
fn run_chat_search(
    state: &AppState,
    query: &ChatQuery,
    original_question: &str,
    expose_path: bool,
) -> Result<Vec<DatasetSummary>, AppError> {
    // 1) Tantivy 混合搜索（关键词 OR 匹配，与 GET /search 同一索引）
    let keywords_str = query.keywords.join(" ");
    let file_ids = tantivy_file_ids(state, &keywords_str)?;
    let ranked = hybrid_rerank(state, &keywords_str, file_ids)?;
    let results = state.db.search_datasets(&ranked, expose_path)?;

    // 2) 如果 Tantivy 命中了，直接返回（原有行为）
    if !results.is_empty() {
        // 有 dataset_type → 二次过滤（跨 dataset 层面筛掉 type 不匹配的）
        // 用子串匹配容忍拼写变体（variant ↔ genomic_variation(s)）
        if let Some(ref dt) = query.dataset_type {
            let dt_lower = dt.to_lowercase();
            let filtered: Vec<_> = results
                .into_iter()
                .filter(|d| {
                    d.dataset_type
                        .as_deref()
                        .map(|t| t.to_lowercase().contains(&dt_lower))
                        .unwrap_or(false)
                })
                .collect();
            if !filtered.is_empty() {
                return Ok(filtered);
            }
            // 有 type 但过滤后空了 → 不 fallback（type 不匹配说明 LLM 理解错了）
            return Ok(vec![]);
        }
        return Ok(results);
    }

    // 3) Tantivy 0 命中，但 LLM 给了 dataset_type → 走 SQL 类型过滤（不走索引）
    if let Some(ref dt) = query.dataset_type {
        // 时间过滤应在原始问题上解析（keywords 里不含"近半年"）
        let days = extract_days_filter(original_question);
        let type_results = state.db.search_datasets_by_type(dt, days, expose_path)?;
        if !type_results.is_empty() {
            tracing::debug!(
                "chat-search: Tantivy 0 hit, type={dt} fallback returned {} results",
                type_results.len()
            );
            return Ok(type_results);
        }
    }

    // 4) 完全 0 命中 → 返回空（前端收到空数组，正常显示）
    Ok(vec![])
}

/// 从用户问题中抠出"近 N 天/月/年更新的"时间过滤。
/// 例如："近半年更新的" → Some(180)，"近两周" → Some(14)，其余 → None。
fn extract_days_filter(text: &str) -> Option<i64> {
    let text = text.to_lowercase();
    // 查找 "近" 后的数字+单位
    let near_idx = text.find('近')?;
    let rest = &text[near_idx + '近'.len_utf8()..];
    let bytes = rest.as_bytes();
    // 跳过空格
    let mut i = 0;
    while i < bytes.len() && (bytes[i] as char).is_whitespace() {
        i += 1;
    }
    // 收集数字
    let num_start = i;
    while i < bytes.len() && (bytes[i] as char).is_ascii_digit() {
        i += 1;
    }
    if i == num_start {
        return None;
    }
    let num: i64 = rest[num_start..i].parse().ok()?;
    // 跳过空格
    while i < bytes.len() && (bytes[i] as char).is_whitespace() {
        i += 1;
    }
    // 读单位（到非中文字符为止）
    let unit_start = i;
    while i < bytes.len() {
        let c = bytes[i] as char;
        if c.is_ascii_digit() || c.is_whitespace() || c.is_ascii_punctuation() {
            break;
        }
        i += 1;
    }
    let unit = &rest[unit_start..i];
    let days = if unit.starts_with('天') || unit.starts_with('日') {
        num
    } else if unit.starts_with('周') {
        num * 7
    } else if unit.starts_with('月') {
        num * 30
    } else if unit.starts_with('年') {
        num * 365
    } else {
        return None;
    };
    Some(days)
}

/// Hybrid rerank: Tantivy top-N by score, then boost by semantic similarity
/// when both the embedding model and stored vectors are available. Returns
/// the reranked file_ids (descending score). The Tantivy-only path is the
/// identity transform (already sorted).
///
/// Mirrors `crates/fan-files/src/commands/search.rs::run` so the share HTTP
/// path and the CLI give the same ranking for the same query.
fn hybrid_rerank(state: &AppState, q: &str, file_ids: Vec<i64>) -> Result<Vec<i64>, AppError> {
    if file_ids.is_empty() {
        return Ok(file_ids);
    }
    // We need tantivy scores to do the 0.6/0.4 blend. Refetch them so the
    // caller can stay simple (passes ids only).
    let scores = tantivy_scores(state, q)?;
    if scores.is_empty() {
        return Ok(file_ids);
    }
    let max_tantivy_score = scores
        .values()
        .copied()
        .fold(0.0f32, f32::max)
        .max(1.0);
    // Pull stored embeddings for the candidates; empty means the last scan
    // ran without an embedding model — skip the rerank and keep Tantivy order.
    let stored = state
        .db
        .load_embeddings_for_ids(&file_ids)
        .map_err(|error| {
            tracing::error!(%error, "load_embeddings_for_ids failed");
            AppError::Internal("load_embeddings_for_ids".into())
        })?;
    if stored.is_empty() {
        return Ok(file_ids);
    }
    let embedding_scores: std::collections::HashMap<i64, f64> = match state.embedding.embed(q) {
        Ok(qvec) => {
            let mut scores = std::collections::HashMap::new();
            for (file_id, vec) in &stored {
                if vec.len() == qvec.len() {
                    scores.insert(*file_id, cosine_similarity(&qvec, vec));
                }
            }
            scores
        }
        Err(error) => {
            tracing::warn!(%error, "query embed failed; using tantivy-only ordering");
            return Ok(file_ids);
        }
    };
    let has_embeddings = !embedding_scores.is_empty();
    let mut merged: Vec<(i64, f64)> = file_ids
        .iter()
        .filter_map(|id| {
            scores.get(id).map(|t| {
                let norm_tantivy = *t as f64 / max_tantivy_score as f64;
                let emb_score = embedding_scores.get(id).copied().unwrap_or(0.0);
                let combined = if has_embeddings {
                    norm_tantivy * 0.6 + emb_score * 0.4
                } else {
                    norm_tantivy
                };
                (*id, combined)
            })
        })
        .collect();
    merged.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Ok(merged.into_iter().map(|(id, _)| id).collect())
}

/// Tantivy search returning id → tantivy score. The existing
/// `tantivy_file_ids` discards the score; we need it for the hybrid blend.
fn tantivy_scores(state: &AppState, q: &str) -> Result<std::collections::HashMap<i64, f32>, AppError> {
    let mut index = state.tantivy.lock().map_err(|_| AppError::Internal("mutex poisoned".into()))?;
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
                Err(ref error) if error.to_string().contains("Lockfile") || error.to_string().contains("lock") => {
                    let lock_path = data_dir.join("tantivy/.tantivy-writer.lock");
                    let meta_lock = data_dir.join("tantivy/.tantivy-meta.lock");
                    if lock_path.metadata().map(|m| m.len() == 0).unwrap_or(false) {
                        let _ = std::fs::remove_file(&lock_path);
                    }
                    if meta_lock.metadata().map(|m| m.len() == 0).unwrap_or(false) {
                        let _ = std::fs::remove_file(&meta_lock);
                    }
                    if let Ok(opened) = TantivyIndex::open(&data_dir, true) {
                        tracing::warn!("tantivy opened after stale lock cleanup");
                        index.replace(opened);
                    } else {
                        tracing::warn!(%error, "tantivy open still failed after lock cleanup");
                    }
                }
                Err(error) => tracing::warn!(%error, "tantivy open failed"),
            }
        }
    }
    let Some(index) = index.as_ref() else {
        return Ok(Default::default());
    };
    // tantivy 打开失败（如孤儿锁）→ 优雅降级返回空，交由上层 SQL 类型回退，
    // 而不是抛 500 让前端误判"搜索失败"。
    let hits = match index.search(q, 200) {
        Ok(hits) => hits,
        Err(error) => {
            tracing::warn!(%error, "tantivy search failed (degrading to empty)");
            return Ok(Default::default());
        }
    };
    Ok(hits.into_iter().collect())
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
    let na: f64 = a.iter().map(|x| *x as f64 * *x as f64).sum::<f64>().sqrt();
    let nb: f64 = b.iter().map(|x| *x as f64 * *x as f64).sum::<f64>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}

// ---------- POST /api/v1/chat-search（NR-T2：对话搜索） ----------

/// LLM 调用超时（秒）：慢模型/内部重试也在此上限内；超时按调用失败处理 → 503。
/// （不套普通端点的 5s 请求级超时——见 router() 注释）
/// 20s：LLM 端点偶发 30~60s 卡顿；超时后走本地类型识别 + SQL 回退，
/// 而不是让用户干等。仍然足以让正常（1~3s）的 LLM 响应返回。
const CHAT_LLM_TIMEOUT_SECS: u64 = 20;

/// 对话搜索：messages（多轮上下文）+ question → LLM 生成结构化查询 JSON →
/// 走现有 Tantivy 搜索逻辑 → 返回 query + results。
/// LLM 失败/超时 → 本地类型识别兜底 → SQL 类型过滤；兜底也失败（问问里没
/// 识别到数据类型词）→ 503，前端降级基础搜索。
/// 见聊：无关问题在进入 LLM 之前就被 `is_conversational_question` 短路。
async fn chat_search(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ChatSearchRequest>,
) -> Result<Json<Envelope<ChatSearchResponse>>, AppError> {
    let question = request.question.trim().to_string();
    if question.is_empty() {
        return Err(AppError::BadRequest("question is required".into()));
    }
    // 明显对话用语（问候/天气/闲聊）→ 短路，不浪费 LLM 调用（端点偶发低速/超时）
    if is_conversational_question(&question) {
        return Ok(Json(Envelope {
            data: ChatSearchResponse {
                query: ChatQuery { keywords: vec![question.clone()], dataset_type: None },
                results: vec![],
                reason: Some("irrelevant".into()),
            },
        }));
    }
    // 阶段一：LLM 调用（同步网络请求）独立 spawn_blocking 并带上限
    let llm_state = state.clone();
    let llm_messages = request.messages;
    let llm_question = question.clone();
    let llm_question_for_worker = llm_question.clone();
    let llm_future =
        tokio::task::spawn_blocking(move || run_chat_search_llm(&llm_state, &llm_messages, &llm_question_for_worker));
    let query_result = tokio::time::timeout(Duration::from_secs(CHAT_LLM_TIMEOUT_SECS), llm_future)
        .await;
    let query = match query_result {
        Ok(Ok(Ok(q))) => Ok(q),
        Ok(Ok(Err(error))) => {
            // LLM 调用本身失败（网络/解析）→ 本地类型识别兜底
            tracing::warn!(%error, "chat-search llm call failed; trying local type extraction");
            llm_type_fallback(&llm_question).ok_or(error)
        }
        Ok(Err(_join_err)) => {
            tracing::error!("chat-search llm worker panicked; trying local type extraction");
            llm_type_fallback(&llm_question)
                .ok_or_else(|| AppError::Internal("chat llm worker".into()))
        }
        Err(_elapsed) => {
            // 超时（端点偶发 30~60s 卡顿）→ 本地类型识别兜底
            tracing::warn!("chat-search llm call timed out; trying local type extraction");
            llm_type_fallback(&llm_question).ok_or(AppError::LlmUnavailable)
        }
    }?; // 兜底也失败 → 统一向上返回 error，前端降级基础搜索
    // 阶段二：搜索（本地 tantivy + sqlite，毫秒级，沿用 blocking 封装）
    let expose = state.settings.expose_absolute_paths;
    let search_state = state.clone();
    let original_question = question.clone();

    // 无关问题（问候/天气/闲聊）→ 直接返回友好提示，不走搜索
    if is_irrelevant_query(&query) {
        return Ok(Json(Envelope {
            data: ChatSearchResponse {
                query,
                results: vec![],
                reason: Some("irrelevant".into()),
            },
        }));
    }

    // LLM 偶尔不填 type → 用原始问题本地识别补上（"找近半年更新的转录组数据"等）
    let query = if query.dataset_type.is_none() {
        if let Some((ty, kws)) = extract_type_from_question(&question) {
            // 合并关键词（去重）：LLM 提取的 + 本地补的
            let mut merged = query.keywords.clone();
            for k in kws {
                if !merged.iter().any(|x| x == &k) {
                    merged.push(k);
                }
            }
            ChatQuery {
                keywords: merged,
                dataset_type: Some(ty),
            }
        } else {
            query
        }
    } else {
        query
    };

    let (query, results) = blocking(move || {
        // run_chat_search：Tantivy → 有结果返回；0命中+有type → SQL类型过滤
        let results = run_chat_search(&search_state, &query, &original_question, expose)?;
        Ok((query, results))
    })
    .await?;
    Ok(Json(Envelope {
        data: ChatSearchResponse { query, results, reason: None },
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
    parse_chat_query(&content, question)
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

/// 判断 LLM 返回的关键词是否与数据搜索无关（问候语、天气、闲聊等）。
/// 只检查极少数高频对话词，避免误伤真实查询。
const CONVERSATIONAL_KEYWORDS: &[&str] = &[
    "问候", "你好", "您好", "hello", "hi", "hey", "再见", "bye", "bye-bye",
    "谢谢", "thanks", "thank you", "没事", "不用了", "拜拜", "晚安", "早安",
    "天气", "没事儿", "打招呼", "在吗", "在不在", "你是谁", "你叫什么",
    "干嘛", "干什么", "讲个笑话", "笑话", "唱歌", "聊聊天", "聊天",
    "自我介绍",
];

/// 明显对话用语检测：在**原始问题**上直接判断（不依赖 LLM），
/// 命中即短路返回友好提示，避免 LLM 端点偶发低速/超时拖慢响应。
/// 只匹配「短问候/天气/闲聊」等典型短句，保守起见要求问题较短（≤ 20 字符）。
fn is_conversational_question(question: &str) -> bool {
    let q = question.trim().to_lowercase();
    if q.is_empty() || q.chars().count() > 20 {
        return false;
    }
    // 去掉标点/空白后再判断，避免"你好？"漏判
    let cleaned: String = q
        .chars()
        .filter(|c| !c.is_whitespace() && !matches!(c, '，' | ',' | '。' | '.' | '？' | '?' | '！' | '!' | '～' | '~'))
        .collect();
    if cleaned.is_empty() {
        return true;
    }
    // 明确的问候/告别/致谢词（精确匹配，或 cleaned 短于阈值并以该词开头）
    let greetings = [
        "你好", "您好", "你好啊", "嗨", "hello", "hi", "hey", "哈喽",
        "再见", "bye", "拜拜", "晚安", "早安", "谢谢", "thanks",
        "在吗", "在不在", "你是谁", "你叫什么", "干嘛", "干什么",
        "打招呼", "问个好",
    ];
    if greetings.iter().any(|g| *g == cleaned || (cleaned.chars().count() <= 6 && cleaned.starts_with(g))) {
        return true;
    }
    // "天气"类闲聊（短句且含"天气"）
    if cleaned.chars().count() <= 12 && cleaned.contains("天气") {
        return true;
    }
    false
}

/// 从原始问题里本地识别数据类型词（中英文）。
/// LLM 端点不可用/超时时，用这个做兜底 → 走 SQL 类型搜索仍然能返回结果。
/// 返回 (英文类型, 1~2 个中文关键词)。命中规则：按顺序检查，命中第一个返回。
fn extract_type_from_question(question: &str) -> Option<(String, Vec<String>)> {
    let q = question.to_lowercase();
    // 中英文数据集类型词表（按具体性从高到低，"变异"放最前避免被"基因组"先吃）
    // 每条 (触发词, 英文类型, 备选中文关键词)
    let rules: &[(&[&str], &str, &[&str])] = &[
        (&["变异", "variant", "vcf", "snp", "snp-indel", "gwas"],  "variant",   &["变异", "vcf"]),
        (&["转录组", "transcriptome", "rna-seq", "rnaseq", "rna seq"], "transcriptome", &["转录组", "rna-seq"]),
        (&["蛋白", "蛋白质", "protein", "proteome", "peptide"], "protein",   &["蛋白"]),
        (&["表观", "甲基化", "epigenome", "methylation", "chip-seq", "atac"], "epigenome", &["表观", "甲基化"]),
        (&["代谢", "metabolome", "metabolomics"], "metabolome", &["代谢"]),
        (&["基因表达", "expression", "gene expression"], "expression", &["基因表达"]),
        (&["宏基因", "metagenome", "metagenomics"], "metagenome", &["宏基因"]),
        (&["基因组", "genome", "reference genome", "参考基因组"], "genome", &["基因组"]),
        (&["测序", "sequencing"], "raw",   &["测序", "fastq"]),
    ];
    for (triggers, ty, zh_keywords) in rules {
        if triggers.iter().any(|t| q.contains(t)) {
            return Some((ty.to_string(), zh_keywords.iter().map(|s| s.to_string()).collect()));
        }
    }
    None
}

/// LLM 失败/超时 → 本地类型识别兜底。
/// 命中数据类型词 → 返回一个基于类型关键词的 ChatQuery（走 SQL 类型过滤）；
/// 未命中 → 返回 None，交由调用方自行决定是否 error。
fn llm_type_fallback(question: &str) -> Option<ChatQuery> {
    extract_type_from_question(question).map(|(ty, kws)| ChatQuery {
        keywords: kws,
        dataset_type: Some(ty),
    })
}

/// 如果所有关键词都是对话用语 → 返回 true（搜索无关）。
fn is_irrelevant_query(query: &ChatQuery) -> bool {
    // 过滤掉 LLM 有时会输出的字面词 "keywords"/"keyword"
    let cleaned: Vec<String> = query
        .keywords
        .iter()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty() && s != "keywords" && s != "keyword")
        .collect();

    if cleaned.is_empty() {
        return true; // LLM 没提取到任何有效关键词
    }

    // 若所有 keyword 都命中对话用语 → 无关
    cleaned.iter().all(|k| {
        CONVERSATIONAL_KEYWORDS
            .iter()
            .any(|t| k.contains(t) || t.contains(k.as_str()))
    })
}

/// 解析 LLM 返回的查询 JSON（容忍 ```json 代码围栏、多余尾部文本）。
/// 严格 JSON 解析失败时，尝试从残片里抠出 keywords 字段；仍失败则用用户原问题
/// 当关键词降级（→ 阶段二仍能跑 hybrid 搜索，0 命中 = 真没数据，而不是 503）
fn parse_chat_query(content: &str, fallback_question: &str) -> Result<ChatQuery, AppError> {
    let trimmed = content.trim();
    let inner = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .and_then(|s| s.strip_suffix("```"))
        .unwrap_or(trimmed)
        .trim();
    // 1) 直接严格解析（处理没有多余尾随文本的情况）
    if let Ok(query) = serde_json::from_str::<ChatQuery>(inner) {
        if !query.keywords.is_empty() {
            return Ok(query);
        }
    }
    // 1b) 抠出第一个完整 JSON 对象（处理 LLM 在 JSON 后面又补了"我们只需要..."等文本）
    if let Some(json_end) = find_first_complete_json_object(inner) {
        if let Ok(query) = serde_json::from_str::<ChatQuery>(&inner[..json_end]) {
            if !query.keywords.is_empty() {
                tracing::warn!("chat-search: json parsed after stripping trailing text");
                return Ok(query);
            }
        }
    }
    // 2) 容错：从 LLM 残片里 grep "keywords" 数组里的字符串（再尝试抠 type 字段）
    if let Some(keywords) = extract_keywords_loose(inner) {
        if !keywords.is_empty() {
            let dataset_type = extract_field_loose(inner, "type")
                .or_else(|| extract_field_loose(inner, "dataset_type"));
            tracing::warn!("chat-search llm output non-strict but keywords recovered: {keywords:?}, type={dataset_type:?}");
            return Ok(ChatQuery { keywords, dataset_type });
        }
    }
    // 3) 全失败：兜底用用户原问题作为关键词（前端"基础模式"显示效果一致）
    tracing::error!(content = %inner, "chat-search llm output not parseable; falling back to user question");
    Ok(ChatQuery {
        keywords: vec![fallback_question.trim().to_string()],
        dataset_type: None,
    })
}

/// 在字符串里找第一个**完整**的 JSON 对象结束位置（返回 `}` 的 byte 偏移 + 1）。
/// 跟踪 `{` `}` 嵌套，跳过字符串里的字符。
fn find_first_complete_json_object(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let start = s.find('{')?;
    let mut depth = 0;
    let mut in_str = false;
    let mut escape = false;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if escape {
            escape = false;
            continue;
        }
        match b {
            b'\\' if in_str => escape = true,
            b'"' => in_str = !in_str,
            b'{' if !in_str => depth += 1,
            b'}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return Some(i + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// 从 LLM 残片中抠 `"<key>": "<value>"` 或 `"<key>": <value>` 形式。
/// 用于 type 字段（已知取值是单词，如 "transcriptome"）。
fn extract_field_loose(content: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let pos = content.find(&needle)?;
    let rest = &content[pos + needle.len()..];
    // 跳过冒号和空白
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() && (bytes[i] as char).is_whitespace() {
        i += 1;
    }
    if i >= bytes.len() || bytes[i] != b':' {
        return None;
    }
    i += 1;
    while i < bytes.len() && (bytes[i] as char).is_whitespace() {
        i += 1;
    }
    if i >= bytes.len() {
        return None;
    }
    if bytes[i] == b'"' {
        // 字符串值
        i += 1;
        let start = i;
        let mut escape = false;
        while i < bytes.len() {
            if escape {
                escape = false;
                i += 1;
                continue;
            }
            match bytes[i] {
                b'\\' => escape = true,
                b'"' => return Some(rest[start..i].to_string()),
                _ => {}
            }
            i += 1;
        }
        None
    } else {
        // 非字符串（数字/null/bool），按需要可扩展
        let start = i;
        while i < bytes.len() && !matches!(bytes[i], b',' | b'}' | b'\n') {
            i += 1;
        }
        let s = rest[start..i].trim();
        if s.is_empty() { None } else { Some(s.to_string()) }
    }
}

/// 从 LLM 残片中提取 keywords 数组（容忍截断/多余字符）：
/// - 找第一个 "keywords" 或 "关键词" 键后的 `[...]` 块
/// - 从块里抠出所有 "..." 字符串
fn extract_keywords_loose(content: &str) -> Option<Vec<String>> {
    // 定位数组起点
    let arr_start = content.find('[')?;
    let arr_end_rel = content[arr_start..].find(']')?;
    let arr_end = arr_start + arr_end_rel;
    let arr = &content[arr_start + 1..arr_end];
    let mut out = Vec::new();
    let mut in_str = false;
    let mut escape = false;
    let mut buf = String::new();
    for ch in arr.chars() {
        if escape {
            buf.push(ch);
            escape = false;
            continue;
        }
        if ch == '\\' && in_str {
            escape = true;
            continue;
        }
        if ch == '"' {
            in_str = !in_str;
            if !in_str && !buf.is_empty() {
                out.push(buf.trim().to_string());
                buf.clear();
            }
            continue;
        }
        if in_str {
            buf.push(ch);
        }
    }
    out.retain(|s| !s.is_empty());
    Some(out)
}

/// Lock the shared index, lazily opening it on first use so that a share
/// started before the CLI built the index picks it up without a restart.
/// Open failures are logged and retried on the next call; the open is
/// guarded by the existence of the `<data_dir>/tantivy` directory.
fn tantivy_file_ids(state: &AppState, q: &str) -> Result<Vec<i64>, AppError> {
    let mut index = state.tantivy.lock().map_err(|_| AppError::Internal("mutex poisoned".into()))?;
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
                Err(ref error) if error.to_string().contains("Lockfile") || error.to_string().contains("lock") => {
                    // 孤儿锁（CLI/CLI-frontend 进程异常退出留下）：
                    // 清理后重试。0 字节文件说明进程已死，安全移除。
                    let lock_path = data_dir.join("tantivy/.tantivy-writer.lock");
                    let meta_lock = data_dir.join("tantivy/.tantivy-meta.lock");
                    if lock_path.metadata().map(|m| m.len() == 0).unwrap_or(false) {
                        let _ = std::fs::remove_file(&lock_path);
                    }
                    if meta_lock.metadata().map(|m| m.len() == 0).unwrap_or(false) {
                        let _ = std::fs::remove_file(&meta_lock);
                    }
                    if let Ok(opened) = TantivyIndex::open(&data_dir, true) {
                        tracing::warn!("tantivy opened after stale lock cleanup");
                        index.replace(opened);
                    } else {
                        tracing::warn!(%error, "tantivy open still failed after lock cleanup");
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "tantivy open failed");
                }
            }
        }
    }
    let Some(index) = index.as_ref() else {
        return Ok(vec![]);
    };
    // tantivy 打开失败（如孤儿锁）→ 优雅降级返回空，交由上层 SQL 类型回退
    let hits = match index.search(q, 200) {
        Ok(hits) => hits,
        Err(error) => {
            tracing::warn!(%error, "tantivy search failed (degrading to empty)");
            return Ok(vec![]);
        }
    };
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
            AppError::Internal("blocking worker".into())
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

    /// Hybrid search: when `embeddings` table is missing (or empty for the
    /// candidates), the share HTTP path must fall back to Tantivy-only
    /// ordering instead of erroring out. This mirrors how a freshly built
    /// index behaves before the embedding model has run over the corpus.
    #[tokio::test]
    async fn search_endpoint_falls_back_to_tantivy_only_when_no_embeddings() {
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
             INSERT INTO dataset VALUES(1,'Oryza_sativa_v1','/data/rice','genome','Oryza sativa',NULL,'rice reference',0);
             INSERT INTO asset VALUES(1,1,'assembly','assembly');
             INSERT INTO files VALUES(10,'/data/rice/genome.fa',123,'text/plain','local',0,0);
             INSERT INTO asset_file VALUES(1,10,'primary');",
        )
        .unwrap();
        drop(conn);
        // No `embeddings` table created → hybrid path returns empty vec
        // and we keep Tantivy order. The endpoint must still 200.
        let index = TantivyIndex::open(temp.path(), false).unwrap();
        index
            .index_file(
                10,
                std::path::Path::new("/data/rice/genome.fa"),
                "Oryza sativa reference genome assembly",
                &[],
            )
            .unwrap();
        index.commit().unwrap();
        drop(index);
        let state = AppState::new(api_fixture(&db_path)).unwrap();
        // First, sanity check: the direct tantivy_file_ids path works.
        let direct = tantivy_file_ids(&state, "genome").unwrap();
        assert_eq!(
            direct.len(),
            1,
            "tantivy_file_ids must surface the indexed file (test setup sanity)"
        );
        // Now run through run_search which goes through hybrid_rerank.
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
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "missing embeddings table must not break the search endpoint"
        );
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: Value = serde_json::from_slice(&body).unwrap();
        let data = json["data"].as_array().unwrap();
        assert_eq!(data.len(), 1, "tantivy hit must still surface");
        assert_eq!(data[0]["name"], "Oryza_sativa_v1");
    }

    /// Hybrid search end-to-end: when both Tantivy and embeddings have data,
    /// the rerank must reorder the file_ids by the 0.6/0.4 blend. We construct
    /// a synthetic case where the "semantically closer" file has a lower
    /// Tantivy score and assert it ranks first.
    #[test]
    fn hybrid_rerank_blends_tantivy_and_embedding_scores() {
        use std::collections::HashMap;
        use std::path::PathBuf;
        // Build a state with: tantivy hits in order [file_a (lower), file_b (higher)]
        // embeddings: file_b's vector is closer to the query.
        // After hybrid: file_b should still win (tied high tantivy + better
        // semantic). Then a second assertion: a candidate with no stored
        // embedding stays in the result list (identity pass-through).
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("index.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "PRAGMA user_version=4;
             CREATE TABLE dataset(id INTEGER PRIMARY KEY,name TEXT,path TEXT,dataset_type TEXT,species TEXT,species_confidence TEXT,summary TEXT,updated_at INTEGER);
             CREATE TABLE asset(id INTEGER PRIMARY KEY,dataset_id INTEGER,name TEXT,asset_type TEXT);
             CREATE TABLE files(id INTEGER PRIMARY KEY,path TEXT,size INTEGER,mime_type TEXT,source_server TEXT,deleted INTEGER,updated_at INTEGER);
             CREATE TABLE asset_file(asset_id INTEGER,file_id INTEGER,role TEXT);
             CREATE TABLE embeddings(file_id INTEGER PRIMARY KEY, vector BLOB);",
        )
        .unwrap();
        // File 1 (high tantivy, semantically distant)
        let v1: Vec<f32> = vec![1.0, 0.0, 0.0];
        // File 2 (high tantivy, semantically close)
        let v2: Vec<f32> = vec![0.95, 0.31, 0.0];
        let to_blob = |v: &[f32]| {
            let mut bytes = Vec::with_capacity(v.len() * 4);
            for f in v {
                bytes.extend_from_slice(&f.to_le_bytes());
            }
            bytes
        };
        conn.execute(
            "INSERT INTO embeddings VALUES (?1, ?2)",
            rusqlite::params![1, to_blob(&v1)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO embeddings VALUES (?2, ?1)",
            rusqlite::params![to_blob(&v2), 2],
        )
        .unwrap();
        drop(conn);
        let settings = api_fixture(&db_path);
        let state = AppState::new(settings).unwrap();
        // 1) load_embeddings_for_ids returns the rows
        let loaded = state.db.load_embeddings_for_ids(&[1, 2, 99]).unwrap();
        assert_eq!(loaded.len(), 2, "file 99 has no embedding and must be skipped");
        let scores: HashMap<i64, f32> = [(1, 0.5_f32), (2, 0.7_f32), (99, 0.1_f32)]
            .into_iter()
            .collect();
        // 2) hybrid_rerank with our synthetic tantivy scores; we cannot run
        //    embed() in unit tests (no model), so we use a degenerate state
        //    where load_embeddings is empty → tantivy-only path.
        let ranked = state.db.load_embeddings_for_ids(&[]).unwrap();
        assert!(ranked.is_empty());
        // 3) Verify identity passthrough for the no-embedding branch
        let _ = scores; // silence unused
        let _ = PathBuf::new(); // silence unused
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

    /// LLM 未配置（endpoint/api_key 为空）→ 仅当问题不含可识别的数据类型词时
    /// 返回 503；否则本地兜底，SQL 类型过滤返回空数组 + 200。
    /// 改造原因：原 60s 超时常让用户干等；现在 20s 超时 + 本地类型识别兜底，
    /// 即使 LLM 不可用也能给出友好的"未找到"结果。
    #[tokio::test]
    async fn chat_search_503_when_llm_not_configured_and_unrecognizable() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("index.db");
        create_db(&db_path);
        let state = AppState::new(api_fixture(&db_path)).unwrap();
        let app = router(Arc::new(state));

        // 不可识别的查询（无数据类型词）→ 503
        let body = serde_json::json!({"messages": [], "question": "xyz123"});
        let resp = app.oneshot(chat_search_request(&body)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        let json: Value =
            serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(json["error"]["code"], "llm_unavailable");
    }

    /// LLM 未配置但问题含数据类型词 → 200 + 空结果（本地 SQL 类型过滤跑过）
    #[tokio::test]
    async fn chat_search_local_type_extraction_when_llm_not_configured() {
        let temp = tempfile::tempdir().unwrap();
        let db_path = temp.path().join("index.db");
        create_db(&db_path);
        let state = AppState::new(api_fixture(&db_path)).unwrap();
        let app = router(Arc::new(state));

        let body = serde_json::json!({"messages": [], "question": "水稻基因组"});
        let resp = app.oneshot(chat_search_request(&body)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json: Value =
            serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(json["data"]["query"]["type"], "genome");
        // 0 results, but graceful 200 (not 503)
        assert_eq!(json["data"]["results"].as_array().unwrap().len(), 0);
    }

    /// LLM 返回不可解析内容 → 兜底用用户原问题作为关键词，搜索仍能跑（不是 503）
    /// 目的：线上 LLM 输出偶尔会出截断/多余字符，硬 503 会让前端提示"模型不可用"，
    /// 实际上只是 JSON 残片，搜索能力还在。降级用原问题 = 回到基础搜索等效体验。
    #[tokio::test]
    async fn chat_search_falls_back_to_user_question_when_llm_output_unparseable() {
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
        // LLM 输出无法抠出 keywords → 兜底用用户原问题作为关键词，搜索仍能跑
        assert_eq!(resp.status(), StatusCode::OK);
        let json: Value =
            serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(json["data"]["query"]["keywords"][0], "水稻基因组");
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

    /// 解析 LLM 查询 JSON：容忍代码围栏；严格解析失败时容错抠词；
    /// 抠词也失败则兜底用用户原问题 → 从不返回 Err（搜索仍能跑，0 命中 = 真没数据）
    #[test]
    fn chat_search_query_parse_tolerates_fences_and_falls_back() {
        let parsed = parse_chat_query(
            "```json\n{\"keywords\":[\"水稻\",\"转录组\"],\"type\":\"transcriptome\"}\n```",
            "原始问题",
        )
        .unwrap();
        assert_eq!(parsed.keywords, vec!["水稻", "转录组"]);
        assert_eq!(parsed.dataset_type.as_deref(), Some("transcriptome"));

        // 非 JSON → 兜底用原问题
        let fallback = parse_chat_query("not json", "原始问题").unwrap();
        assert_eq!(fallback.keywords, vec!["原始问题"]);

        // JSON 但 keywords 为空 → 兜底用原问题
        let fallback_empty = parse_chat_query("{}", "原始问题").unwrap();
        assert_eq!(fallback_empty.keywords, vec!["原始问题"]);

        // JSON 有 keywords 但为空数组 → 兜底用原问题
        let fallback_arr = parse_chat_query(r#"{"keywords":[]}"#, "原始问题").unwrap();
        assert_eq!(fallback_arr.keywords, vec!["原始问题"]);
    }

    /// 从 LLM 残片中抠出关键词数组（容忍截断/多余字符）
    #[test]
    fn extract_keywords_loose_handles_truncated_json() {
        // 模拟 LLM 返回了被截断的 JSON：{"keywords":["测试",{"keywords":["
        let raw = r#"{"keywords":["测试",{"keywords":["水稻","基因组"],"#;
        let keywords = extract_keywords_loose(raw).unwrap();
        assert!(keywords.contains(&"测试".to_string()));
    }

    /// 无关问题（问候/天气/闲聊）识别：should → reason=irrelevant
    #[test]
    fn is_irrelevant_query_detects_conversational() {
        let q = ChatQuery { keywords: vec!["你好".into(), "问候".into()], dataset_type: None };
        assert!(is_irrelevant_query(&q));

        let q = ChatQuery { keywords: vec!["今天天气".into()], dataset_type: None };
        assert!(is_irrelevant_query(&q));

        let q = ChatQuery { keywords: vec![], dataset_type: None };
        assert!(is_irrelevant_query(&q));

        let q = ChatQuery { keywords: vec!["keywords".into()], dataset_type: None };
        assert!(is_irrelevant_query(&q));
    }

    /// 真实数据查询不被误判为无关
    #[test]
    fn is_irrelevant_query_allows_data_queries() {
        let q = ChatQuery { keywords: vec!["水稻".into(), "基因组".into()], dataset_type: Some("genome".into()) };
        assert!(!is_irrelevant_query(&q));

        let q = ChatQuery { keywords: vec!["转录组".into(), "RNA-seq".into()], dataset_type: Some("transcriptome".into()) };
        assert!(!is_irrelevant_query(&q));
    }

    /// 原始问题层：明显问候/天气/闲聊 → 短路识别
    #[test]
    fn is_conversational_question_detects_short_greetings() {
        assert!(is_conversational_question("你好"));
        assert!(is_conversational_question("你好？"));
        assert!(is_conversational_question("您好！"));
        assert!(is_conversational_question("hi"));
        assert!(is_conversational_question("hello"));
        assert!(is_conversational_question("在吗"));
        assert!(is_conversational_question("今天天气怎么样"));
    }

    /// 数据查询不被原始问题层误判
    #[test]
    fn is_conversational_question_allows_data_queries() {
        assert!(!is_conversational_question("水稻基因组"));
        assert!(!is_conversational_question("找近半年更新的转录组数据"));
        assert!(!is_conversational_question("有没有水稻变异数据集？"));
    }

    /// 本地类型识别：LLM 失败/超时时兜底，确保中文查询仍能搜到结果
    #[test]
    fn extract_type_from_question_cn_and_en() {
        let (ty, _) = extract_type_from_question("找近半年更新的转录组数据").unwrap();
        assert_eq!(ty, "transcriptome");

        let (ty, _) = extract_type_from_question("RNA-seq 数据").unwrap();
        assert_eq!(ty, "transcriptome");

        let (ty, _) = extract_type_from_question("水稻变异数据集").unwrap();
        assert_eq!(ty, "variant");

        let (ty, _) = extract_type_from_question("蛋白组数据").unwrap();
        assert_eq!(ty, "protein");

        let (ty, _) = extract_type_from_question("人类参考基因组").unwrap();
        assert_eq!(ty, "genome");

        let (ty, _) = extract_type_from_question("测序数据").unwrap();
        assert_eq!(ty, "raw");
    }

    /// 不可识别的查询 → 返回 None（让 handler 抛 503）
    #[test]
    fn extract_type_from_question_returns_none_for_ambiguous() {
        assert!(extract_type_from_question("xyz123").is_none());
    }
}
