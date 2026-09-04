use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::Mutex;

use crate::errors::AppError;

pub const DEFAULT_MODEL_ID: &str = "multilingual-e5-small";
pub const DEFAULT_MODEL_VERSION: &str = "v1";
pub const DEFAULT_EMBEDDING_DIM: usize = 384;

/// Trait abstracting text embedding generation for screenshots and queries.
pub trait TextEmbeddingEngine: Send + Sync {
    /// Unique identifier for the embedding model (e.g. "multilingual-e5-small").
    fn model_id(&self) -> &str;

    /// Version string for the embedding model (e.g. "v1").
    fn model_version(&self) -> &str;

    /// Output vector dimension (e.g. 384).
    fn dimension(&self) -> usize;

    /// Generates an embedding vector for screenshot text (document passage).
    fn embed_passage(&self, text: &str) -> Result<Vec<f32>, AppError>;

    /// Generates an embedding vector for a user search query.
    fn embed_query(&self, query: &str) -> Result<Vec<f32>, AppError>;
}

/// Production embedding engine using FastEmbed with ONNX Runtime.
pub struct FastembedModelEngine {
    model_id: String,
    model_version: String,
    dimension: usize,
    inner: Mutex<TextEmbedding>,
}

impl FastembedModelEngine {
    /// Initializes the fastembed model with the specified cache directory.
    pub fn new(cache_dir: std::path::PathBuf) -> Result<Self, AppError> {
        let options = InitOptions::new(EmbeddingModel::MultilingualE5Small)
            .with_cache_dir(cache_dir)
            .with_show_download_progress(false);

        let inner = TextEmbedding::try_new(options)
            .map_err(|e| AppError::unknown(format!("Failed to initialize FastEmbed model: {e}")))?;

        Ok(Self {
            model_id: DEFAULT_MODEL_ID.to_string(),
            model_version: DEFAULT_MODEL_VERSION.to_string(),
            dimension: DEFAULT_EMBEDDING_DIM,
            inner: Mutex::new(inner),
        })
    }
}

impl TextEmbeddingEngine for FastembedModelEngine {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn model_version(&self) -> &str {
        &self.model_version
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed_passage(&self, text: &str) -> Result<Vec<f32>, AppError> {
        // E5 models achieve best accuracy with 'passage: ' prefix on documents
        let formatted = format!("passage: {}", text.trim());
        let mut model = self.inner.lock().map_err(|e| {
            AppError::unknown(format!("Failed to acquire fastembed model lock: {e}"))
        })?;

        let embeddings = model
            .embed(vec![formatted.as_str()], None)
            .map_err(|e| AppError::unknown(format!("Fastembed inference failed: {e}")))?;

        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| AppError::unknown("Model returned empty embedding vector"))
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, AppError> {
        // E5 models achieve best accuracy with 'query: ' prefix on search queries
        let formatted = format!("query: {}", query.trim());
        let mut model = self.inner.lock().map_err(|e| {
            AppError::unknown(format!("Failed to acquire fastembed model lock: {e}"))
        })?;

        let embeddings = model
            .embed(vec![formatted.as_str()], None)
            .map_err(|e| AppError::unknown(format!("Fastembed query inference failed: {e}")))?;

        embeddings
            .into_iter()
            .next()
            .ok_or_else(|| AppError::unknown("Model returned empty query embedding vector"))
    }
}

/// Deterministic mock embedding engine for testing without downloading model weights.
#[derive(Debug, Clone)]
pub struct MockEmbeddingEngine {
    model_id: String,
    model_version: String,
    dimension: usize,
}

impl MockEmbeddingEngine {
    pub fn new() -> Self {
        Self {
            model_id: DEFAULT_MODEL_ID.to_string(),
            model_version: DEFAULT_MODEL_VERSION.to_string(),
            dimension: DEFAULT_EMBEDDING_DIM,
        }
    }

    /// Computes a pseudo-semantic deterministic unit vector with concept clusters.
    fn compute_mock_vector(&self, text: &str) -> Vec<f32> {
        let mut vec = vec![0.0f32; self.dimension];
        let lower = text.to_lowercase();
        let tokens: Vec<&str> = lower.split_whitespace().collect();

        if tokens.is_empty() {
            vec[0] = 1.0;
            return vec;
        }

        // Semantic concept clusters mapping multilingual terms to common subspace
        let db_cluster = [
            "database",
            "db",
            "sql",
            "prisma",
            "prismaclient",
            "transaction",
            "p2028",
            "postgres",
            "mysql",
            "sqlite",
            "cơ",
            "sở",
            "dữ",
            "liệu",
        ];
        let err_cluster = [
            "error",
            "failure",
            "failed",
            "timeout",
            "closed",
            "crash",
            "bug",
            "lỗi",
            "hỏng",
            "requesterror",
        ];
        let pay_cluster = [
            "payment", "pay", "checkout", "card", "invoice", "thanh", "toán", "tiền", "funds",
        ];
        let term_cluster = [
            "terminal",
            "bash",
            "zsh",
            "npm",
            "cmd",
            "powershell",
            "console",
        ];

        for token in &tokens {
            let clean = token.trim_matches(|c: char| !c.is_alphanumeric());
            if clean.is_empty() {
                continue;
            }

            // Word hash
            let mut h: u64 = 5381;
            for b in clean.bytes() {
                h = ((h << 5).wrapping_add(h)).wrapping_add(b as u64);
            }
            let idx = (h as usize) % self.dimension;
            vec[idx] += 0.5;

            // Semantic subspace projections
            for &k in &db_cluster {
                if clean.contains(k) {
                    for d in 10..25 {
                        vec[d] += 2.0;
                    }
                    break;
                }
            }
            for &k in &err_cluster {
                if clean.contains(k) {
                    for d in 25..40 {
                        vec[d] += 2.0;
                    }
                    break;
                }
            }
            for &k in &pay_cluster {
                if clean.contains(k) {
                    for d in 40..55 {
                        vec[d] += 2.0;
                    }
                    break;
                }
            }
            for &k in &term_cluster {
                if clean.contains(k) {
                    for d in 55..70 {
                        vec[d] += 2.0;
                    }
                    break;
                }
            }
        }

        // L2 normalize
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 1e-6 {
            for v in vec.iter_mut() {
                *v /= norm;
            }
        } else {
            vec[0] = 1.0;
        }

        vec
    }
}

impl Default for MockEmbeddingEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl TextEmbeddingEngine for MockEmbeddingEngine {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn model_version(&self) -> &str {
        &self.model_version
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed_passage(&self, text: &str) -> Result<Vec<f32>, AppError> {
        Ok(self.compute_mock_vector(text))
    }

    fn embed_query(&self, query: &str) -> Result<Vec<f32>, AppError> {
        Ok(self.compute_mock_vector(query))
    }
}
