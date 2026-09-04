use std::path::Path;

use super::engine::{OcrEngine, OcrEngineInfo, OcrResult};
use super::normalize::normalize_ocr_text;
use crate::errors::AppError;

/// Preprocessing variants for text line crop recognition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropPreprocessingVariant {
    CurrentBaseline,
    GenerousPadding,
    Upscale2xNearest,
    Upscale2xLinear,
    Upscale2xFant,
    Upscale3xLinear,
    GrayscaleContrast,
    GrayscaleContrastUpscale2x,
}

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

            let vi_lang = windows::Globalization::Language::CreateLanguage(
                &windows::core::HSTRING::from("vi-VN"),
            )
            .or_else(|_| {
                windows::Globalization::Language::CreateLanguage(&windows::core::HSTRING::from(
                    "vi",
                ))
            });

            let vi_supported = if let Ok(ref lang) = vi_lang {
                WinRtOcrEngine::IsLanguageSupported(lang).unwrap_or(false)
            } else {
                false
            };

            let supports_vietnamese = available_langs
                .iter()
                .any(|tag| tag.eq_ignore_ascii_case("vi") || tag.to_lowercase().starts_with("vi-"))
                || vi_supported;

            match WinRtOcrEngine::TryCreateFromUserProfileLanguages() {
                Ok(engine) => {
                    let active_language = engine
                        .RecognizerLanguage()
                        .and_then(|l| l.LanguageTag())
                        .map(|t| t.to_string())
                        .unwrap_or_else(|_| "unknown".to_string());

                    let max_image_dimension = WinRtOcrEngine::MaxImageDimension().unwrap_or(2600);

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

    /// Performs in-memory OCR recognition on a cropped text line image using default baseline settings.
    pub fn recognize_crop(&self, crop: &image::RgbImage) -> Result<String, AppError> {
        self.recognize_crop_variant(crop, CropPreprocessingVariant::CurrentBaseline)
    }

    /// Performs in-memory OCR recognition on a cropped text line image with selectable preprocessing variant.
    pub fn recognize_crop_variant(
        &self,
        crop: &image::RgbImage,
        variant: CropPreprocessingVariant,
    ) -> Result<String, AppError> {
        #[cfg(target_os = "windows")]
        {
            use windows::Graphics::Imaging::{
                BitmapDecoder, BitmapInterpolationMode, BitmapTransform, ColorManagementMode,
                ExifOrientationMode,
            };
            use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

            let engine = self.engine.as_ref().ok_or_else(|| {
                AppError::ocr_unavailable(
                    "Windows Media OCR engine is not initialized or supported on this system",
                )
            })?;

            let (w, h) = (crop.width(), crop.height());
            if w < 2 || h < 2 {
                return Ok(String::new());
            }

            // 1. Contrast / Grayscale preprocessing if requested
            let working_crop = match variant {
                CropPreprocessingVariant::GrayscaleContrast
                | CropPreprocessingVariant::GrayscaleContrastUpscale2x => {
                    let mut min_l = 255u8;
                    let mut max_l = 0u8;
                    for p in crop.pixels() {
                        let l =
                            (0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32) as u8;
                        min_l = min_l.min(l);
                        max_l = max_l.max(l);
                    }
                    let mut processed = crop.clone();
                    if max_l > min_l {
                        let range = (max_l - min_l) as f32;
                        for p in processed.pixels_mut() {
                            let l = (0.299 * p[0] as f32
                                + 0.587 * p[1] as f32
                                + 0.114 * p[2] as f32) as u8;
                            let stretched =
                                (((l.saturating_sub(min_l)) as f32 / range) * 255.0).round() as u8;
                            *p = image::Rgb([stretched, stretched, stretched]);
                        }
                    }
                    std::borrow::Cow::Owned(processed)
                }
                _ => std::borrow::Cow::Borrowed(crop),
            };

            // 2. Padding
            let (pad_x, pad_y) = match variant {
                CropPreprocessingVariant::GenerousPadding => (48u32, 24u32),
                _ => (32u32, 16u32),
            };
            let padded_w = w + pad_x * 2;
            let padded_h = h + pad_y * 2;
            let mut padded =
                image::RgbImage::from_pixel(padded_w, padded_h, image::Rgb([255, 255, 255]));
            image::imageops::overlay(
                &mut padded,
                working_crop.as_ref(),
                pad_x as i64,
                pad_y as i64,
            );

            // 3. Encode padded crop to PNG in memory
            let mut png_bytes = Vec::new();
            let mut cursor = std::io::Cursor::new(&mut png_bytes);
            padded
                .write_to(&mut cursor, image::ImageFormat::Png)
                .map_err(|e| AppError::ocr_decode(format!("Failed to encode crop to PNG: {e}")))?;

            let stream = InMemoryRandomAccessStream::new().map_err(|e| {
                AppError::ocr_decode(format!("Failed to create in-memory stream: {e}"))
            })?;

            let writer = DataWriter::CreateDataWriter(&stream).map_err(|e| {
                AppError::ocr_decode(format!("Failed to create stream writer: {e}"))
            })?;

            writer.WriteBytes(&png_bytes).map_err(|e| {
                AppError::ocr_decode(format!("Failed to write bytes to stream: {e}"))
            })?;

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
                    AppError::ocr_decode(format!("Image decoding failed for line crop: {e}"))
                })?;

            let pixel_format = decoder
                .BitmapPixelFormat()
                .map_err(|e| AppError::ocr_decode(e.to_string()))?;
            let alpha_mode = decoder
                .BitmapAlphaMode()
                .map_err(|e| AppError::ocr_decode(e.to_string()))?;

            // 4. Scaling transform setup based on variant
            let (need_scale, scale_factor, interp_mode) = match variant {
                CropPreprocessingVariant::CurrentBaseline => {
                    if h < 36 {
                        (
                            true,
                            (36.0 / (h as f64)).max(1.75),
                            BitmapInterpolationMode::Linear,
                        )
                    } else {
                        (false, 1.0, BitmapInterpolationMode::Linear)
                    }
                }
                CropPreprocessingVariant::GenerousPadding => {
                    (false, 1.0, BitmapInterpolationMode::Linear)
                }
                CropPreprocessingVariant::Upscale2xNearest => {
                    (true, 2.0, BitmapInterpolationMode::NearestNeighbor)
                }
                CropPreprocessingVariant::Upscale2xLinear => {
                    (true, 2.0, BitmapInterpolationMode::Linear)
                }
                CropPreprocessingVariant::Upscale2xFant => {
                    (true, 2.0, BitmapInterpolationMode::Fant)
                }
                CropPreprocessingVariant::Upscale3xLinear => {
                    (true, 3.0, BitmapInterpolationMode::Linear)
                }
                CropPreprocessingVariant::GrayscaleContrast => {
                    (false, 1.0, BitmapInterpolationMode::Linear)
                }
                CropPreprocessingVariant::GrayscaleContrastUpscale2x => {
                    (true, 2.0, BitmapInterpolationMode::Fant)
                }
            };

            let bitmap = if need_scale {
                let target_w = ((padded_w as f64) * scale_factor).round() as u32;
                let target_h = ((padded_h as f64) * scale_factor).round() as u32;

                let transform = BitmapTransform::new().map_err(|e| {
                    AppError::ocr_decode(format!("Failed to create BitmapTransform: {e}"))
                })?;
                transform.SetScaledWidth(target_w).map_err(|e| {
                    AppError::ocr_decode(format!("Failed to set scaled width: {e}"))
                })?;
                transform.SetScaledHeight(target_h).map_err(|e| {
                    AppError::ocr_decode(format!("Failed to set scaled height: {e}"))
                })?;
                transform.SetInterpolationMode(interp_mode).map_err(|e| {
                    AppError::ocr_decode(format!("Failed to set interpolation mode: {e}"))
                })?;

                decoder
                    .GetSoftwareBitmapTransformedAsync(
                        pixel_format,
                        alpha_mode,
                        &transform,
                        ExifOrientationMode::IgnoreExifOrientation,
                        ColorManagementMode::DoNotColorManage,
                    )
                    .map_err(|e| {
                        AppError::ocr_decode(format!(
                            "Failed to get transformed software bitmap: {e}"
                        ))
                    })?
                    .get()
                    .map_err(|e| {
                        AppError::ocr_decode(format!("Crop software bitmap transform failed: {e}"))
                    })?
            } else {
                decoder
                    .GetSoftwareBitmapAsync()
                    .map_err(|e| {
                        AppError::ocr_decode(format!("Failed to get software bitmap: {e}"))
                    })?
                    .get()
                    .map_err(|e| {
                        AppError::ocr_decode(format!("Software bitmap conversion failed: {e}"))
                    })?
            };

            let ocr_result = engine
                .RecognizeAsync(&bitmap)
                .map_err(|e| AppError::ocr(format!("Crop recognition execution failed: {e}")))?
                .get()
                .map_err(|e| {
                    AppError::ocr(format!("Crop OCR recognition result extraction error: {e}"))
                })?;

            let text = ocr_result
                .Text()
                .map_err(|e| AppError::ocr(format!("Failed to extract recognized text: {e}")))?
                .to_string();

            Ok(text.trim().to_string())
        }

        #[cfg(not(target_os = "windows"))]
        {
            let _ = crop;
            let _ = variant;
            Ok(String::new())
        }
    }
}

