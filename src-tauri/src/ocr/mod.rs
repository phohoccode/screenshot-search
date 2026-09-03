pub mod engine;
pub mod mock;
pub mod normalize;
pub mod orchestrator;
pub mod windows;

#[cfg(test)]
mod ocr_tests;

pub use engine::{OcrEngine, OcrResult};
pub use mock::MockOcrEngine;
pub use normalize::normalize_ocr_text;
pub use orchestrator::{run_ocr_batch, OcrBatchSummary, OcrManager};
pub use windows::WindowsMediaOcrEngine;
