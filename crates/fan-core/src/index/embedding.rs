use crate::config::Config;
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

pub struct EmbeddingEngine {
    model: Option<Arc<Mutex<TextEmbedding>>>,
    model_name: String,
    dim: usize,
    available: bool,
}

impl EmbeddingEngine {
    pub fn new(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let model_name = config.embedding.model.clone();
        let dim = Self::model_dim(&model_name);

        // Try to load the real embedding model.
        match Self::load_model(&model_name) {
            Ok(model) => {
                info!(
                    "Loaded embedding model '{}' ({} dims) via fastembed",
                    model_name, dim
                );
                Ok(Self {
                    model: Some(Arc::new(Mutex::new(model))),
                    model_name,
                    dim,
                    available: true,
                })
            }
            Err(e) => {
                warn!(
                    "Failed to load embedding model '{}': {}. Falling back to hash-based embeddings.",
                    model_name, e
                );
                Ok(Self {
                    model: None,
                    model_name,
                    dim,
                    available: false,
                })
            }
        }
    }

    fn load_model(model_name: &str) -> Result<TextEmbedding, Box<dyn std::error::Error>> {
        let model = match model_name {
            "all-MiniLM-L6-v2" => EmbeddingModel::AllMiniLML6V2,
            "all-mpnet-base-v2" => EmbeddingModel::AllMpnetBaseV2,
            "gte-small" => EmbeddingModel::BGESmallENV15,
            "gte-base" => EmbeddingModel::BGEBaseENV15,
            other => {
                return Err(format!("unsupported embedding model: {}", other).into());
            }
        };

        let cache_dir = crate::config::dirs_fan().join("models");
        std::fs::create_dir_all(&cache_dir).ok();

        let options = TextInitOptions::new(model)
            .with_show_download_progress(true)
            .with_cache_dir(cache_dir);

        info!(
            "Initializing fastembed model '{}' (model will be auto-downloaded if needed)...",
            model_name
        );

        let text_embedding = TextEmbedding::try_new(options)?;
        Ok(text_embedding)
    }

    fn model_dim(model_name: &str) -> usize {
        match model_name {
            "all-MiniLM-L6-v2" => 384,
            "all-mpnet-base-v2" => 768,
            "gte-small" => 384,
            "gte-base" => 768,
            other => {
                warn!(
                    "unknown embedding model '{}', defaulting to 384 dims",
                    other
                );
                384
            }
        }
    }

    /// The name of the configured embedding model (e.g. "all-MiniLM-L6-v2").
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Generate an embedding vector for a single text string.
    ///
    /// Uses real sentence-transformer inference if the model is available,
    /// otherwise falls back to locality-sensitive hash embedding.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        if let Some(ref model) = self.model {
            let mut model = model.lock().unwrap();
            let embeddings = model.embed(vec![text.to_string()], None)?;
            Ok(embeddings.into_iter().next().unwrap_or_else(|| vec![0.0; self.dim]))
        } else {
            Ok(hash_embed(text, self.dim))
        }
    }

    /// Generate embedding vectors for multiple text strings.
    ///
    /// Uses batched sentence-transformer inference if the model is available,
    /// otherwise falls back to per-text hash embedding.
    pub fn embed_batch(
        &self,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, Box<dyn std::error::Error>> {
        if let Some(ref model) = self.model {
            let mut model = model.lock().unwrap();
            let embeddings = model.embed(texts.to_vec(), None)?;
            Ok(embeddings)
        } else {
            Ok(texts.iter().map(|t| hash_embed(t, self.dim)).collect())
        }
    }

    /// The dimensionality of the embeddings produced by this engine.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Whether a real embedding model is loaded and ready for inference.
    pub fn is_available(&self) -> bool {
        self.available
    }
}

/// Simple locality-sensitive hash embedding.
///
/// Each word contributes to multiple dimensions via different hash functions,
/// producing a meaningful vector for semantic similarity comparison.
fn hash_embed(text: &str, dim: usize) -> Vec<f32> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let text_lower = text.to_lowercase();
    let mut vec = vec![0.0f32; dim];
    let words: Vec<&str> = text_lower.split_whitespace().collect();

    if words.is_empty() {
        return vec;
    }

    for word in &words {
        // Use multiple hash functions for pseudo-multi-dimensional projection
        let h1 = {
            let mut h = DefaultHasher::new();
            word.hash(&mut h);
            h.finish() as usize
        };
        let h2 = {
            let mut h = DefaultHasher::new();
            format!("{}_2", word).hash(&mut h);
            h.finish() as usize
        };
        let h3 = {
            let mut h = DefaultHasher::new();
            format!("{}_3", word).hash(&mut h);
            h.finish() as usize
        };

        vec[h1 % dim] += 1.0;
        vec[h2 % dim] += 0.7;
        vec[h3 % dim] += 0.5;
    }

    // L2 normalize
    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        vec.iter_mut().for_each(|x| *x /= norm);
    }

    vec
}
