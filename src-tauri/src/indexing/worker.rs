use rusqlite::Connection;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::db::connection::Database;
use crate::db::jobs::{self, IndexJobRecord, JOB_TYPE_DELETE, JOB_TYPE_UPSERT};
use crate::db::screenshots;
use crate::errors::AppError;
use crate::filesystem::fingerprint::compute_sha256;
use crate::filesystem::metadata::DiscoveredFileMetadata;
use crate::ocr::engine::OcrEngine;
use crate::ocr::normalize::normalize_ocr_text;

/// Processes a single claimed `UPSERT_SCREENSHOT` job.
/// Follows strict reliability & atomic search consistency invariants:
/// 1. Verifies file exists on disk.
/// 2. Hashes file outside DB lock.
/// 3. If file changed, immediately purges old FTS record to prevent stale search hits.
/// 4. Runs local OCR outside DB lock.
/// 5. Commits OCR success + FTS sync atomically.
fn process_upsert_job(
    conn: &Connection,
    engine: &dyn OcrEngine,
    job: &IndexJobRecord,
) -> Result<i64, AppError> {
    let path_obj = Path::new(&job.path);
    if !path_obj.exists() {
        return Err(AppError::file_not_found(format!(
            "Screenshot file does not exist on disk: {}",
            job.path
        )));
    }

    // 1. Read fresh filesystem metadata & compute hash outside DB lock
    let meta = DiscoveredFileMetadata::from_path(path_obj)
        .map_err(|e| AppError::unknown(format!("Failed to read metadata for {}: {e}", job.path)))?;
    let content_hash = compute_sha256(path_obj)
        .map_err(|e| AppError::unknown(format!("Failed to compute hash for {}: {e}", job.path)))?;

    // 2. Query existing screenshot record
    let existing_opt = screenshots::get_screenshot_by_path(conn, &job.path)?;

    let (screenshot_id, needs_ocr) = match existing_opt {
        Some(existing) => {
            if existing.file_size == meta.file_size
                && existing.modified_at_fs == meta.modified_at_fs
                && existing.content_hash.as_deref() == Some(&content_hash)
            {
                // File is completely unchanged
                if existing.ocr_status == "SUCCEEDED" {
                    // Ensure FTS has this record
                    if let Some(ref text) = existing.ocr_text {
                        let search_text = crate::search::normalize::normalize_search_text(text);
                        let _ = conn.execute(
                            "INSERT OR REPLACE INTO screenshots_fts (rowid, filename, ocr_search_text)
                             VALUES (?1, ?2, ?3)",
                            rusqlite::params![existing.id, existing.filename, search_text],
                        );
                    }
                    (existing.id, false)
                } else {
                    (existing.id, true)
                }
            } else {
                // File was modified! Invalidate old OCR and purge old FTS entry immediately
                screenshots::update_screenshot(
                    conn,
                    existing.id,
                    meta.file_size,
                    &meta.modified_at_fs,
                    &content_hash,
                )?;
                (existing.id, true)
            }
        }
        None => {
            // New screenshot: insert with ocr_status = 'PENDING'
            let new_id = screenshots::insert_screenshot(
                conn,
                job.folder_id,
                &job.path,
                &meta.filename,
                &meta.extension,
                meta.file_size,
                &meta.modified_at_fs,
                &content_hash,
            )?;
            (new_id, true)
        }
    };

    if !needs_ocr {
        return Ok(screenshot_id);
    }

    // 3. Perform OCR outside DB lock
    let (ocr_text, ocr_engine_name) = match engine.recognize(path_obj) {
        Ok(res) => (normalize_ocr_text(&res.text), res.engine),
        Err(e) => {
            let _ = screenshots::mark_ocr_failed(conn, screenshot_id, engine.name());
            return Err(e);
        }
    };

    // 4. Atomically commit OCR success + FTS synchronization
    screenshots::save_ocr_success(conn, screenshot_id, &ocr_text, &ocr_engine_name)?;

    let search_text = crate::search::normalize::normalize_search_text(&ocr_text);
    conn.execute(
        "INSERT OR REPLACE INTO screenshots_fts (rowid, filename, ocr_search_text)
         VALUES (?1, ?2, ?3)",
        rusqlite::params![screenshot_id, meta.filename, search_text],
    )
    .map_err(|e| {
        AppError::database(format!(
            "Failed to sync FTS for screenshot {screenshot_id}: {e}"
        ))
    })?;

    Ok(screenshot_id)
}

