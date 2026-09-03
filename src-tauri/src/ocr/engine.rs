use std::path::Path;

use crate::errors::AppError;

/// Structured result of an OCR extraction.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrResult {
    /// Full normalized text extracted from the screenshot.
    pub text: String,
    /// Identifier of the OCR engine used (e.g. "windows_media_ocr", "mock_ocr").
    pub engine: String,
    /// Version of the OCR engine or model.
    pub engine_version: String,
    /// Optional overall recognition confidence (0.0 to 1.0).
    pub confidence: Option<f32>,
}

/// Abstract interface for local OCR engines.
/// Decouples indexing orchestration and database persistence from specific OCR providers.
pub trait OcrEngine: Send + Sync {
    /// Performs text recognition on an image at the given filesystem path.
    fn recognize(&self, image_path: &Path) -> Result<OcrResult, AppError>;

    /// Human-readable identifier of this engine.
    fn name(&self) -> &str;

    /// Version string for diagnostics and schema auditing.
    fn version(&self) -> &str;
}
