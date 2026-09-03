use std::path::Path;

use super::engine::{OcrEngine, OcrEngineInfo, OcrResult};
use super::normalize::normalize_ocr_text;
use crate::errors::AppError;

/// Local OCR engine utilizing the built-in Windows 10/11 WinRT OCR APIs (Windows.Media.Ocr).
/// Pre-initializes and reuses the native engine instance, safely downscaling oversized
/// screenshots (e.g. 4K, ultra-wide) in-memory before recognition without modifying disk files.
pub struct WindowsMediaOcrEngine {
    #[cfg(target_os = "windows")]
    engine: Option<windows::Media::Ocr::OcrEngine>,
    info: OcrEngineInfo,
}

impl WindowsMediaOcrEngine {
    pub fn new() -> Self {
        #[cfg(target_os = "windows")]
        {
            use windows::Media::Ocr::OcrEngine as WinRtOcrEngine;

            let available_langs: Vec<String> = WinRtOcrEngine::AvailableRecognizerLanguages()
                .map(|langs| {
                    let mut list = Vec::new();
                    for lang in langs {
                        if let Ok(tag) = lang.LanguageTag() {
                            list.push(tag.to_string());
                        }
                    }
                    list
                })
                .unwrap_or_default();

            let supports_vietnamese = available_langs
                .iter()
                .any(|tag| tag.eq_ignore_ascii_case("vi") || tag.to_lowercase().starts_with("vi-"));

            match WinRtOcrEngine::TryCreateFromUserProfileLanguages() {
                Ok(engine) => {
                    let active_language = engine
                        .RecognizerLanguage()
                        .and_then(|l| l.LanguageTag())
                        .map(|t| t.to_string())
                        .unwrap_or_else(|_| "unknown".to_string());

                    let max_image_dimension = engine.MaxImageDimension().unwrap_or(2600);

                    Self {
                        engine: Some(engine),
                        info: OcrEngineInfo {
                            engine_name: "windows_media_ocr".to_string(),
                            engine_version: "winrt_v1".to_string(),
                            active_language,
                            available_languages: available_langs,
                            supports_vietnamese,
                            max_image_dimension,
                        },
                    }
                }
                Err(e) => {
                    log::warn!("Failed to pre-initialize Windows Media OCR engine: {e}");
                    Self {
                        engine: None,
                        info: OcrEngineInfo {
                            engine_name: "windows_media_ocr".to_string(),
                            engine_version: "winrt_v1".to_string(),
                            active_language: "none".to_string(),
                            available_languages: available_langs,
                            supports_vietnamese,
                            max_image_dimension: 2600,
                        },
                    }
                }
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            Self {
                info: OcrEngineInfo {
                    engine_name: "windows_media_ocr".to_string(),
                    engine_version: "unsupported_os".to_string(),
                    active_language: "none".to_string(),
                    available_languages: Vec::new(),
                    supports_vietnamese: false,
                    max_image_dimension: 2600,
                },
            }
        }
    }
}

impl Default for WindowsMediaOcrEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to calculate aspect-ratio-preserving dimensions for oversized images.
pub fn calculate_downscaled_dimensions(width: u32, height: u32, max_dimension: u32) -> (u32, u32) {
    if width <= max_dimension && height <= max_dimension {
        return (width, height);
    }

    let max_side = width.max(height) as f64;
    let scale = (max_dimension as f64) / max_side;

    let target_width = ((width as f64) * scale).round() as u32;
    let target_height = ((height as f64) * scale).round() as u32;

    (target_width.max(1), target_height.max(1))
}

#[cfg(target_os = "windows")]
impl OcrEngine for WindowsMediaOcrEngine {
    fn recognize(&self, image_path: &Path) -> Result<OcrResult, AppError> {
        use windows::Graphics::Imaging::{BitmapDecoder, BitmapInterpolationMode, BitmapTransform};
        use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

        let engine = self.engine.as_ref().ok_or_else(|| {
            AppError::ocr_unavailable(
                "Windows Media OCR engine is not initialized or supported on this system",
            )
        })?;

        if !image_path.exists() {
            return Err(AppError::ocr_decode(format!(
                "Image file does not exist: {}",
                image_path.display()
            )));
        }

        // 1. Read file bytes from disk (original file is never modified)
        let bytes = std::fs::read(image_path).map_err(|e| {
            AppError::ocr_decode(format!(
                "Failed to read image file {}: {e}",
                image_path.display()
            ))
        })?;

        // 2. Decode image stream in memory
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

        // 3. Inspect dimensions and safely downscale in-memory if dimensions exceed MaxImageDimension
        let pixel_width = decoder.PixelWidth().unwrap_or(0);
        let pixel_height = decoder.PixelHeight().unwrap_or(0);
        let max_dim = self.info.max_image_dimension;

        let bitmap = if pixel_width > max_dim || pixel_height > max_dim {
            let (target_width, target_height) =
                calculate_downscaled_dimensions(pixel_width, pixel_height, max_dim);

            log::debug!(
                "Screenshot {} exceeds OCR MaxImageDimension ({}x{} > {}). Downscaling in-memory to {}x{} for recognition",
                image_path.display(),
                pixel_width,
                pixel_height,
                max_dim,
                target_width,
                target_height
            );

            let transform = BitmapTransform::new().map_err(|e| {
                AppError::ocr_decode(format!("Failed to create BitmapTransform: {e}"))
            })?;
            transform
                .SetScaledWidth(target_width)
                .map_err(|e| AppError::ocr_decode(format!("Failed to set scaled width: {e}")))?;
            transform
                .SetScaledHeight(target_height)
                .map_err(|e| AppError::ocr_decode(format!("Failed to set scaled height: {e}")))?;
            transform
                .SetInterpolationMode(BitmapInterpolationMode::Fant)
                .map_err(|e| {
                    AppError::ocr_decode(format!("Failed to set interpolation mode: {e}"))
                })?;

            decoder
                .GetSoftwareBitmapAsync_WithTransform(
                    decoder
                        .BitmapPixelFormat()
                        .map_err(|e| AppError::ocr_decode(e.to_string()))?,
                    decoder
                        .BitmapAlphaMode()
                        .map_err(|e| AppError::ocr_decode(e.to_string()))?,
                    &transform,
                    windows::Graphics::Imaging::ExifOrientationMode::IgnoreExifOrientation,
                    windows::Graphics::Imaging::ColorManagementMode::DoNotColorManage,
                )
                .map_err(|e| {
                    AppError::ocr_decode(format!("Failed to get downscaled software bitmap: {e}"))
                })?
                .get()
                .map_err(|e| {
                    AppError::ocr_decode(format!(
                        "Downscaled software bitmap transform failed: {e}"
                    ))
                })?
        } else {
            decoder
                .GetSoftwareBitmapAsync()
                .map_err(|e| AppError::ocr_decode(format!("Failed to get software bitmap: {e}")))?
                .get()
                .map_err(|e| {
                    AppError::ocr_decode(format!("Software bitmap conversion failed: {e}"))
                })?
        };

        // 4. Perform recognition using the pre-initialized engine
        let ocr_result = engine
            .RecognizeAsync(&bitmap)
            .map_err(|e| AppError::ocr(format!("Recognition execution failed: {e}")))?
            .get()
            .map_err(|e| AppError::ocr(format!("OCR recognition result extraction error: {e}")))?;

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

    fn get_info(&self) -> OcrEngineInfo {
        self.info.clone()
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

    fn get_info(&self) -> OcrEngineInfo {
        self.info.clone()
    }

    fn name(&self) -> &str {
        "windows_media_ocr"
    }

    fn version(&self) -> &str {
        "unsupported_os"
    }
}
