use std::path::Path;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt;

use crate::db::connection::Database;
use crate::db::folders::{self, FolderRecord};
use crate::errors::{AppError, CommandResult};
use crate::filesystem::metadata::canonicalize_and_normalize;
use crate::indexing::discovery::{execute_discovery_scan, ScanSummary};

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
/// Returns `FolderAlreadyExists` if the path is already registered.
#[tauri::command]
pub fn add_folder(
    db: State<'_, Database>,
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

    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    folders::insert_folder(&conn, &canonical_normalized, is_recursive)
}

/// Removes a folder from Screenshot Search management.
/// Deletes folder metadata from SQLite (which cascades to screenshot records).
/// NEVER deletes original files on the filesystem.
#[tauri::command]
pub fn remove_folder(db: State<'_, Database>, id: i64) -> CommandResult<bool> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    folders::delete_folder(&conn, id)
}

/// Executes a discovery / rescan on a specific folder by ID.
#[tauri::command]
pub fn scan_folder(db: State<'_, Database>, id: i64) -> CommandResult<ScanSummary> {
    let conn = db
        .conn
        .lock()
        .map_err(|e| AppError::database(format!("Failed to acquire database lock: {e}")))?;

    let folder = folders::get_folder_by_id(&conn, id)?
        .ok_or_else(|| AppError::folder_not_found(format!("Folder with id {id} not found")))?;

    execute_discovery_scan(&conn, &folder)
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
