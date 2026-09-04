pub mod engine;
pub mod manager;
pub mod mock;
pub mod multilingual;
pub mod normalize;
pub mod orchestrator;
pub mod router;
pub mod windows;

#[cfg(test)]
mod ocr_tests;

pub use engine::{OcrEngine, OcrEngineInfo, OcrEngineMode, OcrResult};
pub use manager::{MultilingualOcrModelInfo, MultilingualOcrModelManager, MultilingualOcrStatus};
pub use mock::MockOcrEngine;
pub use multilingual::MultilingualOcrEngine;
pub use normalize::normalize_ocr_text;
pub use orchestrator::{run_ocr_batch, OcrBatchSummary, OcrManager};
pub use router::{OcrEngineDiagnostics, OcrEngineRouter};
pub use windows::WindowsMediaOcrEngine;
