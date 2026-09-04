pub mod classifier;
pub mod detector;
pub mod engine;
pub mod hybrid;
pub mod manager;
pub mod mock;
pub mod multilingual;
pub mod normalize;
pub mod orchestrator;
pub mod router;
pub mod vietocr;
pub mod windows;

#[cfg(test)]
mod ocr_tests;

pub use classifier::{LineContentClassifier, LineContentType};
pub use detector::{DetectedTextLine, TextLineDetector};
pub use engine::{OcrEngine, OcrEngineInfo, OcrEngineMode, OcrResult};
pub use hybrid::HybridOcrEngine;
pub use manager::{MultilingualOcrModelInfo, MultilingualOcrModelManager, MultilingualOcrStatus};
pub use mock::MockOcrEngine;
pub use multilingual::MultilingualOcrEngine;
pub use normalize::normalize_ocr_text;
pub use orchestrator::{run_ocr_batch, OcrBatchSummary, OcrManager};
pub use router::{
    OcrEngineDiagnostics, OcrEngineRouter, HYBRID_OCR_QUALITY_APPROVED,
    MULTILINGUAL_QUALITY_APPROVED,
};
pub use vietocr::VietOcrOnnxRecognizer;
pub use windows::WindowsMediaOcrEngine;
