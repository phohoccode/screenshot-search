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

/// Retrieves combined diagnostic information about Windows OCR languages and multilingual fallback model.
#[tauri::command]
pub fn get_ocr_engine_diagnostics(
    router: State<'_, std::sync::Arc<crate::ocr::router::OcrEngineRouter>>,
) -> CommandResult<crate::ocr::router::OcrEngineDiagnostics> {
    Ok(router.get_diagnostics())
}

/// Updates the active OCR Engine Router mode (`Auto`, `Windows`, `Multilingual`).
#[tauri::command]
pub fn set_ocr_engine_mode(
    router: State<'_, std::sync::Arc<crate::ocr::router::OcrEngineRouter>>,
    mode: crate::ocr::engine::OcrEngineMode,
) -> CommandResult<()> {
    router.set_mode(mode);
    Ok(())
}

/// Triggers on-demand background download of the local multilingual OCR model.
#[tauri::command]
pub async fn download_multilingual_ocr_model(
    app: AppHandle,
    router: State<'_, std::sync::Arc<crate::ocr::router::OcrEngineRouter>>,
) -> CommandResult<()> {
    let model_mgr = router.get_model_manager();
    let app_clone = app.clone();

    model_mgr.start_download(Some(std::sync::Arc::new(move || {
        let _ = app_clone.emit("ocr_model_status_changed", ());
    })))?;

    Ok(())
}

/// Retrieves aggregate OCR engine diagnostic statistics.
#[tauri::command]
pub fn get_ocr_engine_stats(
    db: State<'_, Database>,
    router: State<'_, std::sync::Arc<crate::ocr::router::OcrEngineRouter>>,
) -> CommandResult<screenshots::OcrEngineStats> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    let diagnostics = router.get_diagnostics();
    let target_pipeline = if diagnostics.is_multilingual_ready {
        "multilingual_ocr:ppocr_v4"
    } else {
        "windows_media_ocr:winrt_v1"
    };

    screenshots::get_ocr_engine_stats(&conn, target_pipeline)
}

/// Returns the count of screenshots eligible for re-OCR with an improved engine.
#[tauri::command]
pub fn get_re_ocr_eligible_count(
    db: State<'_, Database>,
    router: State<'_, std::sync::Arc<crate::ocr::router::OcrEngineRouter>>,
) -> CommandResult<usize> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    let diagnostics = router.get_diagnostics();
    let target_pipeline = if diagnostics.is_multilingual_ready {
        "multilingual_ocr:ppocr_v4"
    } else {
        "windows_media_ocr:winrt_v1"
    };

    screenshots::get_re_ocr_eligible_count(&conn, target_pipeline)
}

/// Enqueues eligible screenshots for background re-OCR with the improved OCR engine.
#[tauri::command]
pub fn reprocess_screenshots_with_improved_ocr(
    db: State<'_, Database>,
    router: State<'_, std::sync::Arc<crate::ocr::router::OcrEngineRouter>>,
    limit: Option<usize>,
) -> CommandResult<usize> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    let diagnostics = router.get_diagnostics();
    let target_pipeline = match diagnostics.mode {
        crate::ocr::engine::OcrEngineMode::Windows => "windows_media_ocr:winrt_v1".to_string(),
        crate::ocr::engine::OcrEngineMode::Multilingual => "multilingual_ocr:ppocr_v4".to_string(),
        crate::ocr::engine::OcrEngineMode::Auto => {
            if diagnostics.is_multilingual_ready {
                "multilingual_ocr:ppocr_v4".to_string()
            } else {
                "windows_media_ocr:winrt_v1".to_string()
            }
        }
    };

    let eligible = screenshots::get_re_ocr_eligible_screenshots(
        &conn,
        &target_pipeline,
        limit.unwrap_or(10_000),
    )?;
    let mut enqueued = 0;

    for item in eligible {
        let content_hash = screenshots::get_screenshot_by_id(&conn, item.id)?
            .and_then(|d| d.content_hash)
            .unwrap_or_default();

        let dedupe_key =
            crate::db::jobs::build_re_ocr_dedupe_key(item.id, &content_hash, &target_pipeline);
        if let Ok(Some(_)) = crate::db::jobs::enqueue_re_ocr_job(
            &conn,
            item.folder_id,
            item.id,
            &item.path,
            &dedupe_key,
        ) {
            enqueued += 1;
        }
    }

    log::info!("Enqueued {enqueued} screenshots for re-OCR with target pipeline {target_pipeline}");
    Ok(enqueued)
}