impl Default for WindowsMediaOcrEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Bounded sub-rectangle for in-memory image tile extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileBounds {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[cfg(target_os = "windows")]
impl From<TileBounds> for windows::Graphics::Imaging::BitmapBounds {
    fn from(t: TileBounds) -> Self {
        Self {
            X: t.x,
            Y: t.y,
            Width: t.width,
            Height: t.height,
        }
    }
}

/// Adaptive OCR processing strategy based on image dimension and aspect ratio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OcrProcessingStrategy {
    /// Normal image (<= 2600px): direct recognition at 100% native resolution.
    Direct,
    /// Moderately oversized (aspect ratio <= 2.2, max side > 2600px, e.g. 4K 3840x2160):
    /// Proportional downscaling preserving aspect ratio with Fant interpolation.
    ProportionalDownscale {
        target_width: u32,
        target_height: u32,
    },
    /// Extremely tall screenshot (aspect ratio > 2.2, height > 2600px, e.g. 1080x5200, 1440x10000):
    /// Split vertically into bounded overlapping tiles to preserve native text resolution.
    VerticalTiling { tiles: Vec<TileBounds> },
    /// Extremely wide screenshot (aspect ratio > 2.2, width > 2600px, e.g. 5120x1440):
    /// Split horizontally into bounded overlapping tiles.
    HorizontalTiling { tiles: Vec<TileBounds> },
}

