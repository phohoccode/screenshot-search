use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::errors::AppError;
use crate::ocr::engine::{OcrEngine, OcrEngineInfo, OcrResult};
use crate::ocr::normalize::normalize_ocr_text;
use ort::session::Session;
use ort::value::Tensor;

pub const DEFAULT_KEYS_DICT: &str = include_str!("keys.txt");
pub const EXPECTED_CLASS_DIM: usize = 6625;

/// High-accuracy local Multilingual OCR Engine using PaddleOCR v4 (Detection + Recognition).
/// Decodes text strictly from actual image pixels using ONNX Runtime.
pub struct MultilingualOcrEngine {
    models_dir: PathBuf,
    det_session: Arc<Mutex<Session>>,
    rec_session: Arc<Mutex<Session>>,
    keys: Arc<Vec<String>>,
    info: OcrEngineInfo,
}

impl MultilingualOcrEngine {
    pub fn models_dir(&self) -> &Path {
        &self.models_dir
    }

    pub fn new(models_dir: &Path) -> Result<Self, AppError> {
        let det_path = models_dir.join(crate::ocr::manager::DET_MODEL_FILENAME);
        let rec_path = models_dir.join(crate::ocr::manager::REC_MODEL_FILENAME);
        let keys_path = models_dir.join(crate::ocr::manager::KEYS_FILENAME);

        if !det_path.exists() || !rec_path.exists() {
            return Err(AppError::ocr_unavailable(format!(
                "Multilingual OCR model files missing in {}",
                models_dir.display()
            )));
        }

        // Ensure keys.txt is valid and populated
        let keys_content = if keys_path.exists() {
            let existing = fs::read_to_string(&keys_path).unwrap_or_default();
            if existing.lines().count() >= 6000 {
                existing
            } else {
                let _ = fs::write(&keys_path, DEFAULT_KEYS_DICT);
                DEFAULT_KEYS_DICT.to_string()
            }
        } else {
            let _ = fs::write(&keys_path, DEFAULT_KEYS_DICT);
            DEFAULT_KEYS_DICT.to_string()
        };

        // Construct CTC character dictionary: index 0 is blank, index 6624 is space
        let mut keys: Vec<String> = vec!["".to_string()]; // CTC blank
        for line in keys_content.lines() {
            keys.push(line.to_string());
        }
        keys.push(" ".to_string()); // space at end

        if keys.len() != EXPECTED_CLASS_DIM {
            return Err(AppError::ocr_unavailable(format!(
                "Dictionary size mismatch: expected {EXPECTED_CLASS_DIM} classes, found {}",
                keys.len()
            )));
        }

        let det_session = Session::builder()
            .and_then(|mut b| b.commit_from_file(&det_path))
            .map_err(|e| {
                AppError::ocr_unavailable(format!(
                    "Failed to load detection model {}: {e}",
                    det_path.display()
                ))
            })?;

        let rec_session = Session::builder()
            .and_then(|mut b| b.commit_from_file(&rec_path))
            .map_err(|e| {
                AppError::ocr_unavailable(format!(
                    "Failed to load recognition model {}: {e}",
                    rec_path.display()
                ))
            })?;

        let info = OcrEngineInfo {
            engine_name: "multilingual_ocr".to_string(),
            engine_version: "ppocr_v4".to_string(),
            active_language: "vi-VN/en".to_string(),
            available_languages: vec!["vi-VN".to_string(), "en-US".to_string()],
            supports_vietnamese: true,
            max_image_dimension: 4096,
        };

        Ok(Self {
            models_dir: models_dir.to_path_buf(),
            det_session: Arc::new(Mutex::new(det_session)),
            rec_session: Arc::new(Mutex::new(rec_session)),
            keys: Arc::new(keys),
            info,
        })
    }
}

/// Greedy CTC decoding: argmax per timestep, repeated-token collapse, CTC blank removal (index 0).
pub fn ctc_decode(slice: &[f32], time_steps: usize, class_dim: usize, keys: &[String]) -> String {
    if time_steps == 0 || class_dim == 0 || keys.is_empty() {
        return String::new();
    }

    let mut text = String::new();
    let mut prev_class = 0;

    for t in 0..time_steps {
        let offset = t * class_dim;
        if offset + class_dim > slice.len() {
            break;
        }

        let mut best_class = 0;
        let mut best_score = -f32::INFINITY;

        for c in 0..class_dim {
            let score = slice[offset + c];
            if score > best_score {
                best_score = score;
                best_class = c;
            }
        }

        if best_class != 0 && best_class != prev_class {
            if best_class < keys.len() {
                text.push_str(&keys[best_class]);
            }
        }
        prev_class = best_class;
    }

    text
}

