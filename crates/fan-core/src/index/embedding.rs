use crate::config::Config;
use candle_core::{Device, Tensor};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::{info, warn};

pub struct EmbeddingEngine {
    /// Lazy-loaded model state. None until first embed() call.
    state: Mutex<Option<ModelState>>,
    model_name: String,
    dim: usize,
}

struct ModelState {
    model: Arc<Mutex<OnnxModel>>,
    tokenizer: Arc<tokenizers::Tokenizer>,
}

struct OnnxModel {
    proto: candle_onnx::onnx::ModelProto,
    input_names: Vec<String>,
    output_name: String,
    max_seq_len: usize,
}

impl EmbeddingEngine {
    /// Create the engine without loading the model yet.
    /// The ONNX model (~90MB) is loaded lazily on first embed() call.
    pub fn new(config: &Config) -> Result<Self, Box<dyn std::error::Error>> {
        let model_name = config.embedding.model.clone();
        let dim = Self::model_dim(&model_name);

        Ok(Self {
            state: Mutex::new(None),
            model_name,
            dim,
        })
    }

    /// Load the model if not already loaded. Called internally before embedding.
    fn ensure_loaded(&self) -> Result<(), Box<dyn std::error::Error>> {
        let mut state = self.state.lock().unwrap();
        if state.is_some() {
            return Ok(());
        }

        let model_dir = crate::config::dirs_fan().join("models").join(&self.model_name);
        std::fs::create_dir_all(&model_dir).ok();

        // Ensure model files exist
        Self::ensure_files(&self.model_name, &model_dir)?;

        // Load tokenizer
        let tokenizer_path = model_dir.join("tokenizer.json");
        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| format!("Failed to load tokenizer: {}", e))?;

        // Load ONNX model
        let onnx_path = model_dir.join("model.onnx");
        let proto = candle_onnx::read_file(&onnx_path)
            .map_err(|e| format!("Failed to load ONNX model: {}", e))?;

        // Discover input/output names
        let graph = proto.graph.as_ref().ok_or("ONNX model has no graph")?;
        let input_names: Vec<String> = graph.input.iter().map(|vi| vi.name.clone()).collect();
        let output_name = graph.output.first()
            .map(|vi| vi.name.clone())
            .unwrap_or_else(|| "last_hidden_state".to_string());

        info!(
            "Loaded ONNX model '{}' ({} dims). Inputs: {:?}, Output: '{}'",
            self.model_name, self.dim, input_names, output_name
        );

        *state = Some(ModelState {
            model: Arc::new(Mutex::new(OnnxModel {
                proto,
                input_names,
                output_name,
                max_seq_len: 128,
            })),
            tokenizer: Arc::new(tokenizer),
        });