pub const SAFE_MAX_OCR_DIMENSION: u32 = 2600;
pub const TILE_SIZE: u32 = 2000;
pub const TILE_OVERLAP: u32 = 150;

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

/// Determines the optimal OCR processing strategy for the given dimensions.
pub fn determine_processing_strategy(
    width: u32,
    height: u32,
    max_dimension: u32,
) -> OcrProcessingStrategy {
    // 1. Extremely tall screenshot: height > 2x width and height > TILE_SIZE
    if height > width * 2 && height > TILE_SIZE {
        let step = TILE_SIZE.saturating_sub(TILE_OVERLAP).max(1);
        let mut tiles = Vec::new();
        let mut y = 0;
        while y < height {
            let cur_h = (height - y).min(TILE_SIZE);
            tiles.push(TileBounds {
                x: 0,
                y,
                width,
                height: cur_h,
            });
            if y + cur_h >= height {
                break;
            }
            y += step;
        }
        return OcrProcessingStrategy::VerticalTiling { tiles };
    }

    // 2. Extremely wide screenshot: width > 2x height and width > TILE_SIZE
    if width > height * 2 && width > TILE_SIZE {
        let step = TILE_SIZE.saturating_sub(TILE_OVERLAP).max(1);
        let mut tiles = Vec::new();
        let mut x = 0;
        while x < width {
            let cur_w = (width - x).min(TILE_SIZE);
            tiles.push(TileBounds {
                x,
                y: 0,
                width: cur_w,
                height,
            });
            if x + cur_w >= width {
                break;
            }
            x += step;
        }
        return OcrProcessingStrategy::HorizontalTiling { tiles };
    }

    // 3. Normal / Moderate aspect ratio:
    // If within runtime max dimension, process directly at native resolution.
    if width <= max_dimension && height <= max_dimension {
        return OcrProcessingStrategy::Direct;
    }

    // 4. Moderately oversized: proportionally downscale to fit within max_dimension.
    let (target_width, target_height) =
        calculate_downscaled_dimensions(width, height, max_dimension);
    OcrProcessingStrategy::ProportionalDownscale {
        target_width,
        target_height,
    }
}

