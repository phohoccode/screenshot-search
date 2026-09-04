use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;

use crate::errors::AppError;
use crate::ocr::engine::OcrEngine;
use crate::ocr::multilingual::MultilingualOcrEngine;

pub const DEFAULT_OCR_MODEL_ID: &str = "multilingual-ocr";
pub const DEFAULT_OCR_MODEL_VERSION: &str = "ppocr_v4";
pub const APPROXIMATE_OCR_MODEL_SIZE_MB: usize = 16;

/// Remote model asset definitions with sha256 checksums.
pub const DET_MODEL_FILENAME: &str = "ch_PP-OCRv4_det_infer.onnx";
pub const REC_MODEL_FILENAME: &str = "multilingual_PP-OCRv4_rec_infer.onnx";
pub const KEYS_FILENAME: &str = "keys.txt";

pub const DET_MODEL_URL: &str =
    "https://huggingface.co/cycloneboy/ch_PP-OCRv4_det_infer/resolve/main/model.onnx";
pub const REC_MODEL_URL: &str =
    "https://huggingface.co/cycloneboy/ch_PP-OCRv4_rec_infer/resolve/main/model.onnx";

pub const DET_MODEL_SHA256: &str =
    "69ce850fec741a2a4568c7c924bb025c9d4f1129e5f96ab428c799ccc5ef2275";
pub const REC_MODEL_SHA256: &str =
    "ad7dd55f6759fa02333bff6eb179a4f51be5b89cbe6f710249c95f47d0211350";
pub const KEYS_URL: &str =
    "https://huggingface.co/cycloneboy/ch_PP-OCRv4_rec_infer/resolve/main/ch_dict.txt";
pub const KEYS_SHA256: &str = "b22996db93ffedffa90abf62009659af14ae22df06a2da5a1ce0e6fb1117af86";

/// High-level status of the local multilingual OCR model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum MultilingualOcrStatus {
    NotInstalled,
    Downloading { percent: f32 },
    Ready,
    Error { message: String },
}

/// Metadata about the multilingual OCR fallback model.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MultilingualOcrModelInfo {
    pub model_id: String,
    pub model_version: String,
    pub status: MultilingualOcrStatus,
    pub is_available: bool,
    pub approximate_size_mb: usize,
}

/// Manager responsible for downloading, verifying, and initializing the local Multilingual OCR model.
pub struct MultilingualOcrModelManager {
    models_dir: PathBuf,
    status: Arc<RwLock<MultilingualOcrStatus>>,
    engine: Arc<RwLock<Option<Arc<dyn OcrEngine>>>>,
    is_downloading: Arc<AtomicBool>,
}

impl MultilingualOcrModelManager {
    pub fn new(app_data_dir: &Path) -> Arc<Self> {
        let models_dir = app_data_dir.join("models").join(DEFAULT_OCR_MODEL_ID);
        let _ = fs::create_dir_all(&models_dir);

        let manager = Arc::new(Self {
            models_dir,
            status: Arc::new(RwLock::new(MultilingualOcrStatus::NotInstalled)),
            engine: Arc::new(RwLock::new(None)),
            is_downloading: Arc::new(AtomicBool::new(false)),
        });

        if manager.has_local_model_files() {
            log::info!("Found local Multilingual OCR model files. Initializing engine...");
            let mgr_clone = manager.clone();
            thread::Builder::new()
                .name("ocr-model-init".into())
                .spawn(move || {
                    let _ = mgr_clone.load_local_engine();
                })
                .expect("Failed to spawn ocr-model-init thread");
        } else {
            log::info!(
                "Multilingual OCR model not installed. Operating with native Windows Media OCR."
            );
        }

        manager
    }