/// Processes a single claimed `DELETE_SCREENSHOT` job.
/// Verifies genuine deletion before removing DB record and FTS entry.
fn process_delete_job(conn: &Connection, job: &IndexJobRecord) -> Result<(), AppError> {
    let path_obj = Path::new(&job.path);

    // Safeguard: Check that file is genuinely NotFound on disk
    match fs::metadata(path_obj) {
        Ok(_) => {
            // File still exists on disk! Do not delete DB record
            log::info!(
                "Delete job skipped because file still exists on disk: {}",
                job.path
            );
            return Ok(());
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            // Genuine deletion: delete screenshot record (cascades to screenshots_fts)
            if let Some(existing) = screenshots::get_screenshot_by_path(conn, &job.path)? {
                screenshots::delete_screenshot(conn, existing.id)?;
                log::info!(
                    "Removed deleted screenshot record {}: {}",
                    existing.id,
                    job.path
                );
            }
            Ok(())
        }
        Err(e) => {
            // PermissionDenied or device I/O error -> recoverable retry
            Err(AppError::unknown(format!(
                "Cannot verify file deletion for {}: {e}",
                job.path
            )))
        }
    }
}

/// Executes a single index job step against the provided database connection and OCR engine.
pub fn run_indexing_worker_loop_step(
    conn: &Connection,
    engine: &dyn OcrEngine,
    job: &IndexJobRecord,
) -> Result<Option<i64>, AppError> {
    match job.job_type.as_str() {
        JOB_TYPE_UPSERT => process_upsert_job(conn, engine, job).map(Some),
        JOB_TYPE_DELETE => process_delete_job(conn, job).map(|_| None),
        unknown => Err(AppError::unknown(format!(
            "Unknown index job type: {unknown}"
        ))),
    }
}

/// Runs the background indexing worker loop with single-flight OCR concurrency.
pub fn run_indexing_worker_loop(
    db: Database,
    engine: Arc<dyn OcrEngine>,
    is_paused: Arc<AtomicBool>,
    stop_flag: Arc<AtomicBool>,
    on_job_completed: Option<Arc<dyn Fn(i64, &str) + Send + Sync>>,
) {
    log::info!("Background indexing worker started");

    // Startup: recover any stale leases from previous app crash
    if let Ok(conn) = db.conn.lock() {
        let _ = jobs::recover_stale_leases(&conn);
        let _ = jobs::cleanup_completed_jobs(&conn, 24);
    }

    while !stop_flag.load(Ordering::SeqCst) {
        // If user paused indexing, sleep and check again
        if is_paused.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(300));
            continue;
        }

        // 1. Atomically claim next job with a 60-second lease
        let claimed_opt = {
            match db.conn.lock() {
                Ok(conn) => match jobs::claim_next_job(&conn, 60) {
                    Ok(job) => job,
                    Err(e) => {
                        log::warn!("Failed to claim next index job: {e}");
                        None
                    }
                },
                Err(e) => {
                    log::warn!("Database lock acquisition failed in indexing worker: {e}");
                    None
                }
            }
        };

        let job = match claimed_opt {
            Some(j) => j,
            None => {
                // Queue is empty or all jobs are future-scheduled: sleep briefly
                thread::sleep(Duration::from_millis(250));
                continue;
            }
        };

        // 2. Process the claimed job
        log::debug!(
            "Worker claimed job {} (type={}, path={})",
            job.id,
            job.job_type,
            job.path
        );

        let process_result = {
            let conn_guard = db.conn.lock();
            match conn_guard {
                Ok(conn) => run_indexing_worker_loop_step(&conn, engine.as_ref(), &job),
                Err(e) => Err(AppError::database(format!("Failed to lock DB: {e}"))),
            }
        };

        // 3. Update job status in database based on result
        if let Ok(conn) = db.conn.lock() {
            match process_result {
                Ok(screenshot_id) => {
                    if let Err(e) = jobs::complete_job(&conn, job.id, screenshot_id) {
                        log::warn!("Failed to mark job {} SUCCEEDED: {e}", job.id);
                    } else {
                        log::debug!("Job {} completed successfully", job.id);
                        if let Some(ref cb) = on_job_completed {
                            cb(job.id, &job.path);
                        }
                    }
                }
                Err(err) => {
                    log::warn!("Job {} failed with error: {err}", job.id);

                    let error_code_str = format!("{:?}", err.code);
                    let is_recoverable = match err.code {
                        crate::errors::ErrorCode::DatabaseFailed
                        | crate::errors::ErrorCode::DatabaseMigrationFailed
                        | crate::errors::ErrorCode::Unknown => true,
                        crate::errors::ErrorCode::FileNotFound => false,
                        _ => true,
                    };

                    if is_recoverable {
                        let backoff = (1 << job.attempts.min(5)).min(60);
                        let _ = jobs::retry_or_fail_job(
                            &conn,
                            job.id,
                            &error_code_str,
                            &err.message,
                            backoff,
                        );
                    } else {
                        let _ = jobs::fail_job(&conn, job.id, &error_code_str, &err.message);
                    }
                }
            }
        }
    }

    log::info!("Background indexing worker stopped");
}
