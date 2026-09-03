use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};

use crate::db::connection::Database;
use crate::db::screenshots::{self, OcrStats};
use crate::errors::{AppError, CommandResult};
use crate::ocr::engine::{OcrEngine, OcrEngineInfo};
use crate::ocr::orchestrator::{run_ocr_batch, OcrBatchSummary, OcrManager};
use crate::ocr::windows::WindowsMediaOcrEngine;

/// Event payload emitted to the frontend during OCR indexing.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OcrProgressPayload {
    pub total: usize,
    pub processed: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub is_running: bool,
}

/// Starts local OCR indexing for pending screenshots.
/// Protected by an atomic single-flight CAS guard and RAII cleanup.
#[tauri::command]
pub async fn start_ocr_indexing(
    app: AppHandle,
    db: State<'_, Database>,
    ocr_mgr: State<'_, OcrManager>,
    folder_id: Option<i64>,
    limit: Option<usize>,
) -> CommandResult<OcrBatchSummary> {
    // Acquire single-flight lock; fails immediately if another batch is active
    let guard = ocr_mgr
        .acquire_running_guard()
        .ok_or_else(|| AppError::ocr("An OCR indexing job is already running in the background"))?;

    ocr_mgr.cancel_flag.store(false, Ordering::SeqCst);

    let cancel_flag = ocr_mgr.cancel_flag.clone();
    let app_clone = app.clone();
    let db_clone = db.inner().clone();

    // Spawn execution off the async thread with RAII guard moved into the worker
    let result = tauri::async_runtime::spawn_blocking(move || {
        let _running_guard = guard;

        #[cfg(target_os = "windows")]
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }

        let engine = WindowsMediaOcrEngine::new();

        let summary = run_ocr_batch(
            &db_clone,
            &engine,
            folder_id,
            limit,
            cancel_flag,
            Some(&|total, processed, succeeded, failed| {
                let _ = app_clone.emit(
                    "ocr_progress",
                    OcrProgressPayload {
                        total,
                        processed,
                        succeeded,
                        failed,
                        is_running: true,
                    },
                );
            }),
        );

        // Emit final completed progress state
        let _ = app_clone.emit(
            "ocr_progress",
            OcrProgressPayload {
                total: summary.as_ref().map(|s| s.total_candidates).unwrap_or(0),
                processed: summary.as_ref().map(|s| s.processed).unwrap_or(0),
                succeeded: summary.as_ref().map(|s| s.succeeded).unwrap_or(0),
                failed: summary.as_ref().map(|s| s.failed).unwrap_or(0),
                is_running: false,
            },
        );

        #[cfg(target_os = "windows")]
        unsafe {
            windows::Win32::System::Com::CoUninitialize();
        }

        summary
    })
    .await
    .map_err(|e| AppError::ocr(format!("OCR worker task panicked: {e}")))?;

    result
}

/// Retrieves aggregate OCR status statistics across all screenshots.
#[tauri::command]
pub fn get_ocr_stats(db: State<'_, Database>) -> CommandResult<OcrStats> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    screenshots::get_ocr_stats(&conn)
}

/// Retrieves active OCR engine diagnostics, language packs, and dimensions.
#[tauri::command]
pub fn get_ocr_engine_info() -> CommandResult<OcrEngineInfo> {
    let engine = WindowsMediaOcrEngine::new();
    Ok(engine.get_info())
}

/// Requests graceful cancellation of the ongoing OCR indexing batch.
#[tauri::command]
pub fn cancel_ocr_indexing(ocr_mgr: State<'_, OcrManager>) -> CommandResult<bool> {
    ocr_mgr.request_cancel();
    Ok(true)
}

/// Retrieves high-level status of the automatic background indexing service.
#[tauri::command]
pub fn get_indexing_status(
    service: State<'_, std::sync::Arc<crate::indexing::service::IndexingService>>,
) -> CommandResult<crate::indexing::service::IndexingServiceStatus> {
    service.get_status()
}

/// Pauses automatic background indexing.
#[tauri::command]
pub fn pause_indexing(
    service: State<'_, std::sync::Arc<crate::indexing::service::IndexingService>>,
) -> CommandResult<()> {
    service.pause();
    Ok(())
}

/// Resumes automatic background indexing.
#[tauri::command]
pub fn resume_indexing(
    service: State<'_, std::sync::Arc<crate::indexing::service::IndexingService>>,
) -> CommandResult<()> {
    service.resume();
    Ok(())
}

/// Retries all failed index jobs.
#[tauri::command]
pub fn retry_failed_index_jobs(
    service: State<'_, std::sync::Arc<crate::indexing::service::IndexingService>>,
) -> CommandResult<usize> {
    service.retry_failed()
}
