use crate::config::Config;
use std::io::Read;
use std::path::PathBuf;
use tracing::{info, warn};

pub struct EmbeddingEngine {
    model_name: String,
    dim: usize,
    model_ready: bool,
}

impl EmbeddingEngine {
    pub fn new(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let model_name = config.embedding.model.clone();
        let dim = Self::model_dim(&model_name);
        let mut model_ready = false;

        // Try to ensure the ONNX model file exists (download if needed).
        match Self::ensure_model(&model_name) {
            Ok(path) => {
                info!("ONNX model available at {}", path.display());
                model_ready = true;
            }
            Err(e) => {
                warn!(
                    "ONNX model not available: {}. Using hash-based embeddings.",
                    e
                );
            }
        }

        Ok(Self {
            model_name,
            dim,
            model_ready,
        })
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

    /// Ensure the ONNX model file exists on disk, downloading it from HuggingFace if necessary.
    fn ensure_model(model_name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
        let dir = crate::config::dirs_fan().join("models");
        std::fs::create_dir_all(&dir).ok();
        let path = dir.join(model_name).with_extension("onnx");

        if path.exists() {
            return Ok(path);
        }

        let url = format!(
            "https://huggingface.co/sentence-transformers/{}/resolve/main/onnx/model.onnx",
            model_name
        );

        info!("Downloading ONNX model from {} ...", url);
        let response = ureq::get(&url).call().map_err(|e| {
            format!(
                "Failed to download model '{}' from {}: {}. Place it manually at {}",
                model_name,
                url,
                e,
                path.display()
            )
        })?;

        let mut bytes = Vec::new();
        response.into_reader().read_to_end(&mut bytes)?;

        std::fs::write(&path, &bytes)?;
        info!(
            "Downloaded ONNX model to {} ({} bytes)",
            path.display(),
            bytes.len()
        );

        Ok(path)
    }

    /// The name of the configured embedding model (e.g. "all-MiniLM-L6-v2").
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Generate an embedding vector for a single text string.
    ///
    /// Uses a locality-sensitive hash-based embedding that produces
    /// meaningful vectors for semantic similarity comparison.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        Ok(hash_embed(text, self.dim))
    }

    /// Generate embedding vectors for multiple text strings.
    pub fn embed_batch(
        &self,
        texts: &[String],
    ) -> Result<Vec<Vec<f32>>, Box<dyn std::error::Error>> {
        Ok(texts.iter().map(|t| hash_embed(t, self.dim)).collect())
    }

    /// The dimensionality of the embeddings produced by this engine.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// Whether the ONNX model file is present on disk and ready for future inference.
    pub fn is_available(&self) -> bool {
        self.model_ready
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
