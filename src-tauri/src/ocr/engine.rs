use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::errors::AppError;

/// User-configurable OCR engine selection mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OcrEngineMode {
    /// Automatically selects native Windows OCR when vi-VN is supported, or Multilingual OCR fallback.
    Auto,
    /// Forces native Windows Media OCR.
    Windows,
    /// Forces local multilingual ONNX OCR fallback.
    Multilingual,
}

impl Default for OcrEngineMode {
    fn default() -> Self {
        Self::Auto
    }
}

/// Structured result of an OCR extraction.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrResult {
    /// Full normalized text extracted from the screenshot.
    pub text: String,
    /// Identifier of the OCR engine used (e.g. "windows_media_ocr", "multilingual_ocr", "mock_ocr").
    pub engine: String,
    /// Version of the OCR engine or model.
    pub engine_version: String,
    /// Language recognizer used (e.g. "vi-VN", "en-US", "multilingual").
    pub language: Option<String>,
    /// Optional overall recognition confidence (0.0 to 1.0).
    pub confidence: Option<f32>,
}

/// Metadata and language diagnostics for the active local OCR engine.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct OcrEngineInfo {
    pub engine_name: String,
    pub engine_version: String,
    pub active_language: String,
    pub available_languages: Vec<String>,
    pub supports_vietnamese: bool,
    pub max_image_dimension: u32,
}

/// Abstract interface for local OCR engines.
/// Decouples indexing orchestration and database persistence from specific OCR providers.
pub trait OcrEngine: Send + Sync {
    /// Performs text recognition on an image at the given filesystem path.
    fn recognize(&self, image_path: &Path) -> Result<OcrResult, AppError>;

    /// Returns diagnostic information about the engine, active language, and limits.
    fn get_info(&self) -> OcrEngineInfo;

    /// Human-readable identifier of this engine.
    fn name(&self) -> &str;

    /// Version string for diagnostics and schema auditing.
    fn version(&self) -> &str;
}
