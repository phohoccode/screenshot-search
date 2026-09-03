use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use super::engine::OcrEngine;
use super::normalize::normalize_ocr_text;
use crate::db::connection::Database;
use crate::db::screenshots;
use crate::errors::AppError;

/// Summary of an executed OCR indexing batch.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct OcrBatchSummary {
    pub total_candidates: usize,
    pub processed: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub duration_ms: u64,
}

/// Thread-safe manager for tracking background OCR indexing execution and cancellation.
#[derive(Clone)]
pub struct OcrManager {
    pub is_running: Arc<AtomicBool>,
    pub cancel_flag: Arc<AtomicBool>,
}

impl Default for OcrManager {
    fn default() -> Self {
        Self {
            is_running: Arc::new(AtomicBool::new(false)),
            cancel_flag: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl OcrManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request_cancel(&self) {
        self.cancel_flag.store(true, Ordering::SeqCst);
    }

    pub fn is_active(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }
}

/// Executes a batch of pending OCR jobs with bounded concurrency.
///
/// Invariants strictly enforced:
/// 1. Crash recovery: Stale `PROCESSING` jobs are recovered back to `PENDING`.
/// 2. Short transactions: SQLite lock is released during image recognition.
/// 3. Per-file isolation: Single-file OCR failure does not abort the batch.
/// 4. Graceful cancellation: Checks `cancel_flag` between images.
pub fn run_ocr_batch(
    db: &Database,
    engine: &dyn OcrEngine,
    folder_id: Option<i64>,
    limit: Option<usize>,
    cancel_flag: Arc<AtomicBool>,
    on_progress: Option<&dyn Fn(usize, usize, usize, usize)>,
) -> Result<OcrBatchSummary, AppError> {
    let start_time = Instant::now();
    let batch_limit = limit.unwrap_or(500);

    // 1. Crash recovery on startup/init
    {
        let conn = db.conn.lock().map_err(|e| {
            AppError::database(format!("Failed to acquire DB lock for OCR recovery: {e}"))
        })?;
        let _ = screenshots::recover_stale_processing(&conn);
    }

    // 2. Fetch pending items
    let pending_items = {
        let conn = db.conn.lock().map_err(|e| {
            AppError::database(format!("Failed to acquire DB lock to fetch pending: {e}"))
        })?;
        screenshots::get_pending_screenshots(&conn, folder_id, batch_limit)?
    };

    let total_candidates = pending_items.len();
    if total_candidates == 0 {
        return Ok(OcrBatchSummary {
            total_candidates: 0,
            processed: 0,
            succeeded: 0,
            failed: 0,
            duration_ms: start_time.elapsed().as_millis() as u64,
        });
    }

    let mut processed = 0;
    let mut succeeded = 0;
    let mut failed = 0;

    log::info!(
        "Starting OCR batch: {total_candidates} pending screenshots queued (engine: {})",
        engine.name()
    );

    // 3. Process each screenshot sequentially outside long DB locks
    for item in pending_items {
        // Check cancellation
        if cancel_flag.load(Ordering::Relaxed) {
            log::info!("OCR indexing cancelled by user request");
            break;
        }

        // Mark as PROCESSING in SQLite
        {
            let conn = db
                .conn
                .lock()
                .map_err(|e| AppError::database(format!("Failed to acquire DB lock: {e}")))?;
            let _ = screenshots::mark_processing(&conn, item.id);
        }

        // Run OCR recognition (LONG-RUNNING STEP - RUNS WITHOUT DB LOCK)
        let image_path = Path::new(&item.path);
        let recognition_result = engine.recognize(image_path);

        // Update database with result in a short transaction
        {
            let conn = db.conn.lock().map_err(|e| {
                AppError::database(format!("Failed to acquire DB lock to save OCR result: {e}"))
            })?;

            match recognition_result {
                Ok(res) => {
                    let normalized_text = normalize_ocr_text(&res.text);
                    if let Err(e) =
                        screenshots::save_ocr_success(&conn, item.id, &normalized_text, &res.engine)
                    {
                        log::warn!("Failed to save OCR success for {}: {e}", item.path);
                        failed += 1;
                    } else {
                        succeeded += 1;
                    }
                }
                Err(e) => {
                    log::warn!(
                        "OCR recognition failed for {} (id: {}): {e}",
                        item.path,
                        item.id
                    );
                    let _ = screenshots::mark_ocr_failed(&conn, item.id, engine.name());
                    failed += 1;
                }
            }
        }

        processed += 1;

        if let Some(callback) = on_progress {
            callback(total_candidates, processed, succeeded, failed);
        }
    }

    let duration_ms = start_time.elapsed().as_millis() as u64;

    log::info!(
        "OCR batch finished: processed={processed}/{total_candidates}, succeeded={succeeded}, failed={failed}, duration={duration_ms}ms"
    );

    Ok(OcrBatchSummary {
        total_candidates,
        processed,
        succeeded,
        failed,
        duration_ms,
    })
}
