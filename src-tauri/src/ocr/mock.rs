use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use super::engine::{OcrEngine, OcrEngineInfo, OcrResult};
use super::normalize::normalize_ocr_text;
use crate::errors::AppError;

/// A mock OCR engine used for deterministic unit and integration tests.
pub struct MockOcrEngine {
    default_text: Mutex<String>,
    fail_paths: Mutex<HashSet<String>>,
    empty_paths: Mutex<HashSet<String>>,
    supports_vietnamese: bool,
    engine_name: String,
    engine_version: String,
    /// Override the full OcrEngineInfo when non-None (used by router tests).
    custom_info: Option<OcrEngineInfo>,
    /// When true, every call to `recognize` returns an Err regardless of path.
    always_fail: bool,
    always_fail_msg: String,
}

impl MockOcrEngine {
    pub fn new(default_text: impl Into<String>) -> Self {
        Self {
            default_text: Mutex::new(default_text.into()),
            fail_paths: Mutex::new(HashSet::new()),
            empty_paths: Mutex::new(HashSet::new()),
            supports_vietnamese: false,
            engine_name: "mock_ocr".to_string(),
            engine_version: "1.0.0".to_string(),
            custom_info: None,
            always_fail: false,
            always_fail_msg: String::new(),
        }
    }

    /// Creates a mock engine that always returns an error on any `recognize` call.
    /// Used to simulate ONNX inference failure in router regression tests.
    pub fn new_failing(error_msg: impl Into<String>) -> Self {
        Self {
            default_text: Mutex::new(String::new()),
            fail_paths: Mutex::new(HashSet::new()),
            empty_paths: Mutex::new(HashSet::new()),
            supports_vietnamese: false,
            engine_name: "mock_ocr_failing".to_string(),
            engine_version: "1.0.0".to_string(),
            custom_info: None,
            always_fail: true,
            always_fail_msg: error_msg.into(),
        }
    }

    /// Creates a mock engine with a fully customized OcrEngineInfo.
    /// Used in router regression tests to control `supports_vietnamese` and language fields precisely.
    pub fn new_with_info(default_text: impl Into<String>, info: OcrEngineInfo) -> Self {
        let name = info.engine_name.clone();
        let version = info.engine_version.clone();
        let vi = info.supports_vietnamese;
        Self {
            default_text: Mutex::new(default_text.into()),
            fail_paths: Mutex::new(HashSet::new()),
            empty_paths: Mutex::new(HashSet::new()),
            supports_vietnamese: vi,
            engine_name: name,
            engine_version: version,
            custom_info: Some(info),
            always_fail: false,
            always_fail_msg: String::new(),
        }
    }

    pub fn new_with_vietnamese(default_text: impl Into<String>, supports_vietnamese: bool) -> Self {
        Self {
            default_text: Mutex::new(default_text.into()),
            fail_paths: Mutex::new(HashSet::new()),
            empty_paths: Mutex::new(HashSet::new()),
            supports_vietnamese,
            engine_name: "mock_ocr".to_string(),
            engine_version: "1.0.0".to_string(),
            custom_info: None,
            always_fail: false,
            always_fail_msg: String::new(),
        }
    }

    pub fn new_custom(
        default_text: impl Into<String>,
        engine_name: impl Into<String>,
        engine_version: impl Into<String>,
        supports_vietnamese: bool,
    ) -> Self {
        Self {
            default_text: Mutex::new(default_text.into()),
            fail_paths: Mutex::new(HashSet::new()),
            empty_paths: Mutex::new(HashSet::new()),
            supports_vietnamese,
            engine_name: engine_name.into(),
            engine_version: engine_version.into(),
            custom_info: None,
            always_fail: false,
            always_fail_msg: String::new(),
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
        // Always-fail mode (for router inference-failure tests)
        if self.always_fail {
            return Err(AppError::ocr(self.always_fail_msg.clone()));
        }

        let path_str = image_path.to_string_lossy();

        if self.fail_paths.lock().unwrap().contains(path_str.as_ref()) {
            return Err(AppError::ocr("Simulated OCR engine failure on target file"));
        }

        if self.empty_paths.lock().unwrap().contains(path_str.as_ref()) {
            return Ok(OcrResult {
                text: String::new(),
                engine: self.name().to_string(),
                engine_version: self.version().to_string(),
                language: Some("mock".to_string()),
                confidence: Some(1.0),
            });
        }

        let raw = self.default_text.lock().unwrap().clone();
        let normalized = normalize_ocr_text(&raw);

        let lang = if self.supports_vietnamese {
            "vi-VN".to_string()
        } else {
            "mock".to_string()
        };

        Ok(OcrResult {
            text: normalized,
            engine: self.name().to_string(),
            engine_version: self.version().to_string(),
            language: Some(lang),
            confidence: Some(0.95),
        })
    }

    fn get_info(&self) -> OcrEngineInfo {
        if let Some(ref info) = self.custom_info {
            return info.clone();
        }
        OcrEngineInfo {
            engine_name: self.name().to_string(),
            engine_version: self.version().to_string(),
            active_language: if self.supports_vietnamese {
                "vi-VN".to_string()
            } else {
                "en-US".to_string()
            },
            available_languages: if self.supports_vietnamese {
                vec!["en-US".to_string(), "vi-VN".to_string()]
            } else {
                vec!["en-US".to_string()]
            },
            supports_vietnamese: self.supports_vietnamese,
            max_image_dimension: 2600,
        }
    }

    fn name(&self) -> &str {
        &self.engine_name
    }

    fn version(&self) -> &str {
        &self.engine_version
    }
}
