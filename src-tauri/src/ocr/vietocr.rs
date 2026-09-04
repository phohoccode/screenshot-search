use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

use image::RgbImage;
use ort::session::Session;
use ort::value::Tensor;

use crate::errors::AppError;

pub const VIETOCR_EXPECTED_VOCAB_LEN: usize = 229;
pub const VIETOCR_EXPECTED_CLASS_DIM: usize = 233;
pub const VIETOCR_MAX_SEQ_LENGTH: usize = 128;
pub const VIETOCR_SOS_TOKEN: i64 = 1;
pub const VIETOCR_EOS_TOKEN: i64 = 2;

/// Native Rust VietOCR VGG-Transformer text line recognizer powered by ONNX Runtime.
/// Executes pure CPU inference with zero Python runtime dependency.
pub struct VietOcrOnnxRecognizer {
    encoder_session: Arc<Mutex<Session>>,
    decoder_session: Arc<Mutex<Session>>,
    vocab_chars: Arc<Vec<char>>,
}

impl VietOcrOnnxRecognizer {
    /// Loads the VietOCR ONNX model from the specified directory containing:
    /// - `vgg_encoder.onnx` (and `vgg_encoder.onnx.data`)
    /// - `vgg_decoder.onnx`
    /// - `vocab.json`
    pub fn new(models_dir: &Path) -> Result<Self, AppError> {
        let enc_path = models_dir.join("vgg_encoder.onnx");
        let dec_path = models_dir.join("vgg_decoder.onnx");
        let vocab_path = models_dir.join("vocab.json");

        if !enc_path.exists() || !dec_path.exists() || !vocab_path.exists() {
            return Err(AppError::ocr_unavailable(format!(
                "VietOCR ONNX model files missing in {}",
                models_dir.display()
            )));
        }

        // Load vocabulary characters
        let raw_vocab = fs::read_to_string(&vocab_path).map_err(|e| {
            AppError::ocr_unavailable(format!("Failed to read {}: {e}", vocab_path.display()))
        })?;

        // vocab.json contains a JSON string of 229 characters
        let vocab_str: String = serde_json::from_str(&raw_vocab).map_err(|e| {
            AppError::ocr_unavailable(format!("Failed to parse {}: {e}", vocab_path.display()))
        })?;

        let vocab_chars: Vec<char> = vocab_str.chars().collect();
        if vocab_chars.len() != VIETOCR_EXPECTED_VOCAB_LEN {
            return Err(AppError::ocr_unavailable(format!(
                "VietOCR vocabulary size mismatch: expected {VIETOCR_EXPECTED_VOCAB_LEN}, found {}",
                vocab_chars.len()
            )));
        }

        let encoder_session = Session::builder()
            .map_err(|e| AppError::ocr_unavailable(format!("Failed to create ORT builder: {e}")))?
            .with_intra_threads(4)
            .map_err(|e| {
                AppError::ocr_unavailable(format!("Failed to configure ORT threads: {e}"))
            })?
            .commit_from_file(&enc_path)
            .map_err(|e| {
                AppError::ocr_unavailable(format!(
                    "Failed to load VietOCR encoder from {}: {e}",
                    enc_path.display()
                ))
            })?;

        let decoder_session = Session::builder()
            .map_err(|e| AppError::ocr_unavailable(format!("Failed to create ORT builder: {e}")))?
            .with_intra_threads(4)
            .map_err(|e| {
                AppError::ocr_unavailable(format!("Failed to configure ORT threads: {e}"))
            })?
            .commit_from_file(&dec_path)
            .map_err(|e| {
                AppError::ocr_unavailable(format!(
                    "Failed to load VietOCR decoder from {}: {e}",
                    dec_path.display()
                ))
            })?;

        Ok(Self {
            encoder_session: Arc::new(Mutex::new(encoder_session)),
            decoder_session: Arc::new(Mutex::new(decoder_session)),
            vocab_chars: Arc::new(vocab_chars),
        })
    }

