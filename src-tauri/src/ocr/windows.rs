use std::path::Path;

use super::engine::{OcrEngine, OcrResult};
use super::normalize::normalize_ocr_text;
use crate::errors::AppError;

/// Local OCR engine utilizing the built-in Windows 10/11 WinRT OCR APIs (Windows.Media.Ocr).
/// Zero external model download, minimal memory footprint, hardware-accelerated CPU execution.
pub struct WindowsMediaOcrEngine;

impl WindowsMediaOcrEngine {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WindowsMediaOcrEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "windows")]
impl OcrEngine for WindowsMediaOcrEngine {
    fn recognize(&self, image_path: &Path) -> Result<OcrResult, AppError> {
        use windows::Graphics::Imaging::BitmapDecoder;
        use windows::Media::Ocr::OcrEngine as WinRtOcrEngine;
        use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

        if !image_path.exists() {
            return Err(AppError::ocr_decode(format!(
                "Image file does not exist: {}",
                image_path.display()
            )));
        }

        // 1. Read file bytes
        let bytes = std::fs::read(image_path).map_err(|e| {
            AppError::ocr_decode(format!(
                "Failed to read image file {}: {e}",
                image_path.display()
            ))
        })?;

        // 2. Initialize WinRT OCR engine from user language profile
        let engine = WinRtOcrEngine::TryCreateFromUserProfileLanguages().map_err(|e| {
            AppError::ocr_unavailable(format!(
                "Windows OCR engine is unavailable or language pack missing: {e}"
            ))
        })?;

        // 3. Decode image bytes into a SoftwareBitmap
        let stream = InMemoryRandomAccessStream::new()
            .map_err(|e| AppError::ocr_decode(format!("Failed to create in-memory stream: {e}")))?;

        let writer = DataWriter::CreateDataWriter(&stream)
            .map_err(|e| AppError::ocr_decode(format!("Failed to create stream writer: {e}")))?;

        writer
            .WriteBytes(&bytes)
            .map_err(|e| AppError::ocr_decode(format!("Failed to write bytes to stream: {e}")))?;

        writer
            .StoreAsync()
            .map_err(|e| AppError::ocr_decode(format!("Failed to store stream async: {e}")))?
            .get()
            .map_err(|e| AppError::ocr_decode(format!("Stream store operation failed: {e}")))?;

        stream.Seek(0).map_err(|e| {
            AppError::ocr_decode(format!("Failed to seek stream to beginning: {e}"))
        })?;

        let decoder = BitmapDecoder::CreateAsync(&stream)
            .map_err(|e| AppError::ocr_decode(format!("Failed to create bitmap decoder: {e}")))?
            .get()
            .map_err(|e| {
                AppError::ocr_decode(format!(
                    "Image decoding failed (unsupported or corrupted format): {e}"
                ))
            })?;

        let bitmap = decoder
            .GetSoftwareBitmapAsync()
            .map_err(|e| AppError::ocr_decode(format!("Failed to get software bitmap: {e}")))?
            .get()
            .map_err(|e| AppError::ocr_decode(format!("Software bitmap conversion failed: {e}")))?;

        // 4. Perform recognition
        let ocr_result = engine
            .RecognizeAsync(&bitmap)
            .map_err(|e| AppError::ocr(format!("Recognition failed: {e}")))?
            .get()
            .map_err(|e| AppError::ocr(format!("OCR recognition execution error: {e}")))?;

        let raw_text = ocr_result
            .Text()
            .map_err(|e| AppError::ocr(format!("Failed to extract recognized text: {e}")))?
            .to_string();

        let normalized = normalize_ocr_text(&raw_text);

        Ok(OcrResult {
            text: normalized,
            engine: self.name().to_string(),
            engine_version: self.version().to_string(),
            confidence: None,
        })
    }

    fn name(&self) -> &str {
        "windows_media_ocr"
    }

    fn version(&self) -> &str {
        "winrt_v1"
    }
}

#[cfg(not(target_os = "windows"))]
impl OcrEngine for WindowsMediaOcrEngine {
    fn recognize(&self, _image_path: &Path) -> Result<OcrResult, AppError> {
        Err(AppError::ocr_unavailable(
            "Windows.Media.Ocr is only available on Windows operating systems",
        ))
    }

    fn name(&self) -> &str {
        "windows_media_ocr"
    }

    fn version(&self) -> &str {
        "unsupported_os"
    }
}
