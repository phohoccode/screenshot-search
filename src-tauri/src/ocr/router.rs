use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, RwLock};

use crate::errors::AppError;
use crate::ocr::engine::{OcrEngine, OcrEngineInfo, OcrEngineMode, OcrResult};
use crate::ocr::manager::{MultilingualOcrModelInfo, MultilingualOcrModelManager};

/// Combined diagnostic information about OCR engines, host language packs, and model status.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrEngineDiagnostics {
    pub mode: OcrEngineMode,
    pub active_engine_name: String,
    pub windows_info: OcrEngineInfo,
    pub multilingual_info: MultilingualOcrModelInfo,
    pub windows_supports_vietnamese: bool,
    pub is_multilingual_ready: bool,
}

/// Intelligent OCR Engine Router that chooses between native Windows Media OCR
/// and the high-accuracy local Multilingual fallback engine.
pub struct OcrEngineRouter {
    mode: Arc<RwLock<OcrEngineMode>>,
    windows_engine: Arc<dyn OcrEngine>,
    model_manager: Arc<MultilingualOcrModelManager>,
}

impl OcrEngineRouter {
    pub fn new(
        windows_engine: Arc<dyn OcrEngine>,
        model_manager: Arc<MultilingualOcrModelManager>,
    ) -> Arc<Self> {
        Arc::new(Self {
            mode: Arc::new(RwLock::new(OcrEngineMode::Auto)),
            windows_engine,
            model_manager,
        })
    }

    pub fn set_mode(&self, mode: OcrEngineMode) {
        *self.mode.write().unwrap() = mode;
        log::info!("OCR Engine Router mode set to {:?}", mode);
    }

    pub fn get_mode(&self) -> OcrEngineMode {
        *self.mode.read().unwrap()
    }

    pub fn get_model_manager(&self) -> Arc<MultilingualOcrModelManager> {
        self.model_manager.clone()
    }

    pub fn get_windows_engine(&self) -> Arc<dyn OcrEngine> {
        self.windows_engine.clone()
    }

    pub fn get_diagnostics(&self) -> OcrEngineDiagnostics {
        let mode = self.get_mode();
        let windows_info = self.windows_engine.get_info();
        let multilingual_info = self.model_manager.get_model_info();
        let is_multilingual_ready = multilingual_info.is_available;
        let windows_supports_vietnamese = windows_info.supports_vietnamese;

        let active_engine_name = match mode {
            OcrEngineMode::Windows => "windows_media_ocr".to_string(),
            OcrEngineMode::Multilingual => "multilingual_ocr".to_string(),
            OcrEngineMode::Auto => {
                if is_multilingual_ready {
                    "multilingual_ocr".to_string()
                } else {
                    "windows_media_ocr".to_string()
                }
            }
        };

        OcrEngineDiagnostics {
            mode,
            active_engine_name,
            windows_info,
            multilingual_info,
            windows_supports_vietnamese,
            is_multilingual_ready,
        }
    }
}

impl OcrEngine for OcrEngineRouter {
    fn recognize(&self, image_path: &Path) -> Result<OcrResult, AppError> {
        let mode = self.get_mode();

        match mode {
            OcrEngineMode::Windows => {
                log::debug!("Routing OCR to Windows Media OCR (Forced)");
                self.windows_engine.recognize(image_path)
            }
            OcrEngineMode::Multilingual => {
                log::debug!("Routing OCR to Multilingual Fallback OCR (Forced)");
                if let Some(engine) = self.model_manager.get_engine() {
                    engine.recognize(image_path)
                } else {
                    Err(AppError::ocr_unavailable(
                        "Multilingual OCR model is not installed. Please download it from Settings/Indexing.",
                    ))
                }
            }
            OcrEngineMode::Auto => {
                // Auto mode logic:
                // If multilingual model is installed and ready, use it for optimal Vietnamese & mixed quality.
                // If not installed, transparently use native Windows OCR.
                // If multilingual OCR fails on an image, safely fallback to Windows OCR.
                if let Some(engine) = self.model_manager.get_engine() {
                    log::debug!("Auto mode: attempting Multilingual OCR");
                    match engine.recognize(image_path) {
                        Ok(res) => Ok(res),
                        Err(e) => {
                            log::warn!(
                                "Multilingual OCR failed on {}: {e}. Falling back to Windows Media OCR",
                                image_path.display()
                            );
                            self.windows_engine.recognize(image_path)
                        }
                    }
                } else {
                    log::debug!(
                        "Auto mode: Multilingual OCR not available, using Windows Media OCR"
                    );
                    self.windows_engine.recognize(image_path)
                }
            }
        }
    }

    fn get_info(&self) -> OcrEngineInfo {
        let mode = self.get_mode();
        match mode {
            OcrEngineMode::Windows => self.windows_engine.get_info(),
            OcrEngineMode::Multilingual => {
                if let Some(engine) = self.model_manager.get_engine() {
                    engine.get_info()
                } else {
                    self.windows_engine.get_info()
                }
            }
            OcrEngineMode::Auto => {
                if let Some(engine) = self.model_manager.get_engine() {
                    engine.get_info()
                } else {
                    self.windows_engine.get_info()
                }
            }
        }
    }

    fn name(&self) -> &str {
        "ocr_engine_router"
    }

    fn version(&self) -> &str {
        "v1"
    }
}
