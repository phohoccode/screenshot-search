use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, State};

use crate::db::connection::Database;
use crate::db::screenshots::{self, OcrStats};
use crate::errors::{AppError, CommandResult};
use crate::ocr::engine::{OcrEngine, OcrEngineInfo};
use crate::ocr::orchestrator::{run_ocr_batch, OcrBatchSummary, OcrManager};
use crate::ocr::windows::WindowsMediaOcrEngine;

fn target_pipeline_version(diagnostics: &crate::ocr::router::OcrEngineDiagnostics) -> String {
    let windows_pipeline = || {
        format!(
            "{}:{}",
            diagnostics.windows_info.engine_name, diagnostics.windows_info.engine_version
        )
    };
    let hybrid_pipeline = || {
        format!(
            "hybrid_windows_vietocr:{}",
            diagnostics.multilingual_info.model_version
        )
    };

    match diagnostics.mode {
        crate::ocr::engine::OcrEngineMode::Windows => windows_pipeline(),
        crate::ocr::engine::OcrEngineMode::Multilingual => hybrid_pipeline(),
        crate::ocr::engine::OcrEngineMode::Auto => {
            if diagnostics.windows_supports_vietnamese || !diagnostics.is_multilingual_ready {
                windows_pipeline()
            } else {
                hybrid_pipeline()
            }
        }
    }
}

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
    router: State<'_, std::sync::Arc<crate::ocr::router::OcrEngineRouter>>,
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
    let router_clone = router.inner().clone();

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

        let summary = run_ocr_batch(
            &db_clone,
            router_clone.as_ref(),
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

/// Updates the active OCR Engine Router mode.
#[tauri::command]
pub fn set_ocr_engine_mode(
    router: State<'_, std::sync::Arc<crate::ocr::router::OcrEngineRouter>>,
    mode: crate::ocr::engine::OcrEngineMode,
) -> CommandResult<()> {
    router.try_set_mode(mode)
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
    let target_pipeline = target_pipeline_version(&diagnostics);

    screenshots::get_ocr_engine_stats(&conn, &target_pipeline)
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
    let target_pipeline = target_pipeline_version(&diagnostics);

    screenshots::get_re_ocr_eligible_count(&conn, &target_pipeline)
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
    let target_pipeline = target_pipeline_version(&diagnostics);

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::engine::{OcrEngineInfo, OcrEngineMode};
    use crate::ocr::manager::{MultilingualOcrModelInfo, MultilingualOcrStatus};
    use crate::ocr::router::OcrEngineDiagnostics;

    fn diagnostics(
        mode: OcrEngineMode,
        windows_supports_vietnamese: bool,
        is_multilingual_ready: bool,
    ) -> OcrEngineDiagnostics {
        OcrEngineDiagnostics {
            mode,
            active_engine_name: String::new(),
            windows_info: OcrEngineInfo {
                engine_name: "windows_media_ocr".to_string(),
                engine_version: "winrt_v1".to_string(),
                active_language: "en-US".to_string(),
                available_languages: vec!["en-US".to_string()],
                supports_vietnamese: windows_supports_vietnamese,
                max_image_dimension: 10_000,
            },
            multilingual_info: MultilingualOcrModelInfo {
                model_id: "multilingual-ocr".to_string(),
                model_version: crate::ocr::hybrid::HYBRID_ENGINE_VERSION.to_string(),
                status: if is_multilingual_ready {
                    MultilingualOcrStatus::Ready
                } else {
                    MultilingualOcrStatus::NotInstalled
                },
                is_available: is_multilingual_ready,
                approximate_size_mb: 158,
            },
            windows_supports_vietnamese,
            is_multilingual_ready,
        }
    }

    #[test]
    fn target_pipeline_matches_actual_router_output() {
        let forced_windows = diagnostics(OcrEngineMode::Windows, false, true);
        assert_eq!(
            target_pipeline_version(&forced_windows),
            "windows_media_ocr:winrt_v1"
        );

        let forced_hybrid = diagnostics(OcrEngineMode::Multilingual, false, true);
        assert_eq!(
            target_pipeline_version(&forced_hybrid),
            format!(
                "hybrid_windows_vietocr:{}",
                crate::ocr::hybrid::HYBRID_ENGINE_VERSION
            )
        );

        let hybrid = diagnostics(OcrEngineMode::Auto, false, true);
        assert_eq!(
            target_pipeline_version(&hybrid),
            format!(
                "hybrid_windows_vietocr:{}",
                crate::ocr::hybrid::HYBRID_ENGINE_VERSION
            )
        );

        let native_vietnamese = diagnostics(OcrEngineMode::Auto, true, true);
        assert_eq!(
            target_pipeline_version(&native_vietnamese),
            "windows_media_ocr:winrt_v1"
        );

        let missing_model = diagnostics(OcrEngineMode::Auto, false, false);
        assert_eq!(
            target_pipeline_version(&missing_model),
            "windows_media_ocr:winrt_v1"
        );
    }
}
