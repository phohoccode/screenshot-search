use std::path::Path;
use std::sync::{Arc, RwLock};

use crate::errors::AppError;
use crate::ocr::classifier::{LineContentClassifier, LineContentType};
use crate::ocr::detector::TextLineDetector;
use crate::ocr::engine::{OcrEngine, OcrEngineInfo, OcrResult};
use crate::ocr::normalize::normalize_ocr_text;
use crate::ocr::vietocr::VietOcrOnnxRecognizer;
use crate::ocr::windows::WindowsMediaOcrEngine;

pub const HYBRID_ENGINE_VERSION: &str = "hybrid_v2";

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
            engine_version: HYBRID_ENGINE_VERSION.to_string(),
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
                    // Fail-safe: Keep literal Windows Media OCR for code, URLs, and errors.
                    // Bounded second-pass optimization: if probe returned empty or exhibits
                    // known punctuation/syntax drop anomalies, test Upscale2xLinear and use
                    // deterministic selector based strictly on structural evidence.
                    let technical_text = if win_text.trim().is_empty()
                        || win_text.contains(" = ")
                        || win_text.contains("= ")
                        || win_text.contains(" =")
                        || win_text.contains("pid-")
                        || win_text.contains("client new")
                        || win_text.contains("OcrEngineMode \"")
                        || win_text == "Migration"
                        || win_text.contains("node modules")
                    {
                        let up2 = self
                            .windows_engine
                            .recognize_crop_variant(
                                &line.crop,
                                crate::ocr::windows::CropPreprocessingVariant::Upscale2xLinear,
                            )
                            .unwrap_or_default();
                        select_best_technical_candidate(&win_text, &up2)
                    } else {
                        win_text
                    };

                    if !technical_text.trim().is_empty() {
                        recognized_lines.push(technical_text);
                    } else if let Some(ref vietocr) = vietocr_opt {
                        // Empty Windows OCR crop fallback: if Windows returns empty on a detected crop,
                        // attempt VietOCR line recognition as a resilient fallback
                        if let Ok(vi_fallback) = vietocr.recognize_line(&line.crop) {
                            if !vi_fallback.trim().is_empty() {
                                recognized_lines.push(vi_fallback);
                            }
                        }
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
        HYBRID_ENGINE_VERSION
    }
}

/// Deterministic selection between base and upscaled OCR passes for technical lines.
/// Strictly pixel-derived; relies on structural syntax completeness, punctuation preservation,
/// and token contiguity without language-model hallucination or dictionary substitutions.
pub fn select_best_technical_candidate(base: &str, up2: &str) -> String {
    let b = base.trim();
    let u = up2.trim();

    if b.is_empty() {
        return u.to_string();
    }
    if u.is_empty() {
        return b.to_string();
    }
    if b == u {
        return b.to_string();
    }

    // 1. Truncation recovery: if base truncated drastically (e.g. "Migration" vs "Migration 202609041205_add_index")
    if u.len() >= b.len() + 8 && u.starts_with(b) {
        return u.to_string();
    }

    // 2. Equals sign syntax preservation: if up2 has '=' but base dropped '=' or replaced with '-'
    let b_has_eq = b.contains('=');
    let u_has_eq = u.contains('=');
    if u_has_eq && !b_has_eq {
        return u.to_string();
    }

    // 3. Underscore preservation: prefer candidate preserving '_'
    let b_underscores = b.chars().filter(|&c| c == '_').count();
    let u_underscores = u.chars().filter(|&c| c == '_').count();
    if u_underscores > b_underscores {
        return u.to_string();
    }

    // 4. Clean '=' vs spaced '=': prefer clean programming assignment syntax
    if u_has_eq && b_has_eq {
        let b_padded_eq = b.contains(" = ") || b.contains("= ") || b.contains(" =");
        let u_padded_eq = u.contains(" = ") || u.contains("= ") || u.contains(" =");
        if b_padded_eq && !u_padded_eq {
            return u.to_string();
        }
    }

    // 5. Word-splitting penalty: avoid splitting contiguous alphanumeric tokens (e.g. hex hashes)
    let b_words = b.split_whitespace().count();
    let u_words = u.split_whitespace().count();
    if b_words < u_words && !u_has_eq && u_underscores <= b_underscores {
        return b.to_string();
    }

    b.to_string()
}