/// Identifies text line bounding boxes from DBNet probability map using 4-way connected components.
fn detect_bounding_boxes(
    prob_map: &[f32],
    target_w: u32,
    target_h: u32,
    thresh: f32,
    min_area: usize,
) -> Vec<(u32, u32, u32, u32)> {
    let plane_size = (target_w * target_h) as usize;
    if prob_map.len() < plane_size {
        return Vec::new();
    }

    let mut visited = vec![false; plane_size];
    let mut boxes = Vec::new();

    for y in 0..target_h {
        for x in 0..target_w {
            let idx = (y * target_w + x) as usize;
            if !visited[idx] && prob_map[idx] >= thresh {
                let mut q = VecDeque::new();
                q.push_back((x, y));
                visited[idx] = true;

                let mut min_x = x;
                let mut max_x = x;
                let mut min_y = y;
                let mut max_y = y;
                let mut count = 0;

                while let Some((cx, cy)) = q.pop_front() {
                    count += 1;
                    min_x = min_x.min(cx);
                    max_x = max_x.max(cx);
                    min_y = min_y.min(cy);
                    max_y = max_y.max(cy);

                    for (dx, dy) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                        let nx = cx as i32 + dx;
                        let ny = cy as i32 + dy;
                        if nx >= 0 && nx < target_w as i32 && ny >= 0 && ny < target_h as i32 {
                            let n_idx = (ny as u32 * target_w + nx as u32) as usize;
                            if !visited[n_idx] && prob_map[n_idx] >= thresh {
                                visited[n_idx] = true;
                                q.push_back((nx as u32, ny as u32));
                            }
                        }
                    }
                }

                if count >= min_area && (max_x - min_x) >= 4 && (max_y - min_y) >= 4 {
                    boxes.push((min_x, min_y, max_x, max_y));
                }
            }
        }
    }

    // Sort reading order: top-to-bottom (bucketed by line band), left-to-right
    boxes.sort_by_key(|b| (b.1 / 14, b.0));

    boxes
}

