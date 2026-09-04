use std::path::Path;
use tauri::State;

use crate::db::connection::Database;
use crate::db::screenshots::{self, ScreenshotDetail, SearchIndexHealth};
use crate::errors::{AppError, CommandResult};
use crate::search::{self, SearchRequest, SearchResultPage};

/// Executes search against screenshots, preferring hybrid ranking (FTS5 + semantic embeddings)
/// when the local model is installed, and seamlessly falling back to FTS5 keyword search otherwise.
#[tauri::command]
pub fn search_screenshots(
    db: State<'_, Database>,
    service: State<'_, std::sync::Arc<crate::indexing::service::IndexingService>>,
    query: String,
    folder_id: Option<i64>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> CommandResult<SearchResultPage> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    let req = SearchRequest {
        query,
        folder_id,
        limit,
        offset,
    };

    // If query has searchable text and local semantic model is ready, execute two-stage hybrid search
    if !req.query.trim().is_empty() {
        if let Some(engine) = service.semantic_mgr().get_engine() {
            match search::search_hybrid(&conn, engine.as_ref(), &req) {
                Ok(hybrid_page) => return Ok(hybrid_page),
                Err(err) => {
                    log::warn!("Hybrid search encountered error, falling back to FTS5: {err}");
                }
            }
        }
    }

    // Default/fallback to SQLite FTS5 search
    search::search_screenshots(&conn, &req)
}

/// Retrieves complete metadata and OCR text for a single screenshot by ID.
#[tauri::command]
pub fn get_screenshot(db: State<'_, Database>, id: i64) -> CommandResult<ScreenshotDetail> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    screenshots::get_screenshot_by_id(&conn, id)?
        .ok_or_else(|| AppError::file_not_found(format!("Screenshot with ID {id} not found")))
}

/// Securely loads image data for a screenshot that exists in the database.
/// Security boundary: receives strictly `id: i64`, verifies existence in DB,
/// ensures no arbitrary filesystem access, returns base64 data URL.
pub fn get_screenshot_image_data_internal(db: &Database, id: i64) -> CommandResult<String> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    let detail = screenshots::get_screenshot_by_id(&conn, id)?
        .ok_or_else(|| AppError::file_not_found(format!("Screenshot with ID {id} not found")))?;

    let path = Path::new(&detail.path);
    if !path.exists() {
        return Err(AppError::file_not_found(format!(
            "Screenshot file no longer exists at: {}",
            detail.path
        )));
    }

    let bytes = std::fs::read(path)
        .map_err(|e| AppError::unknown(format!("Failed to read screenshot file from disk: {e}")))?;

    let mime = match detail.extension.to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    };

    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(format!("data:{mime};base64,{b64}"))
}

#[tauri::command]
pub fn get_screenshot_image(db: State<'_, Database>, id: i64) -> CommandResult<String> {
    get_screenshot_image_data_internal(&db, id)
}

/// Opens the screenshot using the native operating system default viewer.
/// Security boundary: receives strictly `id: i64`, looks up verified path in database.
#[tauri::command]
pub fn open_screenshot(db: State<'_, Database>, id: i64) -> CommandResult<bool> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    let detail = screenshots::get_screenshot_by_id(&conn, id)?
        .ok_or_else(|| AppError::file_not_found(format!("Screenshot with ID {id} not found")))?;

    if !Path::new(&detail.path).exists() {
        return Err(AppError::file_not_found(format!(
            "Screenshot file no longer exists at: {}",
            detail.path
        )));
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", &detail.path])
            .spawn()
            .map_err(|e| AppError::unknown(format!("Failed to launch default viewer: {e}")))?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("xdg-open")
            .arg(&detail.path)
            .spawn()
            .map_err(|e| AppError::unknown(format!("Failed to open file: {e}")))?;
    }

    Ok(true)
}

/// Reveals and highlights the screenshot file inside the native OS file explorer.
/// Security boundary: receives strictly `id: i64`, looks up verified path in database.
#[tauri::command]
pub fn reveal_screenshot(db: State<'_, Database>, id: i64) -> CommandResult<bool> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    let detail = screenshots::get_screenshot_by_id(&conn, id)?
        .ok_or_else(|| AppError::file_not_found(format!("Screenshot with ID {id} not found")))?;

    if !Path::new(&detail.path).exists() {
        return Err(AppError::file_not_found(format!(
            "Screenshot file no longer exists at: {}",
            detail.path
        )));
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(format!("/select,{}", detail.path))
            .spawn()
            .map_err(|e| AppError::unknown(format!("Failed to reveal file in explorer: {e}")))?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Some(parent) = Path::new(&detail.path).parent() {
            std::process::Command::new("xdg-open")
                .arg(parent)
                .spawn()
                .map_err(|e| AppError::unknown(format!("Failed to open folder: {e}")))?;
        }
    }

    Ok(true)
}

/// Rebuilds the search index from scratch.
#[tauri::command]
pub fn rebuild_search_index(db: State<'_, Database>) -> CommandResult<usize> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    screenshots::rebuild_search_index(&conn)
}

/// Diagnoses search index health comparing FTS entries to indexed screenshots.
#[tauri::command]
pub fn check_search_index_health(db: State<'_, Database>) -> CommandResult<SearchIndexHealth> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    screenshots::check_search_index_health(&conn)
}

/// Retrieves status and information about the local semantic model.
#[tauri::command]
pub fn get_semantic_model_info(
    service: State<'_, std::sync::Arc<crate::indexing::service::IndexingService>>,
) -> CommandResult<crate::semantic::SemanticModelInfo> {
    Ok(service.semantic_mgr().get_model_info())
}

/// Triggers on-demand background download of the semantic embedding model.
#[tauri::command]
pub fn download_semantic_model(
    app: tauri::AppHandle,
    service: State<'_, std::sync::Arc<crate::indexing::service::IndexingService>>,
) -> CommandResult<bool> {
    let service_clone = service.inner().clone();
    let app_clone = app.clone();

    service
        .semantic_mgr()
        .start_download(Some(std::sync::Arc::new(move || {
            use tauri::Emitter;
            log::info!("Semantic model ready. Triggering embedding reconciliation...");
            let _ = service_clone.reconcile_pending_embeddings();
            let _ = app_clone.emit("semantic_model_ready", ());
        })))?;

    Ok(true)
}

/// Rebuilds the semantic embedding index from existing OCR results without re-OCR.
#[tauri::command]
pub fn rebuild_semantic_index(
    service: State<'_, std::sync::Arc<crate::indexing::service::IndexingService>>,
) -> CommandResult<usize> {
    service.rebuild_semantic_index()
}

/// Retrieves aggregated metrics regarding semantic embedding coverage.
#[tauri::command]
pub fn get_embedding_stats(
    service: State<'_, std::sync::Arc<crate::indexing::service::IndexingService>>,
) -> CommandResult<crate::db::embeddings::EmbeddingStats> {
    service.get_embedding_stats()
}
