use std::collections::VecDeque;
use std::path::Path;
use std::sync::{Arc, Mutex};

use image::RgbImage;
use ort::session::Session;
use ort::value::Tensor;

use crate::errors::AppError;

/// Maximum number of detected text lines processed per screenshot to prevent pathological DoS.
pub const MAX_DETECTED_LINES_PER_SCREENSHOT: usize = 128;

/// Bounding box in original image pixel coordinates (x0, y0, x1, y1).
pub type BoundingBox = (u32, u32, u32, u32);

/// A detected text line region containing the cropped image and spatial metadata.
#[derive(Clone)]
pub struct DetectedTextLine {
    pub crop: RgbImage,
    pub box_rect: BoundingBox,
    pub line_index: usize,
}

/// Standalone, reusable DBNet text line detector powered by ONNX Runtime.
pub struct TextLineDetector {
    det_session: Arc<Mutex<Session>>,
}

impl TextLineDetector {
    /// Loads a detector instance from the specified ONNX model file path.
    pub fn new(model_path: &Path) -> Result<Self, AppError> {
        if !model_path.exists() {
            return Err(AppError::ocr_unavailable(format!(
                "Detector model not found at {}",
                model_path.display()
            )));
        }

        let session = Session::builder()
            .and_then(|mut b| b.commit_from_file(model_path))
            .map_err(|e| {
                AppError::ocr_unavailable(format!(
                    "Failed to initialize detector ONNX session from {}: {e}",
                    model_path.display()
                ))
            })?;

        Ok(Self {
            det_session: Arc::new(Mutex::new(session)),
        })
    }

    /// Detects text lines from an in-memory image, returns crops and bounding boxes.
    pub fn detect_lines(&self, img: &RgbImage) -> Result<Vec<DetectedTextLine>, AppError> {
        let (orig_w, orig_h) = (img.width(), img.height());
        if orig_w == 0 || orig_h == 0 {
            return Ok(Vec::new());
        }

        // 1. Prepare detector tensor: scale with max dimension 960 (multiple of 32)
        let max_dim = 960.0f32;
        let scale = (max_dim / (orig_w.max(orig_h) as f32)).min(1.0);
        let target_w = (((orig_w as f32 * scale) / 32.0).ceil() as u32 * 32).max(32);
        let target_h = (((orig_h as f32 * scale) / 32.0).ceil() as u32 * 32).max(32);

        let resized_det = image::imageops::resize(
            img,
            target_w,
            target_h,
            image::imageops::FilterType::Triangle,
        );

        // Normalize with ImageNet mean and std
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

        // 2. Run ONNX detector inference
        let mut session = self
            .det_session
            .lock()
            .map_err(|e| AppError::unknown(format!("Detector session lock failed: {e}")))?;

        let det_outputs = session
            .run(ort::inputs!["x" => det_tensor])
            .map_err(|e| AppError::unknown(format!("Detector ONNX inference failed: {e}")))?;

        let (_d_shape, d_slice) = det_outputs["sigmoid_0.tmp_0"]
            .try_extract_tensor::<f32>()
            .map_err(|e| AppError::unknown(format!("Failed to extract detector output: {e}")))?;

        // 3. Extract connected components
        let raw_boxes = Self::find_boxes_from_prob_map(d_slice, target_w, target_h, 0.35, 16);
        drop(det_outputs);
        drop(session);

        if raw_boxes.is_empty() {
            return Ok(Vec::new());
        }

        // 4. Crop and map to original image coordinate space with diacritic unclip padding
        let mut lines = Vec::new();

        for (idx, b) in raw_boxes.into_iter().enumerate() {
            if lines.len() >= MAX_DETECTED_LINES_PER_SCREENSHOT {
                log::warn!(
                    "Screenshot line count exceeded safety cap ({MAX_DETECTED_LINES_PER_SCREENSHOT}). Truncating to avoid DoS."
                );
                break;
            }

            let box_w = b.2 - b.0 + 1;
            let box_h = b.3 - b.1 + 1;

            // Diacritic and Descender Unclip Expansion: expand vertically by 55% (min 6px)
            // and horizontally by 15% (min 4px) to ensure top tone marks (hooks, hats, tildes)
            // and bottom descenders / dots ('j', 'p', 'g', 'y', dấu nặng) are fully preserved.
            let pad_x = ((box_w as f32 * 0.15) as u32).max(4);
            let pad_y = ((box_h as f32 * 0.55) as u32).max(6);

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

            let crop =
                image::imageops::crop_imm(img, orig_bx0, orig_by0, crop_w, crop_h).to_image();

            lines.push(DetectedTextLine {
                crop,
                box_rect: (orig_bx0, orig_by0, orig_bx1, orig_by1),
                line_index: idx,
            });
        }

        Ok(lines)
    }

    /// Identifies text line bounding boxes from DBNet probability map using 4-way connected components.
    fn find_boxes_from_prob_map(
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

        Self::merge_word_boxes_into_lines(boxes)
    }

    /// Merges fragmented word-level boxes on the same horizontal text line into unified line boxes.
    fn merge_word_boxes_into_lines(
        mut boxes: Vec<(u32, u32, u32, u32)>,
    ) -> Vec<(u32, u32, u32, u32)> {
        if boxes.len() <= 1 {
            return boxes;
        }

        let mut merged = true;
        while merged {
            merged = false;
            let mut new_boxes: Vec<(u32, u32, u32, u32)> = Vec::new();
            let mut used = vec![false; boxes.len()];

            for i in 0..boxes.len() {
                if used[i] {
                    continue;
                }
                let mut curr = boxes[i];
                used[i] = true;

                for j in (i + 1)..boxes.len() {
                    if used[j] {
                        continue;
                    }
                    let other = boxes[j];

                    let curr_h = curr.3 - curr.1 + 1;
                    let other_h = other.3 - other.1 + 1;
                    let min_h = curr_h.min(other_h);
                    let max_h = curr_h.max(other_h);

                    let overlap_y0 = curr.1.max(other.1);
                    let overlap_y1 = curr.3.min(other.3);
                    let overlap_y = if overlap_y1 >= overlap_y0 {
                        overlap_y1 - overlap_y0 + 1
                    } else {
                        0
                    };

                    let has_vertical_overlap = (overlap_y as f32) >= (min_h as f32 * 0.35);

                    let gap_x = if curr.2 < other.0 {
                        other.0 - curr.2
                    } else if other.2 < curr.0 {
                        curr.0 - other.2
                    } else {
                        0
                    };

                    // Merge words on the same horizontal line (gap <= 3x line height)
                    if has_vertical_overlap && gap_x <= (max_h * 3).max(20) {
                        curr = (
                            curr.0.min(other.0),
                            curr.1.min(other.1),
                            curr.2.max(other.2),
                            curr.3.max(other.3),
                        );
                        used[j] = true;
                        merged = true;
                    }
                }
                new_boxes.push(curr);
            }
            boxes = new_boxes;
        }

        // Sort lines top-to-bottom, left-to-right
        boxes.sort_by(|a, b| {
            let a_center_y = (a.1 + a.3) / 2;
            let b_center_y = (b.1 + b.3) / 2;
            let a_h = a.3 - a.1 + 1;
            let b_h = b.3 - b.1 + 1;
            let min_h = a_h.min(b_h);

            if (a_center_y as i32 - b_center_y as i32).abs() < (min_h as i32 / 2).max(6) {
                a.0.cmp(&b.0)
            } else {
                a.1.cmp(&b.1)
            }
        });

        boxes
    }
}
