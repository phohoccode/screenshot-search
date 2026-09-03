use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use super::engine::{OcrEngine, OcrResult};
use super::normalize::normalize_ocr_text;
use crate::errors::AppError;

/// A mock OCR engine used for deterministic unit and integration tests.
pub struct MockOcrEngine {
    default_text: Mutex<String>,
    fail_paths: Mutex<HashSet<String>>,
    empty_paths: Mutex<HashSet<String>>,
}

impl MockOcrEngine {
    pub fn new(default_text: impl Into<String>) -> Self {
        Self {
            default_text: Mutex::new(default_text.into()),
            fail_paths: Mutex::new(HashSet::new()),
            empty_paths: Mutex::new(HashSet::new()),
        }
    }

    /// Configures specific image paths to fail simulated OCR recognition.
    pub fn add_failing_path(&self, path: impl Into<String>) {
        let mut set = self.fail_paths.lock().unwrap();
        set.insert(path.into());
    }

    /// Configures specific image paths to return empty OCR text (e.g. scenic wallpaper).
    pub fn add_empty_path(&self, path: impl Into<String>) {
        let mut set = self.empty_paths.lock().unwrap();
        set.insert(path.into());
    }

    /// Sets the default extracted text returned for normal images.
    pub fn set_default_text(&self, text: impl Into<String>) {
        let mut def = self.default_text.lock().unwrap();
        *def = text.into();
    }
}

impl OcrEngine for MockOcrEngine {
    fn recognize(&self, image_path: &Path) -> Result<OcrResult, AppError> {
        let path_str = image_path.to_string_lossy();

        if self.fail_paths.lock().unwrap().contains(path_str.as_ref()) {
            return Err(AppError::ocr("Simulated OCR engine failure on target file"));
        }

        if self.empty_paths.lock().unwrap().contains(path_str.as_ref()) {
            return Ok(OcrResult {
                text: String::new(),
                engine: self.name().to_string(),
                engine_version: self.version().to_string(),
                confidence: Some(1.0),
            });
        }

        let raw = self.default_text.lock().unwrap().clone();
        let normalized = normalize_ocr_text(&raw);

        Ok(OcrResult {
            text: normalized,
            engine: self.name().to_string(),
            engine_version: self.version().to_string(),
            confidence: Some(0.95),
        })
    }

    fn name(&self) -> &str {
        "mock_ocr"
    }

    fn version(&self) -> &str {
        "1.0.0"
    }
}
