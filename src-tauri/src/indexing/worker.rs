use rusqlite::Connection;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::db::connection::Database;
use crate::db::jobs::{
    self, IndexJobRecord, JOB_TYPE_DELETE, JOB_TYPE_EMBEDDING, JOB_TYPE_RE_OCR, JOB_TYPE_UPSERT,
};
use crate::db::screenshots;
use crate::errors::AppError;
use crate::filesystem::fingerprint::compute_sha256;
use crate::filesystem::metadata::DiscoveredFileMetadata;
use crate::ocr::engine::OcrEngine;
use crate::ocr::normalize::normalize_ocr_text;
use crate::semantic::SemanticModelManager;

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
    semantic_mgr: Option<&SemanticModelManager>,
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
                let _ = crate::db::embeddings::delete_embedding(conn, existing.id);
                (existing.id, true)
            }
        }
        None => {
            // Check if this is a rename/move of an existing screenshot whose file on disk no longer exists
            let renamed_candidate =
                screenshots::get_screenshot_by_hash(conn, job.folder_id, &content_hash)
                    .ok()
                    .flatten()
                    .filter(|candidate| !Path::new(&candidate.path).exists());

            if let Some(existing) = renamed_candidate {
                // Migrate the existing screenshot record to the new path in-place
                screenshots::rename_screenshot(conn, job.folder_id, &existing.path, &job.path)?;
                let needs_ocr = existing.ocr_status != "SUCCEEDED";
                (existing.id, needs_ocr)
            } else {
                // Genuinely new screenshot: insert with ocr_status = 'PENDING'
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
        }
    };

    if !needs_ocr {
        return Ok(screenshot_id);
    }

    // 3. Perform OCR outside DB lock
    let ocr_res = match engine.recognize(path_obj) {
        Ok(res) => res,
        Err(e) => {
            let _ = screenshots::mark_ocr_failed(conn, screenshot_id, engine.name());
            return Err(e);
        }
    };

    let ocr_text = normalize_ocr_text(&ocr_res.text);
    let pipeline_version = format!("{}:{}", ocr_res.engine, ocr_res.engine_version);

    // 4. Atomically commit OCR success + FTS synchronization with engine metadata
    screenshots::save_ocr_success_with_metadata(
        conn,
        screenshot_id,
        &ocr_text,
        &ocr_res.engine,
        Some(&ocr_res.engine_version),
        ocr_res.language.as_deref(),
        Some(&pipeline_version),
    )?;

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

    // 5. If semantic model is available, enqueue GENERATE_TEXT_EMBEDDING
    if let Some(mgr) = semantic_mgr {
        if mgr.get_engine().is_some() {
            let dedupe_key = jobs::build_embedding_dedupe_key(
                screenshot_id,
                &content_hash,
                crate::semantic::DEFAULT_MODEL_VERSION,
            );
            let _ = jobs::enqueue_embedding_job(
                conn,
                job.folder_id,
                screenshot_id,
                &job.path,
                &dedupe_key,
            );
        }
    }

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
            // Genuine deletion: delete screenshot record (cascades to screenshots_fts & screenshot_embeddings)
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