        Ok(())
    }

    fn ensure_files(model_name: &str, model_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let onnx_path = model_dir.join("model.onnx");
        let tok_path = model_dir.join("tokenizer.json");

        if !onnx_path.exists() {
            let url = format!(
                "https://huggingface.co/sentence-transformers/{}/resolve/main/onnx/model.onnx",
                model_name
            );
            info!("Downloading ONNX model from {}...", url);
            if let Err(e) = download_file(&url, &onnx_path) {
                let mirror_url = format!(
                    "https://hf-mirror.com/sentence-transformers/{}/resolve/main/onnx/model.onnx",
                    model_name
                );
                download_file(&mirror_url, &onnx_path)
                    .map_err(|e2| format!("Download failed: {} | {}", e, e2))?;
            }
        }

        if !tok_path.exists() {
            let url = format!(
                "https://huggingface.co/sentence-transformers/{}/resolve/main/tokenizer.json",
                model_name
            );
            if let Err(e) = download_file(&url, &tok_path) {
                let mirror_url = format!(
                    "https://hf-mirror.com/sentence-transformers/{}/resolve/main/tokenizer.json",
                    model_name
                );
                download_file(&mirror_url, &tok_path)
                    .map_err(|e2| format!("Download failed: {} | {}", e, e2))?;
            }
        }

        Ok(())
    }

    fn model_dim(model_name: &str) -> usize {
        match model_name {
            "all-MiniLM-L6-v2" => 384,
            "all-mpnet-base-v2" => 768,
            "gte-small" => 384,
            "gte-base" => 768,
            other => {
                warn!("unknown embedding model '{}', defaulting to 384 dims", other);
                384
            }
        }
    }

    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Generate an embedding vector. Loads the model on first call.
    pub fn embed(&self, text: &str) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        match self.ensure_loaded() {
            Ok(()) => {
                let state = self.state.lock().unwrap();
                let state = state.as_ref().unwrap();
                let model = state.model.lock().unwrap();
                Self::run_inference(&model, &state.tokenizer, text)
            }
            Err(e) => {
                warn!("Failed to load embedding model: {}. Using hash fallback.", e);
                Ok(hash_embed(text, self.dim))
            }
        }
    }

    pub fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, Box<dyn std::error::Error>> {
        match self.ensure_loaded() {
            Ok(()) => {
                let state = self.state.lock().unwrap();
                let state = state.as_ref().unwrap();
                let model = state.model.lock().unwrap();
                texts.iter().map(|t| Self::run_inference(&model, &state.tokenizer, t)).collect()
            }
            Err(e) => {
                warn!("Failed to load embedding model: {}. Using hash fallback.", e);
                Ok(texts.iter().map(|t| hash_embed(t, self.dim)).collect())
            }
        }
    }

    pub fn dim(&self) -> usize { self.dim }

    pub fn is_available(&self) -> bool {
        self.state.lock().unwrap().is_some()
    }

    fn run_inference(
        model: &OnnxModel,
        tokenizer: &tokenizers::Tokenizer,
        text: &str,
    ) -> Result<Vec<f32>, Box<dyn std::error::Error>> {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.len() < 2 {
            return Ok(hash_embed(text, 384));
        }

        let encoding = tokenizer.encode(text, false)
            .map_err(|e| format!("Tokenization error: {}", e))?;

        let ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        let mask: Vec<i64> = encoding.get_attention_mask().iter().map(|&m| m as i64).collect();
        let type_ids: Vec<i64> = encoding.get_type_ids().iter().map(|&t| t as i64).collect();

        let len = ids.len().min(model.max_seq_len);
        let ids = &ids[..len];
        let mask = &mask[..len];
        let type_ids = &type_ids[..len];

        let input_ids = Tensor::new(ids, &Device::Cpu)?.unsqueeze(0)?;
        let attention_mask = Tensor::new(mask, &Device::Cpu)?.unsqueeze(0)?;
        let token_type_ids = Tensor::new(type_ids, &Device::Cpu)?.unsqueeze(0)?;

        let mut inputs: HashMap<String, Tensor> = HashMap::new();
        for name in &model.input_names {
            if name == "input_ids" { inputs.insert("input_ids".into(), input_ids.clone()); }
            else if name == "attention_mask" { inputs.insert("attention_mask".into(), attention_mask.clone()); }
            else if name == "token_type_ids" { inputs.insert("token_type_ids".into(), token_type_ids.clone()); }
        }
        if inputs.is_empty() {
            inputs.insert("input_ids".into(), input_ids.clone());
            inputs.insert("attention_mask".into(), attention_mask.clone());
            inputs.insert("token_type_ids".into(), token_type_ids.clone());
        }

        let outputs = candle_onnx::simple_eval(&model.proto, inputs)
            .map_err(|e| format!("ONNX inference error: {}", e))?;

        let token_embeddings = outputs.get(&model.output_name)
            .ok_or_else(|| format!("Output '{}' not found", model.output_name))?;

        let emb_3d = match token_embeddings.dims().len() {
            3 => token_embeddings.clone(),
            2 => token_embeddings.unsqueeze(0)?,
            _ => return Err(format!("unexpected embedding shape: {:?}", token_embeddings.dims()).into()),
        };
        let dims = emb_3d.dims();
        let hidden_dim = dims[2];
        let seq_len = dims[1];

        let mask_len = attention_mask.dims()[1];
        let use_len = seq_len.min(mask_len);
        let attn_mask = attention_mask.narrow(1, 0, use_len)?;
        let emb = emb_3d.narrow(1, 0, use_len)?;

        let mask_f32 = attn_mask.to_dtype(candle_core::DType::F32)?;
        let mask_2d = mask_f32.unsqueeze(2)?;
        let mask_broadcast = mask_2d.broadcast_as(&[1usize, use_len, hidden_dim])?;
        let masked = emb.broadcast_mul(&mask_broadcast)?;

        let sum = masked.sum(1)?;
        let mask_count = mask_broadcast.sum(1)?;
        let eps = mask_count.ones_like()?.affine(1e-9, 0.0)?;
        let safe_count = mask_count.broadcast_add(&eps)?;
        let pooled = sum.broadcast_div(&safe_count)?;

        let pooled_1d = pooled.flatten_all()?;
        let norm_sq = pooled_1d.sqr()?.sum_all()?.sqrt()?;
        let norm_safe = norm_sq.maximum(&Tensor::new(&[1e-12f32], &Device::Cpu)?)?;
        let normalized = pooled_1d.broadcast_div(&norm_safe)?;

        Ok(normalized.to_vec1()?)
    }
}

/// Download a file from a URL to a local path using ureq.
fn download_file(url: &str, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let response = ureq::get(url).call()
        .map_err(|e| format!("HTTP request failed for {}: {}", url, e))?;
    let mut bytes = Vec::new();
    response.into_reader().read_to_end(&mut bytes)
        .map_err(|e| format!("Failed to read response body: {}", e))?;
    std::fs::write(path, &bytes)
        .map_err(|e| format!("Failed to write to {}: {}", path.display(), e))?;
    info!("Downloaded {} bytes to {}", bytes.len(), path.display());
    Ok(())
}

/// Simple locality-sensitive hash embedding (fallback).
fn hash_embed(text: &str, dim: usize) -> Vec<f32> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let text_lower = text.to_lowercase();
    let mut vec = vec![0.0f32; dim];
    let words: Vec<&str> = text_lower.split_whitespace().collect();
    if words.is_empty() { return vec; }

    for word in &words {
        let h1 = { let mut h = DefaultHasher::new(); word.hash(&mut h); h.finish() as usize };
        let h2 = { let mut h = DefaultHasher::new(); format!("{}_2", word).hash(&mut h); h.finish() as usize };
        let h3 = { let mut h = DefaultHasher::new(); format!("{}_3", word).hash(&mut h); h.finish() as usize };
        vec[h1 % dim] += 1.0;
        vec[h2 % dim] += 0.7;
        vec[h3 % dim] += 0.5;
    }

    let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 { vec.iter_mut().for_each(|x| *x /= norm); }
    vec
}
