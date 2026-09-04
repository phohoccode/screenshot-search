use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, RwLock};

use crate::errors::AppError;
use crate::ocr::engine::{OcrEngine, OcrEngineInfo, OcrEngineMode, OcrResult};
use crate::ocr::manager::{MultilingualOcrModelInfo, MultilingualOcrModelManager};

/// Phase 3.5B quality gate.
/// Set to `true` ONLY after a replacement recognizer has been benchmarked and confirmed to meet:
///   - Aggregate CER < 15% on the 30-fixture Vietnamese benchmark corpus
///   - WER materially below Windows en-US baseline (25.91% CER / 84.75% WER)
///   - Technical exact-token accuracy ≥ 95%
///
/// Current status (2026-09-04):
///   multilingual_PP-OCRv4_rec_infer.onnx: CER=105.48%, WER=113.20%, Tech=5.0% → REJECTED
///   No quality-approved Vietnamese OCR fallback exists yet.
///   Auto mode falls back to Windows Media OCR until this gate is enabled.
pub const MULTILINGUAL_QUALITY_APPROVED: bool = false;

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
                // Deterministic Precedence (mirrors recognize() logic above):
                // 1. Windows vi-VN available → windows_media_ocr
                // 2. Windows lacks vi-VN + MULTILINGUAL_QUALITY_APPROVED + model ready → multilingual_ocr
                // 3. Otherwise → windows_media_ocr (quality gate blocked or model not installed)
                if windows_supports_vietnamese {
                    "windows_media_ocr".to_string()
                } else if MULTILINGUAL_QUALITY_APPROVED && is_multilingual_ready {
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
                // Deterministic Precedence:
                // 1. If Windows Media OCR supports Vietnamese natively → use it (native WinRT, zero RAM overhead).
                // 2. If Windows lacks vi-VN AND the multilingual engine has passed the Phase 3.5B quality gate
                //    (MULTILINGUAL_QUALITY_APPROVED = true) → attempt Multilingual OCR.
                // 3. If Multilingual OCR is missing, not approved, or inference fails → fallback to Windows Media OCR.
                //
                // Phase 3.5B audit (2026-09-04) result:
                //   multilingual_PP-OCRv4_rec_infer.onnx: CER=105.48%, WER=113.20%, Tech=5.0%
                //   → QUALITY GATE BLOCKED. Auto mode will NOT route to this engine.
                //   → Windows Media OCR (en-US, CER=25.91%) is strictly better for Vietnamese on this machine.
                let windows_supports_vi = self.windows_engine.get_info().supports_vietnamese;
                if windows_supports_vi {
                    log::debug!("Auto mode: Windows Media OCR supports Vietnamese natively; prioritizing native engine");
                    self.windows_engine.recognize(image_path)
                } else if MULTILINGUAL_QUALITY_APPROVED {
                    if let Some(engine) = self.model_manager.get_engine() {
                        log::debug!("Auto mode: Windows lacks vi-VN; quality-approved Multilingual OCR selected");
                        match engine.recognize(image_path) {
                            Ok(res) => Ok(res),
                            Err(e) => {
                                log::warn!(
                                    "Multilingual OCR inference failed on {}: {e}. Gracefully falling back to Windows Media OCR",
                                    image_path.display()
                                );
                                self.windows_engine.recognize(image_path)
                            }
                        }
                    } else {
                        log::debug!("Auto mode: Multilingual OCR not available, falling back to Windows Media OCR");
                        self.windows_engine.recognize(image_path)
                    }
                } else {
                    log::warn!(
                        "Auto mode: Multilingual OCR quality gate blocked (MULTILINGUAL_QUALITY_APPROVED=false). \
                        Current model CER=105.48% fails threshold. Using Windows Media OCR instead."
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
                let windows_info = self.windows_engine.get_info();
                if windows_info.supports_vietnamese {
                    windows_info
                } else if let Some(engine) = self.model_manager.get_engine() {
                    engine.get_info()
                } else {
                    windows_info
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