/// Processes a single claimed `GENERATE_TEXT_EMBEDDING` job.
/// Generates semantic vector and persists to screenshot_embeddings table.
fn process_embedding_job(
    conn: &Connection,
    semantic_mgr: Option<&SemanticModelManager>,
    job: &IndexJobRecord,
) -> Result<Option<i64>, AppError> {
    let mgr = semantic_mgr.ok_or_else(|| {
        AppError::unknown("Semantic model manager not configured in indexing worker")
    })?;

    let engine = mgr.get_engine().ok_or_else(|| {
        AppError::unknown("Semantic model is not currently installed or available")
    })?;

    // Determine screenshot ID: either job.screenshot_id or look up by path
    let screenshot_id = match job.screenshot_id {
        Some(id) => id,
        None => {
            let s = screenshots::get_screenshot_by_path(conn, &job.path)?.ok_or_else(|| {
                AppError::file_not_found(format!(
                    "Screenshot not found for embedding: {}",
                    job.path
                ))
            })?;
            s.id
        }
    };

    let detail = screenshots::get_screenshot_by_id(conn, screenshot_id)?.ok_or_else(|| {
        AppError::file_not_found(format!("Screenshot ID {screenshot_id} not found"))
    })?;

    if detail.ocr_status != "SUCCEEDED" {
        log::debug!("Skipping embedding for screenshot {screenshot_id}: OCR not SUCCEEDED");
        return Ok(Some(screenshot_id));
    }

    let ocr_text = detail.ocr_text.unwrap_or_default();
    let doc_text = crate::semantic::format_semantic_document(&detail.filename, &ocr_text, None);

    // 1. Run inference outside DB lock
    let vector = engine.embed_passage(&doc_text)?;

    // 2. Persist vector in short SQLite write
    crate::db::embeddings::save_embedding(
        conn,
        screenshot_id,
        engine.model_id(),
        engine.model_version(),
        &vector,
    )?;

    log::debug!("Generated semantic embedding for screenshot {screenshot_id}");
    Ok(Some(screenshot_id))
}

/// Executes a single index job step against the provided database connection and OCR engine.
/// Backward-compatible wrapper for testing.
pub fn run_indexing_worker_loop_step(
    conn: &Connection,
    engine: &dyn OcrEngine,
    job: &IndexJobRecord,
) -> Result<Option<i64>, AppError> {
    run_indexing_worker_loop_step_with_semantic(conn, engine, None, job)
}

/// Processes a single claimed `RE_OCR_SCREENSHOT` job.
/// Upgrades OCR and FTS while safely preserving existing data on failure and invalidating stale embeddings on success.
fn process_re_ocr_job(
    conn: &Connection,
    engine: &dyn OcrEngine,
    semantic_mgr: Option<&SemanticModelManager>,
    job: &IndexJobRecord,
) -> Result<Option<i64>, AppError> {
    let path_obj = Path::new(&job.path);
    if !path_obj.exists() {
        return Err(AppError::file_not_found(format!(
            "Screenshot file does not exist on disk for re-OCR: {}",
            job.path
        )));
    }

    let screenshot_id = match job.screenshot_id {
        Some(id) => id,
        None => {
            let s = screenshots::get_screenshot_by_path(conn, &job.path)?.ok_or_else(|| {
                AppError::file_not_found(format!("Screenshot not found for re-OCR: {}", job.path))
            })?;
            s.id
        }
    };

    let detail = screenshots::get_screenshot_by_id(conn, screenshot_id)?.ok_or_else(|| {
        AppError::file_not_found(format!("Screenshot ID {screenshot_id} not found"))
    })?;

    // A durable re-OCR job may survive an app restart while the requested local
    // model is still loading asynchronously. Do not silently downgrade that job
    // to the router's temporary Windows fallback. Preserve the existing OCR/FTS
    // and let the normal recoverable retry path wait for the requested pipeline.
    let target_pipeline = re_ocr_target_pipeline(&job.dedupe_key);
    if let Some(target) = target_pipeline {
        let info = engine.get_info();
        let active_pipeline = format!("{}:{}", info.engine_name, info.engine_version);
        if active_pipeline != target {
            return Err(AppError::ocr_unavailable(format!(
                "Re-OCR target pipeline {target} is not ready; active pipeline is {active_pipeline}"
            )));
        }
    }

    // 1. Run new OCR outside DB lock
    let ocr_res = match engine.recognize(path_obj) {
        Ok(res) => res,
        Err(e) => {
            // Failure preservation invariant: Keep previous successful OCR and FTS data intact
            log::warn!(
                "Re-OCR failed for screenshot {screenshot_id} ({}): {e}. Preserving existing OCR data.",
                job.path
            );
            return Err(e);
        }
    };

    let ocr_text = normalize_ocr_text(&ocr_res.text);
    let pipeline_version = format!("{}:{}", ocr_res.engine, ocr_res.engine_version);

    if let Some(target) = target_pipeline {
        if pipeline_version != target {
            return Err(AppError::ocr_unavailable(format!(
                "Re-OCR produced pipeline {pipeline_version}, but the durable job requires {target}"
            )));
        }
    }

    // 2. Atomically update OCR text, FTS index, and invalidate stale embedding
    screenshots::replace_ocr_atomically(
        conn,
        screenshot_id,
        &ocr_text,
        &ocr_res.engine,
        Some(&ocr_res.engine_version),
        ocr_res.language.as_deref(),
        &pipeline_version,
    )?;

    // 3. If semantic model is available, enqueue GENERATE_TEXT_EMBEDDING
    if let Some(mgr) = semantic_mgr {
        if mgr.get_engine().is_some() {
            let content_hash = detail.content_hash.unwrap_or_default();
            let dedupe_key = jobs::build_embedding_dedupe_key(
                screenshot_id,
                &content_hash,
                crate::semantic::DEFAULT_MODEL_VERSION,
            );
            let _ = jobs::enqueue_embedding_job(
                conn,
                job.folder_id,
                screenshot_id,
                &job.path,
                &dedupe_key,
            );
        }
    }

    log::info!(
        "Successfully re-processed OCR for screenshot {screenshot_id} using {}",
        ocr_res.engine
    );
    Ok(Some(screenshot_id))
}

