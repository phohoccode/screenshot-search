use std::path::Path;
use tauri::State;

use crate::db::connection::Database;
use crate::db::screenshots::{self, ScreenshotDetail, SearchIndexHealth};
use crate::errors::{AppError, CommandResult};
use crate::search::{self, SearchRequest, SearchResultPage};

/// Executes full-text search against the SQLite FTS5 index.
#[tauri::command]
pub fn search_screenshots(
    db: State<'_, Database>,
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
