use rusqlite::{params, Connection};
use std::collections::HashMap;

use crate::errors::AppError;

/// Lightweight representation of an existing screenshot for change detection.
#[derive(Debug, Clone)]
pub struct ExistingScreenshot {
    pub id: i64,
    pub path: String,
    pub file_size: u64,
    pub modified_at_fs: String,
    pub content_hash: Option<String>,
}

/// Retrieves a map of path -> ExistingScreenshot for all screenshots belonging to a folder.
pub fn get_existing_screenshots_for_folder(
    conn: &Connection,
    folder_id: i64,
) -> Result<HashMap<String, ExistingScreenshot>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, path, file_size, modified_at_fs, content_hash
             FROM screenshots
             WHERE folder_id = ?1",
        )
        .map_err(|e| AppError::database(format!("Failed to prepare get screenshots query: {e}")))?;

    let rows = stmt
        .query_map(params![folder_id], |row| {
            let id: i64 = row.get(0)?;
            let path: String = row.get(1)?;
            let file_size: i64 = row.get(2)?;
            let modified_at_fs: String = row.get(3)?;
            let content_hash: Option<String> = row.get(4)?;

            Ok(ExistingScreenshot {
                id,
                path,
                file_size: file_size as u64,
                modified_at_fs,
                content_hash,
            })
        })
        .map_err(|e| AppError::database(format!("Failed to query screenshots for folder: {e}")))?;

    let mut map = HashMap::new();
    for row in rows {
        let screenshot =
            row.map_err(|e| AppError::database(format!("Failed to read screenshot row: {e}")))?;
        map.insert(screenshot.path.clone(), screenshot);
    }

    Ok(map)
}

/// Inserts a newly discovered screenshot.
pub fn insert_screenshot(
    conn: &Connection,
    folder_id: i64,
    path: &str,
    filename: &str,
    extension: &str,
    file_size: u64,
    modified_at_fs: &str,
    content_hash: &str,
) -> Result<i64, AppError> {
    conn.execute(
        "INSERT INTO screenshots (
            folder_id, path, filename, extension, file_size,
            modified_at_fs, content_hash, ocr_status
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'PENDING')",
        params![
            folder_id,
            path,
            filename,
            extension,
            file_size as i64,
            modified_at_fs,
            content_hash
        ],
    )
    .map_err(|e| AppError::database(format!("Failed to insert screenshot: {e}")))?;

    Ok(conn.last_insert_rowid())
}

/// Updates an existing screenshot when size, timestamp, or content has changed.
/// Resets `ocr_status` to 'PENDING' and clears any stale OCR text.
pub fn update_screenshot(
    conn: &Connection,
    id: i64,
    file_size: u64,
    modified_at_fs: &str,
    content_hash: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE screenshots SET
            file_size = ?1,
            modified_at_fs = ?2,
            content_hash = ?3,
            ocr_status = 'PENDING',
            ocr_text = NULL,
            updated_at = datetime('now')
         WHERE id = ?4",
        params![file_size as i64, modified_at_fs, content_hash, id],
    )
    .map_err(|e| AppError::database(format!("Failed to update screenshot: {e}")))?;

    Ok(())
}

/// Deletes a screenshot by its ID (used when the original file is deleted on disk).
/// Only removes database index metadata — NEVER touches the filesystem.
pub fn delete_screenshot(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute("DELETE FROM screenshots WHERE id = ?1", params![id])
        .map_err(|e| AppError::database(format!("Failed to delete screenshot: {e}")))?;

    Ok(())
}

/// Returns the total number of screenshots currently registered for a folder.
pub fn count_for_folder(conn: &Connection, folder_id: i64) -> Result<usize, AppError> {
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM screenshots WHERE folder_id = ?1",
            params![folder_id],
            |row| row.get(0),
        )
        .map_err(|e| AppError::database(format!("Failed to count screenshots: {e}")))?;

    Ok(count as usize)
}