/// Merges text lines extracted from sequential overlapping tiles in deterministic reading order.
/// Conservatively deduplicates overlapping lines and heals partially cropped boundary lines.
pub fn merge_tile_texts(tile_texts: &[String]) -> String {
    let mut merged_lines: Vec<String> = Vec::new();

    for text in tile_texts {
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            // Check if this line was already captured near the tail of previous tiles
            let tail_start = merged_lines.len().saturating_sub(6);
            let mut is_dup = false;

            for i in tail_start..merged_lines.len() {
                let existing = merged_lines[i].trim();
                if existing.eq_ignore_ascii_case(trimmed) {
                    is_dup = true;
                    break;
                }
                // If a previous line was partially chopped off at tile border, replace with fuller version
                if trimmed.len() > existing.len() && trimmed.starts_with(existing) {
                    merged_lines[i] = trimmed.to_string();
                    is_dup = true;
                    break;
                }
            }

            if !is_dup {
                merged_lines.push(trimmed.to_string());
            }
        }
    }

    merged_lines.join("\n")
}

#[cfg(target_os = "windows")]
impl OcrEngine for WindowsMediaOcrEngine {
    fn recognize(&self, image_path: &Path) -> Result<OcrResult, AppError> {
        use windows::Graphics::Imaging::{
            BitmapDecoder, BitmapInterpolationMode, BitmapTransform, ColorManagementMode,
            ExifOrientationMode,
        };
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

        // 1. Read file bytes from disk (original file is strictly never modified)
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

        // 3. Inspect dimensions and determine optimal processing strategy
        let pixel_width = decoder.PixelWidth().unwrap_or(0);
        let pixel_height = decoder.PixelHeight().unwrap_or(0);
        let max_dim = self.info.max_image_dimension;

        let strategy = determine_processing_strategy(pixel_width, pixel_height, max_dim);
        log::debug!(
            "OCR processing strategy for {} ({}x{}): {:?}",
            image_path.display(),
            pixel_width,
            pixel_height,
            strategy
        );

        let pixel_format = decoder
            .BitmapPixelFormat()
            .map_err(|e| AppError::ocr_decode(e.to_string()))?;
        let alpha_mode = decoder
            .BitmapAlphaMode()
            .map_err(|e| AppError::ocr_decode(e.to_string()))?;

        // 4. Perform recognition according to strategy
        let raw_text = match strategy {
            OcrProcessingStrategy::Direct => {
                let bitmap = decoder
                    .GetSoftwareBitmapAsync()
                    .map_err(|e| {
                        AppError::ocr_decode(format!("Failed to get software bitmap: {e}"))
                    })?
                    .get()
                    .map_err(|e| {
                        AppError::ocr_decode(format!("Software bitmap conversion failed: {e}"))
                    })?;

                let ocr_result = engine
                    .RecognizeAsync(&bitmap)
                    .map_err(|e| AppError::ocr(format!("Recognition execution failed: {e}")))?
                    .get()
                    .map_err(|e| {
                        AppError::ocr(format!("OCR recognition result extraction error: {e}"))
                    })?;

                ocr_result
                    .Text()
                    .map_err(|e| AppError::ocr(format!("Failed to extract recognized text: {e}")))?
                    .to_string()
            }
            OcrProcessingStrategy::ProportionalDownscale {
                target_width,
                target_height,
            } => {
                let transform = BitmapTransform::new().map_err(|e| {
                    AppError::ocr_decode(format!("Failed to create BitmapTransform: {e}"))
                })?;
                transform.SetScaledWidth(target_width).map_err(|e| {
                    AppError::ocr_decode(format!("Failed to set scaled width: {e}"))
                })?;
                transform.SetScaledHeight(target_height).map_err(|e| {
                    AppError::ocr_decode(format!("Failed to set scaled height: {e}"))
                })?;
                transform
                    .SetInterpolationMode(BitmapInterpolationMode::Fant)
                    .map_err(|e| {
                        AppError::ocr_decode(format!("Failed to set interpolation mode: {e}"))
                    })?;

                let bitmap = decoder
                    .GetSoftwareBitmapTransformedAsync(
                        pixel_format,
                        alpha_mode,
                        &transform,
                        ExifOrientationMode::IgnoreExifOrientation,
                        ColorManagementMode::DoNotColorManage,
                    )
                    .map_err(|e| {
                        AppError::ocr_decode(format!(
                            "Failed to get downscaled software bitmap: {e}"
                        ))
                    })?
                    .get()
                    .map_err(|e| {
                        AppError::ocr_decode(format!(
                            "Downscaled software bitmap transform failed: {e}"
                        ))
                    })?;

                let ocr_result = engine
                    .RecognizeAsync(&bitmap)
                    .map_err(|e| AppError::ocr(format!("Recognition execution failed: {e}")))?
                    .get()
                    .map_err(|e| {
                        AppError::ocr(format!("OCR recognition result extraction error: {e}"))
                    })?;

                ocr_result
                    .Text()
                    .map_err(|e| AppError::ocr(format!("Failed to extract recognized text: {e}")))?
                    .to_string()
            }
            OcrProcessingStrategy::VerticalTiling { tiles }
            | OcrProcessingStrategy::HorizontalTiling { tiles } => {
                let mut tile_texts = Vec::with_capacity(tiles.len());

                for (idx, tile) in tiles.into_iter().enumerate() {
                    let transform = BitmapTransform::new().map_err(|e| {
                        AppError::ocr_decode(format!("Failed to create tile BitmapTransform: {e}"))
                    })?;
                    transform.SetBounds(tile.into()).map_err(|e| {
                        AppError::ocr_decode(format!(
                            "Failed to set tile bounds for tile {idx}: {e}"
                        ))
                    })?;

                    let bitmap = decoder
                        .GetSoftwareBitmapTransformedAsync(
                            pixel_format,
                            alpha_mode,
                            &transform,
                            ExifOrientationMode::IgnoreExifOrientation,
                            ColorManagementMode::DoNotColorManage,
                        )
                        .map_err(|e| {
                            AppError::ocr_decode(format!(
                                "Failed to extract software bitmap for tile {idx}: {e}"
                            ))
                        })?
                        .get()
                        .map_err(|e| {
                            AppError::ocr_decode(format!(
                                "Tile software bitmap transform failed for tile {idx}: {e}"
                            ))
                        })?;

                    let ocr_result = engine
                        .RecognizeAsync(&bitmap)
                        .map_err(|e| {
                            AppError::ocr(format!("Recognition failed for tile {idx}: {e}"))
                        })?
                        .get()
                        .map_err(|e| {
                            AppError::ocr(format!("Result extraction failed for tile {idx}: {e}"))
                        })?;

                    let text = ocr_result
                        .Text()
                        .map_err(|e| {
                            AppError::ocr(format!("Text extraction failed for tile {idx}: {e}"))
                        })?
                        .to_string();

                    tile_texts.push(text);
                }

                merge_tile_texts(&tile_texts)
            }
        };

        let normalized = normalize_ocr_text(&raw_text);

        Ok(OcrResult {
            text: normalized,
            engine: self.name().to_string(),
            engine_version: self.version().to_string(),
            language: Some(self.info.active_language.clone()),
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