impl OcrEngine for MultilingualOcrEngine {
    fn recognize(&self, image_path: &Path) -> Result<OcrResult, AppError> {
        if !image_path.exists() {
            return Err(AppError::file_not_found(format!(
                "Screenshot not found at: {}",
                image_path.display()
            )));
        }

        // 1. Decode original screenshot image from disk
        let img = image::open(image_path)
            .map_err(|e| {
                AppError::ocr_decode(format!(
                    "Failed to open image {}: {e}",
                    image_path.display()
                ))
            })?
            .to_rgb8();

        let (orig_w, orig_h) = (img.width(), img.height());

        // 2. Prepare detector tensor (scale with max dimension 960, multiple of 32)
        let max_dim = 960.0f32;
        let scale = (max_dim / (orig_w.max(orig_h) as f32)).min(1.0);
        let target_w = (((orig_w as f32 * scale) / 32.0).ceil() as u32 * 32).max(32);
        let target_h = (((orig_h as f32 * scale) / 32.0).ceil() as u32 * 32).max(32);

        let resized_det = image::imageops::resize(
            &img,
            target_w,
            target_h,
            image::imageops::FilterType::Triangle,
        );

        // Normalize image with ImageNet mean & std for PP-OCR DBNet detector
        let mean = [0.485f32, 0.456f32, 0.406f32];
        let std = [0.229f32, 0.224f32, 0.225f32];
        let plane_size = (target_h * target_w) as usize;
        let mut det_data = vec![0.0f32; 3 * plane_size];

        for y in 0..target_h {
            for x in 0..target_w {
                let p = resized_det.get_pixel(x, y);
                let idx = (y * target_w + x) as usize;
                det_data[0 * plane_size + idx] = (p[0] as f32 / 255.0 - mean[0]) / std[0];
                det_data[1 * plane_size + idx] = (p[1] as f32 / 255.0 - mean[1]) / std[1];
                det_data[2 * plane_size + idx] = (p[2] as f32 / 255.0 - mean[2]) / std[2];
            }
        }

        let det_tensor = Tensor::from_array((
            [1, 3, target_h as usize, target_w as usize],
            det_data.into_boxed_slice(),
        ))
        .map_err(|e| AppError::unknown(format!("Failed to build detector tensor: {e}")))?;

        // 3. Run ONNX Detection Inference
        let mut det_session = self
            .det_session
            .lock()
            .map_err(|e| AppError::unknown(format!("Failed to lock detector session: {e}")))?;
        let det_outputs = det_session
            .run(ort::inputs!["x" => det_tensor])
            .map_err(|e| AppError::unknown(format!("Detector ONNX inference failed: {e}")))?;

        let (_d_shape, d_slice) = det_outputs["sigmoid_0.tmp_0"]
            .try_extract_tensor::<f32>()
            .map_err(|e| AppError::unknown(format!("Failed to extract detector output: {e}")))?;

        // 4. Detect text region bounding boxes
        let boxes = detect_bounding_boxes(d_slice, target_w, target_h, 0.35, 16);
        drop(det_outputs);
        drop(det_session);

        if boxes.is_empty() {
            log::debug!(
                "Multilingual OCR: No text regions detected in {}",
                image_path.display()
            );
            return Ok(OcrResult {
                text: String::new(),
                engine: self.name().to_string(),
                engine_version: self.version().to_string(),
                language: Some("vi-VN".to_string()),
                confidence: Some(0.0),
            });
        }

        // 5. Crop and recognize each detected text region
        let mut recognized_lines = Vec::new();
        let mut rec_session = self
            .rec_session
            .lock()
            .map_err(|e| AppError::unknown(format!("Failed to lock recognizer session: {e}")))?;

        for b in &boxes {
            // Expand box by 10% to preserve diacritics and accents
            let box_w = b.2 - b.0 + 1;
            let box_h = b.3 - b.1 + 1;
            let pad_x = (box_w as f32 * 0.10) as u32;
            let pad_y = (box_h as f32 * 0.10) as u32;

            let bx0 = b.0.saturating_sub(pad_x);
            let by0 = b.1.saturating_sub(pad_y);
            let bx1 = (b.2 + pad_x).min(target_w - 1);
            let by1 = (b.3 + pad_y).min(target_h - 1);

            let orig_bx0 = ((bx0 as f32 * orig_w as f32 / target_w as f32) as u32).min(orig_w - 1);
            let orig_by0 = ((by0 as f32 * orig_h as f32 / target_h as f32) as u32).min(orig_h - 1);
            let orig_bx1 =
                (((bx1 + 1) as f32 * orig_w as f32 / target_w as f32) as u32).min(orig_w);
            let orig_by1 =
                (((by1 + 1) as f32 * orig_h as f32 / target_h as f32) as u32).min(orig_h);

            let crop_w = orig_bx1.saturating_sub(orig_bx0);
            let crop_h = orig_by1.saturating_sub(orig_by0);
            if crop_w < 4 || crop_h < 4 {
                continue;
            }

            let cropped =
                image::imageops::crop_imm(&img, orig_bx0, orig_by0, crop_w, crop_h).to_image();

            // Resize crop: height fixed to 48, width aspect-scaled (clamped between 48 and 960)
            let rec_h = 48u32;
            let rec_w = ((rec_h as f32 * crop_w as f32 / crop_h as f32) as u32).clamp(48, 960);
            let resized_crop = image::imageops::resize(
                &cropped,
                rec_w,
                rec_h,
                image::imageops::FilterType::Triangle,
            );

            let rec_plane_size = (rec_h * rec_w) as usize;
            let mut rec_data = vec![0.0f32; 3 * rec_plane_size];

            for y in 0..rec_h {
                for x in 0..rec_w {
                    let p = resized_crop.get_pixel(x, y);
                    let idx = (y * rec_w + x) as usize;
                    rec_data[0 * rec_plane_size + idx] = (p[0] as f32 / 255.0 - 0.5) / 0.5;
                    rec_data[1 * rec_plane_size + idx] = (p[1] as f32 / 255.0 - 0.5) / 0.5;
                    rec_data[2 * rec_plane_size + idx] = (p[2] as f32 / 255.0 - 0.5) / 0.5;
                }
            }

            let rec_tensor = Tensor::from_array((
                [1, 3, rec_h as usize, rec_w as usize],
                rec_data.into_boxed_slice(),
            ))
            .map_err(|e| AppError::unknown(format!("Failed to build recognizer tensor: {e}")))?;

            // Run ONNX Recognition Inference
            let rec_outputs = rec_session
                .run(ort::inputs!["x" => rec_tensor])
                .map_err(|e| AppError::unknown(format!("Recognizer ONNX inference failed: {e}")))?;

            let (r_shape, r_slice) = rec_outputs["softmax_11.tmp_0"]
                .try_extract_tensor::<f32>()
                .map_err(|e| {
                    AppError::unknown(format!("Failed to extract recognizer output: {e}"))
                })?;

            let time_steps = r_shape[1] as usize;
            let class_dim = r_shape[2] as usize;

            let line_text = ctc_decode(r_slice, time_steps, class_dim, &self.keys);
            let trimmed = line_text.trim();
            if !trimmed.is_empty() {
                recognized_lines.push(trimmed.to_string());
            }
        }

        let full_text = recognized_lines.join("\n");
        let normalized = normalize_ocr_text(&full_text);

        Ok(OcrResult {
            text: normalized,
            engine: self.name().to_string(),
            engine_version: self.version().to_string(),
            language: Some("vi-VN".to_string()),
            confidence: Some(0.95),
        })
    }

    fn get_info(&self) -> OcrEngineInfo {
        self.info.clone()
    }

    fn name(&self) -> &str {
        "multilingual_ocr"
    }

    fn version(&self) -> &str {
        "ppocr_v4"
    }
}
