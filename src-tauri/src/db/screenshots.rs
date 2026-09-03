use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
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

/// Item queued for OCR processing.
#[derive(Debug, Clone)]
pub struct PendingScreenshotItem {
    pub id: i64,
    pub folder_id: i64,
    pub path: String,
    pub filename: String,
    pub extension: String,
}

/// Global OCR indexing statistics across all managed screenshots.
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct OcrStats {
    pub total: usize,
    pub pending: usize,
    pub processing: usize,
    pub succeeded: usize,
    pub failed: usize,
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

/// Crash Recovery: Resets any stale `PROCESSING` jobs to `PENDING` on application startup.
pub fn recover_stale_processing(conn: &Connection) -> Result<usize, AppError> {
    let affected = conn
        .execute(
            "UPDATE screenshots 
             SET ocr_status = 'PENDING', updated_at = datetime('now') 
             WHERE ocr_status = 'PROCESSING'",
            [],
        )
        .map_err(|e| AppError::database(format!("Failed to recover stale processing jobs: {e}")))?;

    if affected > 0 {
        log::info!("Recovered {affected} stale PROCESSING screenshots back to PENDING");
    }

    Ok(affected)
}

/// Queries next batch of pending screenshots for OCR recognition.
pub fn get_pending_screenshots(
    conn: &Connection,
    folder_id: Option<i64>,
    limit: usize,
) -> Result<Vec<PendingScreenshotItem>, AppError> {
    let query = match folder_id {
        Some(_) => {
            "SELECT id, folder_id, path, filename, extension
             FROM screenshots
             WHERE ocr_status = 'PENDING' AND folder_id = ?1
             ORDER BY id ASC
             LIMIT ?2"
        }
        None => {
            "SELECT id, folder_id, path, filename, extension
             FROM screenshots
             WHERE ocr_status = 'PENDING'
             ORDER BY id ASC
             LIMIT ?1"
        }
    };

    let mut stmt = conn
        .prepare(query)
        .map_err(|e| AppError::database(format!("Failed to prepare pending query: {e}")))?;

    let rows = match folder_id {
        Some(f_id) => stmt.query_map(params![f_id, limit as i64], |row| {
            Ok(PendingScreenshotItem {
                id: row.get(0)?,
                folder_id: row.get(1)?,
                path: row.get(2)?,
                filename: row.get(3)?,
                extension: row.get(4)?,
            })
        }),
        None => stmt.query_map(params![limit as i64], |row| {
            Ok(PendingScreenshotItem {
                id: row.get(0)?,
                folder_id: row.get(1)?,
                path: row.get(2)?,
                filename: row.get(3)?,
                extension: row.get(4)?,
            })
        }),
    }
    .map_err(|e| AppError::database(format!("Failed to execute pending query: {e}")))?;

    let mut items = Vec::new();
    for row in rows {
        items.push(
            row.map_err(|e| AppError::database(format!("Failed to read pending item: {e}")))?,
        );
    }

    Ok(items)
}

/// Atomically claims a screenshot for processing by transitioning it from `PENDING` to `PROCESSING`.
/// Returns `Ok(true)` if claimed successfully, or `Ok(false)` if already claimed or no longer pending.
pub fn mark_processing(conn: &Connection, id: i64) -> Result<bool, AppError> {
    let affected = conn
        .execute(
            "UPDATE screenshots 
             SET ocr_status = 'PROCESSING', updated_at = datetime('now') 
             WHERE id = ?1 AND ocr_status = 'PENDING'",
            params![id],
        )
        .map_err(|e| AppError::database(format!("Failed to mark screenshot processing: {e}")))?;

    Ok(affected == 1)
}

/// Persists successful OCR text and updates status to `SUCCEEDED`.
pub fn save_ocr_success(
    conn: &Connection,
    id: i64,
    ocr_text: &str,
    ocr_engine: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE screenshots 
         SET ocr_status = 'SUCCEEDED', 
             ocr_text = ?1, 
             ocr_engine = ?2, 
             indexed_at = datetime('now'), 
             updated_at = datetime('now') 
         WHERE id = ?3",
        params![ocr_text, ocr_engine, id],
    )
    .map_err(|e| AppError::database(format!("Failed to save OCR success: {e}")))?;

    Ok(())
}

/// Marks a screenshot as `FAILED` if OCR recognition fails.
pub fn mark_ocr_failed(conn: &Connection, id: i64, ocr_engine: &str) -> Result<(), AppError> {
    conn.execute(
        "UPDATE screenshots 
         SET ocr_status = 'FAILED', 
             ocr_engine = ?1, 
             updated_at = datetime('now') 
         WHERE id = ?2",
        params![ocr_engine, id],
    )
    .map_err(|e| AppError::database(format!("Failed to mark OCR failed: {e}")))?;

    Ok(())
}

/// Aggregates total, pending, processing, succeeded, and failed screenshot counts.
pub fn get_ocr_stats(conn: &Connection) -> Result<OcrStats, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT 
                COUNT(*) as total,
                SUM(CASE WHEN ocr_status = 'PENDING' THEN 1 ELSE 0 END) as pending,
                SUM(CASE WHEN ocr_status = 'PROCESSING' THEN 1 ELSE 0 END) as processing,
                SUM(CASE WHEN ocr_status = 'SUCCEEDED' THEN 1 ELSE 0 END) as succeeded,
                SUM(CASE WHEN ocr_status = 'FAILED' THEN 1 ELSE 0 END) as failed
             FROM screenshots",
        )
        .map_err(|e| AppError::database(format!("Failed to prepare OCR stats query: {e}")))?;

    let stats = stmt
        .query_row([], |row| {
            let total: i64 = row.get(0)?;
            let pending: Option<i64> = row.get(1)?;
            let processing: Option<i64> = row.get(2)?;
            let succeeded: Option<i64> = row.get(3)?;
            let failed: Option<i64> = row.get(4)?;

            Ok(OcrStats {
                total: total as usize,
                pending: pending.unwrap_or(0) as usize,
                processing: processing.unwrap_or(0) as usize,
                succeeded: succeeded.unwrap_or(0) as usize,
                failed: failed.unwrap_or(0) as usize,
            })
        })
        .map_err(|e| AppError::database(format!("Failed to query OCR stats: {e}")))?;

    Ok(stats)
}
