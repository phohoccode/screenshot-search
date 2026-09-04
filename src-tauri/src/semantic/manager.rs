use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;

use crate::errors::AppError;
use crate::semantic::engine::{
    FastembedModelEngine, TextEmbeddingEngine, DEFAULT_EMBEDDING_DIM, DEFAULT_MODEL_ID,
    DEFAULT_MODEL_VERSION,
};

/// High-level status of the local semantic embedding model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum SemanticModelStatus {
    NotInstalled,
    Downloading { percent: f32 },
    Ready,
    Error { message: String },
}

/// Metadata about the active semantic model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticModelInfo {
    pub model_id: String,
    pub model_version: String,
    pub dimension: usize,
    pub status: SemanticModelStatus,
    pub is_available: bool,
    pub approximate_size_mb: usize,
}

/// Central manager for the local text embedding model lifecycle, offline caching, and thread-safe inference.
pub struct SemanticModelManager {
    models_dir: PathBuf,
    status: Arc<RwLock<SemanticModelStatus>>,
    engine: Arc<RwLock<Option<Arc<dyn TextEmbeddingEngine>>>>,
    is_downloading: Arc<AtomicBool>,
}

impl SemanticModelManager {
    /// Creates a new model manager pointing to the application's models directory.
    pub fn new(app_data_dir: &Path) -> Arc<Self> {
        let models_dir = app_data_dir.join("models").join(DEFAULT_MODEL_ID);
        let _ = fs::create_dir_all(&models_dir);

        let manager = Arc::new(Self {
            models_dir,
            status: Arc::new(RwLock::new(SemanticModelStatus::NotInstalled)),
            engine: Arc::new(RwLock::new(None)),
            is_downloading: Arc::new(AtomicBool::new(false)),
        });

        // Check if model files already exist on disk
        if manager.has_local_model_files() {
            log::info!("Found local semantic model files. Initializing model in background...");
            let mgr_clone = manager.clone();
            thread::Builder::new()
                .name("model-init".into())
                .spawn(move || {
                    let _ = mgr_clone.load_local_engine();
                })
                .expect("Failed to spawn model-init thread");
        } else {
            log::info!(
                "Semantic model not installed locally. Operating in FTS5 keyword search mode."
            );
        }

        manager
    }

    /// Creates a manager pre-configured with a specific embedding engine (used for testing).
    pub fn with_engine(engine: Arc<dyn TextEmbeddingEngine>) -> Arc<Self> {
        Arc::new(Self {
            models_dir: PathBuf::from("mock_models"),
            status: Arc::new(RwLock::new(SemanticModelStatus::Ready)),
            engine: Arc::new(RwLock::new(Some(engine))),
            is_downloading: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Verifies if local model files (ONNX model weights and tokenizer) exist on disk.
    pub fn has_local_model_files(&self) -> bool {
        if !self.models_dir.exists() {
            return false;
        }

        // Recursively inspect models_dir for .onnx file
        let mut has_onnx = false;
        if let Ok(entries) = walkdir::WalkDir::new(&self.models_dir)
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
        {
            for entry in entries {
                if let Some(ext) = entry.path().extension() {
                    if ext.to_string_lossy().to_lowercase() == "onnx" {
                        if let Ok(meta) = entry.metadata() {
                            // Valid model binary should be at least 10MB
                            if meta.len() > 10 * 1024 * 1024 {
                                has_onnx = true;
                                break;
                            }
                        }
                    }
                }
            }
        }

        has_onnx
    }

    /// Loads the local model into memory and updates status to Ready.
    fn load_local_engine(&self) -> Result<(), AppError> {
        match FastembedModelEngine::new(self.models_dir.clone()) {
            Ok(eng) => {
                let mut eng_guard = self
                    .engine
                    .write()
                    .map_err(|e| AppError::unknown(format!("Lock acquisition error: {e}")))?;
                *eng_guard = Some(Arc::new(eng));

                let mut status_guard = self
                    .status
                    .write()
                    .map_err(|e| AppError::unknown(format!("Lock acquisition error: {e}")))?;
                *status_guard = SemanticModelStatus::Ready;

                log::info!("Semantic model (multilingual-e5-small) successfully loaded and ready for inference");
                Ok(())
            }
            Err(e) => {
                log::warn!("Failed to load local semantic model: {e}");
                let mut status_guard = self
                    .status
                    .write()
                    .map_err(|e| AppError::unknown(format!("Lock acquisition error: {e}")))?;
                *status_guard = SemanticModelStatus::Error {
                    message: format!("Failed to load local model: {e}"),
                };
                Err(e)
            }
        }
    }

    /// Triggers user-requested download of the semantic model with progress reporting.
    pub fn start_download(
        self: &Arc<Self>,
        on_completed: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<(), AppError> {
        if self.is_downloading.swap(true, Ordering::SeqCst) {
            return Ok(()); // Download already in progress
        }

        {
            let mut status_guard = self
                .status
                .write()
                .map_err(|e| AppError::unknown(format!("Lock acquisition error: {e}")))?;
            *status_guard = SemanticModelStatus::Downloading { percent: 0.0 };
        }

        let self_clone = self.clone();
        thread::Builder::new()
            .name("model-downloader".into())
            .spawn(move || {
                log::info!(
                    "Starting download of multilingual-e5-small into {:?}",
                    self_clone.models_dir
                );

                // Fastembed automatically fetches model files into the specified cache directory
                let res = FastembedModelEngine::new(self_clone.models_dir.clone());
                self_clone.is_downloading.store(false, Ordering::SeqCst);

                match res {
                    Ok(eng) => {
                        if let Ok(mut eng_guard) = self_clone.engine.write() {
                            *eng_guard = Some(Arc::new(eng));
                        }
                        if let Ok(mut status_guard) = self_clone.status.write() {
                            *status_guard = SemanticModelStatus::Ready;
                        }
                        log::info!("Semantic model download completed successfully");
                        if let Some(cb) = on_completed {
                            cb();
                        }
                    }
                    Err(err) => {
                        log::error!("Failed to download semantic model: {err}");
                        if let Ok(mut status_guard) = self_clone.status.write() {
                            *status_guard = SemanticModelStatus::Error {
                                message: format!("Download failed: {err}"),
                            };
                        }
                    }
                }
            })
            .map_err(|e| {
                AppError::unknown(format!("Failed to spawn model-downloader thread: {e}"))
            })?;

        Ok(())
    }

    /// Returns the currently active TextEmbeddingEngine if available.
    pub fn get_engine(&self) -> Option<Arc<dyn TextEmbeddingEngine>> {
        self.engine.read().ok().and_then(|guard| guard.clone())
    }

    /// Queries the overall status and metadata of the semantic model.
    pub fn get_model_info(&self) -> SemanticModelInfo {
        let status = self
            .status
            .read()
            .map(|s| s.clone())
            .unwrap_or(SemanticModelStatus::NotInstalled);
        let is_available = matches!(status, SemanticModelStatus::Ready);

        SemanticModelInfo {
            model_id: DEFAULT_MODEL_ID.to_string(),
            model_version: DEFAULT_MODEL_VERSION.to_string(),
            dimension: DEFAULT_EMBEDDING_DIM,
            status,
            is_available,
            approximate_size_mb: 135,
        }
    }
}