fn re_ocr_target_pipeline(dedupe_key: &str) -> Option<&str> {
    let mut parts = dedupe_key.splitn(4, ':');
    if parts.next()? != "RE_OCR" {
        return None;
    }
    parts.next()?;
    parts.next()?;
    let target = parts.next()?;
    (!target.is_empty()).then_some(target)
}

/// Executes a single index job step with OCR and optional semantic model manager.
pub fn run_indexing_worker_loop_step_with_semantic(
    conn: &Connection,
    engine: &dyn OcrEngine,
    semantic_mgr: Option<&SemanticModelManager>,
    job: &IndexJobRecord,
) -> Result<Option<i64>, AppError> {
    match job.job_type.as_str() {
        JOB_TYPE_UPSERT => process_upsert_job(conn, engine, semantic_mgr, job).map(Some),
        JOB_TYPE_DELETE => process_delete_job(conn, job).map(|_| None),
        JOB_TYPE_EMBEDDING => process_embedding_job(conn, semantic_mgr, job),
        JOB_TYPE_RE_OCR => process_re_ocr_job(conn, engine, semantic_mgr, job),
        unknown => Err(AppError::unknown(format!(
            "Unknown index job type: {unknown}"
        ))),
    }
}

/// Runs the background indexing worker loop with single-flight OCR concurrency.
pub fn run_indexing_worker_loop(
    db: Database,
    engine: Arc<dyn OcrEngine>,
    semantic_mgr: Option<Arc<SemanticModelManager>>,
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
                Ok(conn) => run_indexing_worker_loop_step_with_semantic(
                    &conn,
                    engine.as_ref(),
                    semantic_mgr.as_deref(),
                    &job,
                ),
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

#[cfg(test)]
mod tests {
    use super::re_ocr_target_pipeline;

    #[test]
    fn durable_re_ocr_key_preserves_full_target_pipeline() {
        assert_eq!(
            re_ocr_target_pipeline("RE_OCR:42:abc123:hybrid_windows_vietocr:hybrid_v2"),
            Some("hybrid_windows_vietocr:hybrid_v2")
        );
        assert_eq!(
            re_ocr_target_pipeline("re_ocr:42:legacy"),
            None,
            "legacy test keys must remain backward compatible"
        );
    }
}
