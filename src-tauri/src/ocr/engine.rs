use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::errors::AppError;

/// User-configurable OCR engine selection mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OcrEngineMode {
    /// Automatically selects native Windows OCR when vi-VN is supported, or Multilingual OCR fallback.
    #[serde(rename = "auto")]
    Auto,
    /// Forces native Windows Media OCR.
    #[serde(rename = "windows_native")]
    Windows,
    /// Forces local multilingual ONNX OCR fallback.
    #[serde(rename = "hybrid_vietnamese")]
    Multilingual,
}

impl Default for OcrEngineMode {
    fn default() -> Self {
        Self::Auto
    }
}

#[cfg(test)]
mod tests {
    use super::OcrEngineMode;

    #[test]
    fn ocr_engine_mode_wire_contract_round_trips() {
        let cases = [
            ("auto", OcrEngineMode::Auto),
            ("windows_native", OcrEngineMode::Windows),
            ("hybrid_vietnamese", OcrEngineMode::Multilingual),
        ];

        for (wire_value, mode) in cases {
            let json = serde_json::to_string(&mode).expect("serialize OCR mode");
            assert_eq!(json, format!("\"{wire_value}\""));

            let decoded: OcrEngineMode = serde_json::from_str(&json).expect("deserialize OCR mode");
            assert_eq!(decoded, mode);
        }
    }

    #[test]
    fn ocr_engine_mode_wire_contract_rejects_invalid_values() {
        let error = serde_json::from_str::<OcrEngineMode>(r#""hybrid""#)
            .expect_err("invalid OCR mode must be rejected");
        assert!(error.to_string().contains("unknown variant"));
    }

    #[test]
    fn frontend_ocr_mode_mapping_uses_the_same_wire_values() {
        let frontend_types = include_str!("../../../src/types/index.ts");
        for wire_value in ["auto", "windows_native", "hybrid_vietnamese"] {
            assert!(
                frontend_types.contains(&format!("\"{wire_value}\"")),
                "frontend mapping is missing {wire_value}"
            );
        }
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