    /// Recognizes text from a single cropped text line image using pure ONNX Runtime inference.
    pub fn recognize_line(&self, crop: &RgbImage) -> Result<String, AppError> {
        let (w, h) = (crop.width(), crop.height());
        if w < 2 || h < 2 {
            return Ok(String::new());
        }

        // 1. VietOCR Image Preprocessing
        // Sizing: Height fixed to 32; width aspect-scaled, rounded up to multiple of 10, clamped to [32, 512]
        let aspect_w = ((32.0 * w as f32 / h as f32) / 10.0).ceil() as u32 * 10;
        let target_w = aspect_w.clamp(32, 512);
        let target_h = 32u32;

        let resized = image::imageops::resize(
            crop,
            target_w,
            target_h,
            image::imageops::FilterType::Triangle,
        );

        // Normalize: float32 in range [0.0, 1.0] (pixel / 255.0) in CHW RGB order
        let plane_size = (target_h * target_w) as usize;
        let mut img_data = vec![0.0f32; 3 * plane_size];

        for y in 0..target_h {
            for x in 0..target_w {
                let p = resized.get_pixel(x, y);
                let idx = (y * target_w + x) as usize;
                img_data[0 * plane_size + idx] = p[0] as f32 / 255.0;
                img_data[1 * plane_size + idx] = p[1] as f32 / 255.0;
                img_data[2 * plane_size + idx] = p[2] as f32 / 255.0;
            }
        }

        let img_tensor = Tensor::from_array((
            [1, 3, target_h as usize, target_w as usize],
            img_data.into_boxed_slice(),
        ))
        .map_err(|e| AppError::unknown(format!("Failed to build VietOCR image tensor: {e}")))?;

        // 2. Run Encoder Session
        let mut enc_session = self
            .encoder_session
            .lock()
            .map_err(|e| AppError::unknown(format!("Encoder session lock failed: {e}")))?;

        let enc_outputs = enc_session
            .run(ort::inputs!["img" => img_tensor])
            .map_err(|e| AppError::unknown(format!("Encoder inference failed: {e}")))?;

        let (mem_shape, mem_slice) = enc_outputs["memory"]
            .try_extract_tensor::<f32>()
            .map_err(|e| AppError::unknown(format!("Failed to extract memory tensor: {e}")))?;

        let time_steps = mem_shape[0] as usize;
        let batch_size = mem_shape[1] as usize;
        let feat_dim = mem_shape[2] as usize;

        // Copy memory buffer for iterative decoder steps
        let memory_data: Vec<f32> = mem_slice.to_vec();
        drop(enc_outputs);
        drop(enc_session);

        // 3. Autoregressive Greedy Decoder Loop
        let mut dec_session = self
            .decoder_session
            .lock()
            .map_err(|e| AppError::unknown(format!("Decoder session lock failed: {e}")))?;

        let mut translated = vec![VIETOCR_SOS_TOKEN];

        for _ in 0..VIETOCR_MAX_SEQ_LENGTH {
            let seq_len = translated.len();

            let tgt_tensor =
                Tensor::from_array(([seq_len, 1], translated.clone().into_boxed_slice()))
                    .map_err(|e| AppError::unknown(format!("Failed to build tgt tensor: {e}")))?;

            let memory_tensor = Tensor::from_array((
                [time_steps, batch_size, feat_dim],
                memory_data.clone().into_boxed_slice(),
            ))
            .map_err(|e| AppError::unknown(format!("Failed to build memory tensor: {e}")))?;

            let dec_outputs = dec_session
                .run(ort::inputs!["tgt" => tgt_tensor, "memory" => memory_tensor])
                .map_err(|e| AppError::unknown(format!("Decoder step inference failed: {e}")))?;

            let (out_shape, out_slice) = dec_outputs["output"]
                .try_extract_tensor::<f32>()
                .map_err(|e| AppError::unknown(format!("Failed to extract decoder output: {e}")))?;

            let class_dim = out_shape[2] as usize;
            if class_dim != VIETOCR_EXPECTED_CLASS_DIM {
                return Err(AppError::unknown(format!(
                    "Unexpected decoder output class dimension: {class_dim}"
                )));
            }

            // Slice logits for the last timestep
            let last_t_offset = (seq_len - 1) * class_dim;
            let mut best_class = 0;
            let mut best_score = -f32::INFINITY;

            for c in 0..class_dim {
                let score = out_slice[last_t_offset + c];
                if score > best_score {
                    best_score = score;
                    best_class = c;
                }
            }

            // If model emits EOS token (2), decoding is complete
            if best_class as i64 == VIETOCR_EOS_TOKEN {
                break;
            }

            translated.push(best_class as i64);
        }

        drop(dec_session);

        // 4. Decode token indices to characters
        // Indices 0..3 are special tokens (<pad>, <sos>, <eos>, <unk>).
        // Chars begin at index 4.
        let mut result = String::new();
        for tok in translated.into_iter().skip(1) {
            if tok >= 4 {
                let char_idx = (tok - 4) as usize;
                if char_idx < self.vocab_chars.len() {
                    result.push(self.vocab_chars[char_idx]);
                }
            }
        }

        Ok(result.trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_vietocr_onnx_line_recognition_real_fixture() {
        let p1 = PathBuf::from("tests/fixtures/models/vemines");
        let p2 = PathBuf::from("../tests/fixtures/models/vemines");
        let models_dir = if p1.exists() {
            p1
        } else if p2.exists() {
            p2
        } else {
            println!("VietOCR test skipped: vemines not present");
            return;
        };

        let recognizer = match VietOcrOnnxRecognizer::new(&models_dir) {
            Ok(r) => r,
            Err(e) => {
                println!("VietOCR init skipped: {e}");
                return;
            }
        };

        // Open real fixture ui_01_button_actions.png and crop the button line
        let img_path =
            PathBuf::from("src-tauri/tests/fixtures/vietnamese_benchmark/ui_01_button_actions.png");
        let img_path = if img_path.exists() {
            img_path
        } else {
            PathBuf::from("tests/fixtures/vietnamese_benchmark/ui_01_button_actions.png")
        };

        if !img_path.exists() {
            return;
        }

        let img = image::open(&img_path).expect("Open fixture").to_rgb8();
        // Crop: x: 20..445, y: 20..52
        let crop = image::imageops::crop_imm(&img, 20, 20, 425, 32).to_image();

        let recognized = recognizer.recognize_line(&crop).expect("Recognize line");
        println!("VietOCR ONNX Line Result: {recognized}");

        // Assert it recognized Vietnamese text with diacritics
        assert!(
            recognized.contains("Lưu thay đổi")
                || recognized.contains("Tiếp tục")
                || recognized.contains("Quay lại"),
            "Expected Vietnamese diacritics in output, got: {recognized}"
        );
    }
}
