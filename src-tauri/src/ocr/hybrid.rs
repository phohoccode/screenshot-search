use std::path::Path;
use std::sync::{Arc, RwLock};

use crate::errors::AppError;
use crate::ocr::classifier::{LineContentClassifier, LineContentType};
use crate::ocr::detector::TextLineDetector;
use crate::ocr::engine::{OcrEngine, OcrEngineInfo, OcrResult};
use crate::ocr::normalize::normalize_ocr_text;
use crate::ocr::vietocr::VietOcrOnnxRecognizer;
use crate::ocr::windows::WindowsMediaOcrEngine;

/// Hybrid Per-Line OCR Engine combining:
/// - DBNet Text Line Detector (PP-OCRv4)
/// - Windows Media OCR (literal probe & technical recognition)
/// - VietOCR VGG-Transformer ONNX (natural language recognition)
pub struct HybridOcrEngine {
    detector: Arc<TextLineDetector>,
    windows_engine: Arc<WindowsMediaOcrEngine>,
    vietocr_recognizer: Arc<RwLock<Option<Arc<VietOcrOnnxRecognizer>>>>,
    info: OcrEngineInfo,
}

impl HybridOcrEngine {
    pub fn new(
        detector: Arc<TextLineDetector>,
        windows_engine: Arc<WindowsMediaOcrEngine>,
        vietocr_recognizer: Option<Arc<VietOcrOnnxRecognizer>>,
    ) -> Self {
        let info = OcrEngineInfo {
            engine_name: "hybrid_windows_vietocr".to_string(),
            engine_version: "hybrid_v1".to_string(),
            active_language: "mixed/vi-en".to_string(),
            available_languages: vec!["vi-VN".to_string(), "en-US".to_string()],
            supports_vietnamese: true,
            max_image_dimension: 4096,
        };

        Self {
            detector,
            windows_engine,
            vietocr_recognizer: Arc::new(RwLock::new(vietocr_recognizer)),
            info,
        }
    }

    pub fn set_vietocr_recognizer(&self, recognizer: Option<Arc<VietOcrOnnxRecognizer>>) {
        *self.vietocr_recognizer.write().unwrap() = recognizer;
    }

    pub fn is_vietocr_ready(&self) -> bool {
        self.vietocr_recognizer.read().unwrap().is_some()
    }

    pub fn get_vietocr_recognizer(&self) -> Option<Arc<VietOcrOnnxRecognizer>> {
        self.vietocr_recognizer.read().unwrap().clone()
    }
}

impl OcrEngine for HybridOcrEngine {
    fn recognize(&self, image_path: &Path) -> Result<OcrResult, AppError> {
        if !image_path.exists() {
            return Err(AppError::file_not_found(format!(
                "Screenshot image not found at: {}",
                image_path.display()
            )));
        }

        // 1. Decode original screenshot in memory
        let img = image::open(image_path)
            .map_err(|e| {
                AppError::ocr_decode(format!(
                    "Failed to open image for hybrid OCR {}: {e}",
                    image_path.display()
                ))
            })?
            .to_rgb8();

        // 2. Detect text line bounding boxes using DBNet
        let lines = match self.detector.detect_lines(&img) {
            Ok(lines) => lines,
            Err(e) => {
                log::warn!(
                    "Text line detection failed on {}: {e}. Falling back to full-screen Windows OCR.",
                    image_path.display()
                );
                return self.windows_engine.recognize(image_path);
            }
        };

        // If no text lines detected, fall back to native full-screenshot Windows OCR
        if lines.is_empty() {
            log::debug!(
                "Hybrid OCR: No lines detected by DBNet on {}. Falling back to full-screen Windows OCR.",
                image_path.display()
            );
            return self.windows_engine.recognize(image_path);
        }

        // 3. Process each line with Windows OCR probe and classifier routing
        let mut recognized_lines: Vec<String> = Vec::with_capacity(lines.len());
        let vietocr_opt = self.get_vietocr_recognizer();

        for line in &lines {
            // A. Windows OCR Probe
            let win_text = match self.windows_engine.recognize_crop(&line.crop) {
                Ok(t) => t,
                Err(e) => {
                    log::debug!(
                        "Windows line crop OCR failed on line {}: {e}",
                        line.line_index
                    );
                    String::new()
                }
            };

            // B. Deterministic Line Classification
            let content_type = LineContentClassifier::classify(&win_text);

            // C. Routing & Recognition
            match content_type {
                LineContentType::Natural => {
                    if let Some(ref vietocr) = vietocr_opt {
                        // Attempt VietOCR ONNX line recognition
                        match vietocr.recognize_line(&line.crop) {
                            Ok(vi_text) if !vi_text.trim().is_empty() => {
                                recognized_lines.push(vi_text);
                            }
                            Ok(_) => {
                                // Empty VietOCR output -> fallback to Windows probe
                                if !win_text.trim().is_empty() {
                                    recognized_lines.push(win_text);
                                }
                            }
                            Err(e) => {
                                log::warn!(
                                    "VietOCR recognition failed on line {}: {e}. Retaining Windows OCR probe.",
                                    line.line_index
                                );
                                if !win_text.trim().is_empty() {
                                    recognized_lines.push(win_text);
                                }
                            }
                        }
                    } else {
                        // VietOCR not ready -> fallback to Windows probe
                        if !win_text.trim().is_empty() {
                            recognized_lines.push(win_text);
                        }
                    }
                }
                LineContentType::Technical | LineContentType::Uncertain => {
                    // Fail-safe: Always keep literal Windows Media OCR for code, URLs, and errors
                    if !win_text.trim().is_empty() {
                        recognized_lines.push(win_text);
                    }
                }
            }
        }

        // 4. If all lines returned empty, fallback to full-screen Windows OCR
        if recognized_lines.is_empty() {
            return self.windows_engine.recognize(image_path);
        }

        // 5. Merge lines in spatial reading order and normalize Unicode NFC
        let merged_text = recognized_lines.join("\n");
        let normalized = normalize_ocr_text(&merged_text);

        Ok(OcrResult {
            text: normalized,
            engine: self.name().to_string(),
            engine_version: self.version().to_string(),
            language: Some("mixed".to_string()),
            confidence: Some(0.95),
        })
    }

    fn get_info(&self) -> OcrEngineInfo {
        self.info.clone()
    }

    fn name(&self) -> &str {
        "hybrid_windows_vietocr"
    }

    fn version(&self) -> &str {
        "hybrid_v1"
    }
}
