use crate::config::Config;

#[allow(dead_code)]
pub struct EmbeddingEngine {
    model: String,
    _api_url: Option<String>,
    dim: usize,
}

impl EmbeddingEngine {
    pub fn new(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let dim = match config.embedding.model.as_str() {
            "all-MiniLM-L6-v2" => 384,
            "all-mpnet-base-v2" => 768,
            "gte-small" => 384,
            "gte-base" => 768,
            other => {
                tracing::warn!("unknown embedding model '{}', defaulting to 384 dims", other);
                384
            }
        };

        Ok(Self {
            model: config.embedding.model.clone(),
            _api_url: config.embedding.external_api_url.clone(),
            dim,
        })
    }

    /// Generate an embedding vector for a text string.
    /// Currently a placeholder that returns a zero vector of the configured dimension.
    /// Will be replaced with ONNX inference in Task 6.
    pub fn embed(&self, _text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        Ok(vec![0.0_f32; self.dim])
    }

    /// The dimensionality of the embeddings produced by this engine.
    pub fn dim(&self) -> usize {
        self.dim
    }
}