    /// Creates a manager pre-configured with a specific OCR engine (used for testing).
    pub fn with_engine(engine: Arc<dyn OcrEngine>) -> Arc<Self> {
        Arc::new(Self {
            models_dir: PathBuf::from("mock_ocr_models"),
            status: Arc::new(RwLock::new(MultilingualOcrStatus::Ready)),
            engine: Arc::new(RwLock::new(Some(engine))),
            is_downloading: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Creates a manager in `NotInstalled` state with no engine (used for testing Scenario 3).
    pub fn new_empty_for_test() -> Arc<Self> {
        Arc::new(Self {
            models_dir: PathBuf::from("mock_ocr_models"),
            status: Arc::new(RwLock::new(MultilingualOcrStatus::NotInstalled)),
            engine: Arc::new(RwLock::new(None)),
            is_downloading: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Checks if local model files exist in the models directory.
    pub fn has_local_model_files(&self) -> bool {
        if !self.models_dir.exists() {
            return false;
        }

        let det_path = self.models_dir.join(DET_MODEL_FILENAME);
        let rec_path = self.models_dir.join(REC_MODEL_FILENAME);
        let keys_path = self.models_dir.join(KEYS_FILENAME);

        det_path.exists() && rec_path.exists() && keys_path.exists()
    }

    /// Loads the multilingual OCR engine from the local models directory.
    pub fn load_local_engine(&self) -> Result<(), AppError> {
        if !self.has_local_model_files() {
            *self.status.write().unwrap() = MultilingualOcrStatus::NotInstalled;
            return Err(AppError::ocr_unavailable(
                "Multilingual OCR model files not found on disk",
            ));
        }

        log::info!(
            "Loading Multilingual OCR engine from {}",
            self.models_dir.display()
        );
        match MultilingualOcrEngine::new(&self.models_dir) {
            Ok(engine) => {
                let arc_engine: Arc<dyn OcrEngine> = Arc::new(engine);
                *self.engine.write().unwrap() = Some(arc_engine);
                *self.status.write().unwrap() = MultilingualOcrStatus::Ready;
                log::info!(
                    "Multilingual OCR engine initialized successfully and is ready for inference"
                );
                Ok(())
            }
            Err(e) => {
                log::error!("Failed to initialize Multilingual OCR engine: {e}");
                *self.status.write().unwrap() = MultilingualOcrStatus::Error {
                    message: format!("Failed to initialize Multilingual OCR engine: {e}"),
                };
                Err(e)
            }
        }
    }

    /// Initiates on-demand background download of the model assets.
    pub fn start_download(
        self: &Arc<Self>,
        on_complete: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Result<(), AppError> {
        if self
            .is_downloading
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            log::warn!("Multilingual OCR model download already in progress");
            return Ok(());
        }

        *self.status.write().unwrap() = MultilingualOcrStatus::Downloading { percent: 0.0 };

        let self_clone = self.clone();
        thread::Builder::new()
            .name("ocr-model-download".into())
            .spawn(move || {
                let result = self_clone.perform_download();
                self_clone.is_downloading.store(false, Ordering::SeqCst);

                match result {
                    Ok(_) => {
                        log::info!("Multilingual OCR model download completed successfully. Loading engine...");
                        if let Ok(()) = self_clone.load_local_engine() {
                            if let Some(cb) = on_complete {
                                cb();
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("Multilingual OCR model download failed: {e}");
                        *self_clone.status.write().unwrap() = MultilingualOcrStatus::Error {
                            message: format!("Download failed: {e}"),
                        };
                    }
                }
            })
            .map_err(|e| AppError::unknown(format!("Failed to spawn download thread: {e}")))?;

        Ok(())
    }

    fn perform_download(&self) -> Result<(), AppError> {
        let _ = fs::create_dir_all(&self.models_dir);

        log::info!(
            "Downloading multilingual OCR model files to {}",
            self.models_dir.display()
        );

        // 1. Keys character dictionary for PP-OCRv4 recognition
        *self.status.write().unwrap() = MultilingualOcrStatus::Downloading { percent: 10.0 };
        let keys_target = self.models_dir.join(KEYS_FILENAME);
        let keys_tmp = self.models_dir.join(format!("{KEYS_FILENAME}.tmp"));
        let keys_bytes = crate::ocr::multilingual::DEFAULT_KEYS_DICT.as_bytes();
        verify_and_install_asset(keys_bytes, KEYS_SHA256, &keys_target, &keys_tmp)?;

        // 2. Detection ONNX model
        *self.status.write().unwrap() = MultilingualOcrStatus::Downloading { percent: 45.0 };
        let det_target = self.models_dir.join(DET_MODEL_FILENAME);
        if !det_target.exists() {
            download_and_verify_asset(DET_MODEL_URL, DET_MODEL_SHA256, &det_target)?;
        }

        // 3. Recognition ONNX model
        *self.status.write().unwrap() = MultilingualOcrStatus::Downloading { percent: 85.0 };
        let rec_target = self.models_dir.join(REC_MODEL_FILENAME);
        if !rec_target.exists() {
            download_and_verify_asset(REC_MODEL_URL, REC_MODEL_SHA256, &rec_target)?;
        }

        *self.status.write().unwrap() = MultilingualOcrStatus::Downloading { percent: 100.0 };
        log::info!(
            "Multilingual OCR assets verified and atomically installed at {}",
            self.models_dir.display()
        );

        Ok(())
    }

    /// Access the active Multilingual OCR engine if ready.
    pub fn get_engine(&self) -> Option<Arc<dyn OcrEngine>> {
        self.engine.read().unwrap().clone()
    }

    /// Retrieves diagnostic information for UI and settings.
    pub fn get_model_info(&self) -> MultilingualOcrModelInfo {
        let status = self.status.read().unwrap().clone();
        let is_available = matches!(status, MultilingualOcrStatus::Ready);

        MultilingualOcrModelInfo {
            model_id: DEFAULT_OCR_MODEL_ID.to_string(),
            model_version: DEFAULT_OCR_MODEL_VERSION.to_string(),
            status,
            is_available,
            approximate_size_mb: APPROXIMATE_OCR_MODEL_SIZE_MB,
        }
    }
}

/// Streams bytes from reader into temporary path, calculates SHA-256 in flight,
/// verifies integrity against expected checksum, and atomically swaps to final target path.
/// If verification fails or download is aborted, the temp file is removed and target is untouched.
pub fn verify_and_install_asset<R: std::io::Read>(
    mut reader: R,
    expected_sha256: &str,
    target_path: &Path,
    tmp_path: &Path,
) -> Result<(), AppError> {
    use sha2::{Digest, Sha256};
    use std::io::Write;

    let mut hasher = Sha256::new();
    let mut file = File::create(tmp_path).map_err(|e| {
        AppError::io(format!(
            "Failed to create temporary download file {}: {e}",
            tmp_path.display()
        ))
    })?;

    let mut buf = [0u8; 32768];
    loop {
        let n = reader.read(&mut buf).map_err(|e| {
            let _ = fs::remove_file(tmp_path);
            AppError::io(format!(
                "Error reading stream for {}: {e}",
                target_path.display()
            ))
        })?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        file.write_all(&buf[..n]).map_err(|e| {
            let _ = fs::remove_file(tmp_path);
            AppError::io(format!("Error writing to temporary download file: {e}"))
        })?;
    }

    file.flush().map_err(|e| {
        let _ = fs::remove_file(tmp_path);
        AppError::io(format!("Failed to flush temporary download file: {e}"))
    })?;
    drop(file);

    let computed_hash = format!("{:x}", hasher.finalize());
    if !expected_sha256.is_empty() && !computed_hash.eq_ignore_ascii_case(expected_sha256) {
        let _ = fs::remove_file(tmp_path);
        return Err(AppError::unknown(format!(
            "Checksum mismatch for {}: expected {}, computed {}. Download rejected.",
            target_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy(),
            expected_sha256,
            computed_hash
        )));
    }

    // Atomic installation: rename tmp to target
    fs::rename(tmp_path, target_path).map_err(|e| {
        let _ = fs::remove_file(tmp_path);
        AppError::io(format!(
            "Failed to atomically install asset to {}: {e}",
            target_path.display()
        ))
    })?;

    Ok(())
}

/// Downloads asset from fixed HTTPS URL, verifies SHA-256 integrity, and atomically installs it.
pub fn download_and_verify_asset(
    url: &str,
    expected_sha256: &str,
    target_path: &Path,
) -> Result<(), AppError> {
    let tmp_path = target_path.with_extension("tmp");
    log::info!(
        "Downloading asset from {} to {}",
        url,
        target_path.display()
    );

    let response = ureq::get(url)
        .header("User-Agent", "ScreenshotSearch-ModelDownloader/1.0")
        .call()
        .map_err(|e| AppError::unknown(format!("Failed to download {url}: {e}")))?;

    if !response.status().is_success() {
        return Err(AppError::unknown(format!(
            "Download failed for {url}: HTTP status {}",
            response.status().as_u16()
        )));
    }

    let mut reader = response.into_body().into_reader();
    verify_and_install_asset(&mut reader, expected_sha256, target_path, &tmp_path)
}
