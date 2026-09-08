use crate::{
    config::Settings,
    db::Database,
    models::{Facets, Stats},
};
use fan_core::{index::embedding::EmbeddingEngine, index::tantivy::TantivyIndex, llm::LlmClient};
use std::{sync::Mutex, time::Instant};

pub struct Cache<T> {
    pub loaded: Instant,
    pub value: T,
}
pub struct AppState {
    pub db: Database,
    pub settings: Settings,
    /// LLM 客户端（chat-search 用；未配置时 is_configured() 为 false → 503）
    pub llm: LlmClient,
    /// Full-text index, shared data dir with the SQLite db
    /// (`<data_dir>/tantivy`). Lazily opened on the first search so that a
    /// share started before the CLI built the index picks it up without a
    /// restart; None when the index does not exist yet.
    pub tantivy: Mutex<Option<TantivyIndex>>,
    /// Embedding engine for hybrid search (Tantivy + semantic reranking).
    /// Model is loaded lazily on first embed() call (90MB ONNX; no-op if missing).
    /// Shares the same data dir (`<data_dir>/models/<model_name>`) as the CLI.
    pub embedding: EmbeddingEngine,
    pub stats: Mutex<Option<Cache<Stats>>>,
    pub facets: Mutex<Option<Cache<Facets>>>,
}
impl AppState {
    pub fn new(settings: Settings) -> Result<Self, Box<dyn std::error::Error>> {
        let db = Database::open(&settings)?;
        // EmbeddingEngine::with_embedding_config is infallible at construction;
        // model loading (ONNX file) fails gracefully with a hash fallback.
        let embedding = EmbeddingEngine::with_embedding_config(settings.embedding.clone())?;
        Ok(Self {
            llm: LlmClient::new(settings.llm.clone()),
            db,
            settings,
            tantivy: Mutex::new(None),
            embedding,
            stats: Mutex::new(None),
            facets: Mutex::new(None),
        })
    }
}
