use std::path::Path;
use std::sync::Arc;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::db::connection::Database;
use crate::db::folders::{self, FolderRecord};
use crate::db::jobs::{self, JOB_TYPE_UPSERT};
use crate::errors::{AppError, CommandResult};
use crate::filesystem::metadata::canonicalize_and_normalize;
use crate::indexing::discovery::{execute_discovery_scan, ScanSummary};
use crate::indexing::service::IndexingService;

/// Lists all registered folders along with their screenshot count.
#[tauri::command]
pub fn list_folders(db: State<'_, Database>) -> CommandResult<Vec<FolderRecord>> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    folders::list_folders(&conn)
}

/// Adds a new folder for screenshot indexing.
/// Validates that the path exists and is a directory.
/// Resolves canonical casing and normalizes path before database check.
/// Registers with the filesystem watcher immediately.
/// Returns `FolderAlreadyExists` if the path is already registered.
#[tauri::command]
pub fn add_folder(
    db: State<'_, Database>,
    indexing_service: State<'_, Arc<IndexingService>>,
    path: String,
    recursive: Option<bool>,
) -> CommandResult<FolderRecord> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(AppError::invalid_path("Folder path cannot be empty"));
    }

    let p = Path::new(trimmed);
    if !p.exists() {
        return Err(AppError::folder_not_found(format!(
            "Directory does not exist: {trimmed}"
        )));
    }

    if !p.is_dir() {
        return Err(AppError::invalid_path(format!(
            "Specified path is not a directory: {trimmed}"
        )));
    }

    let canonical_normalized = canonicalize_and_normalize(p);
    let is_recursive = recursive.unwrap_or(true);

    let record = {
        let conn = db
            .conn
            .lock()
            .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

        folders::insert_folder(&conn, &canonical_normalized, is_recursive)?
    };

    // Register active filesystem watcher for newly added folder
    if let Err(e) =
        indexing_service
            .watcher()
            .watch_folder(record.id, &record.path, record.recursive)
    {
        log::warn!(
            "Failed to start watcher for newly added folder {}: {e}",
            record.id
        );
    }

    Ok(record)
}

/// Removes a folder from Screenshot Search management.
/// Unregisters from filesystem watcher, cancels pending jobs, and deletes folder metadata from SQLite.
/// NEVER deletes original files on the filesystem.
#[tauri::command]
pub fn remove_folder(
    db: State<'_, Database>,
    indexing_service: State<'_, Arc<IndexingService>>,
    id: i64,
) -> CommandResult<bool> {
    // 1. Unwatch the folder
    let _ = indexing_service.watcher().unwatch_folder(id);

    // 2. Delete folder metadata and clean up
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    // Clean up any remaining index_jobs for this folder
    let _ = conn.execute(
        "DELETE FROM index_jobs WHERE folder_id = ?1",
        rusqlite::params![id],
    );

    folders::delete_folder(&conn, id)
}

/// Executes a discovery / rescan on a specific folder by ID.
/// Automatically enqueues any discovered PENDING screenshots into the durable queue.
#[tauri::command]
pub fn scan_folder(db: State<'_, Database>, id: i64) -> CommandResult<ScanSummary> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    let folder = folders::get_folder_by_id(&conn, id)?
        .ok_or_else(|| AppError::folder_not_found(format!("Folder with id {id} not found")))?;

    let summary = execute_discovery_scan(&conn, &folder)?;

    // Enqueue any pending screenshots from this folder into index_jobs
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, path, content_hash FROM screenshots 
         WHERE folder_id = ?1 AND ocr_status = 'PENDING'",
    ) {
        let rows = stmt.query_map([id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        });
        if let Ok(mapped) = rows {
            for item in mapped.flatten() {
                let (_, path, content_hash) = item;
                let dedupe_key = format!(
                    "UPSERT:{}:{}:{}",
                    id,
                    path,
                    content_hash.unwrap_or_default()
                );
                let _ = jobs::enqueue_job(&conn, id, &path, JOB_TYPE_UPSERT, &dedupe_key);
            }
        }
    }

    Ok(summary)
}

/// Opens the native OS folder picker dialog.
/// Returns `Some(normalized_path)` if a folder was selected, or `None` if cancelled.
#[tauri::command]
pub fn pick_folder(app: AppHandle) -> CommandResult<Option<String>> {
    let result = app.dialog().file().blocking_pick_folder();

    match result {
        Some(file_path) => {
            let path_str = file_path.to_string();
            let normalized = canonicalize_and_normalize(Path::new(&path_str));
            Ok(Some(normalized))
        }
        None => Ok(None),
    }
}

/// Returns the total number of screenshots indexed across all folders.
#[tauri::command]
pub fn get_total_screenshot_count(db: State<'_, Database>) -> CommandResult<usize> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM screenshots", [], |row| row.get(0))
        .map_err(AppError::from)?;

    Ok(count as usize)
}
