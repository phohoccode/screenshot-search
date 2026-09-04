use rusqlite::{params, Connection, OptionalExtension};
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

    // Immediately purge stale FTS record so modified file ceases matching previous content
    let _ = conn.execute("DELETE FROM screenshots_fts WHERE rowid = ?1", params![id]);

    Ok(())
}

/// Deletes a screenshot by its ID (used when the original file is deleted on disk).
/// Only removes database index metadata — NEVER touches the filesystem.
pub fn delete_screenshot(conn: &Connection, id: i64) -> Result<(), AppError> {
    let _ = conn.execute("DELETE FROM screenshots_fts WHERE rowid = ?1", params![id]);

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

    let map_row = |row: &rusqlite::Row| {
        Ok(PendingScreenshotItem {
            id: row.get(0)?,
            folder_id: row.get(1)?,
            path: row.get(2)?,
            filename: row.get(3)?,
            extension: row.get(4)?,
        })
    };

    let rows = match folder_id {
        Some(f_id) => stmt.query_map(params![f_id, limit as i64], map_row),
        None => stmt.query_map(params![limit as i64], map_row),
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

/// Detailed representation of a screenshot for modal preview and inspection.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScreenshotDetail {
    pub id: i64,
    pub folder_id: i64,
    pub path: String,
    pub filename: String,
    pub extension: String,
    pub file_size: u64,
    pub modified_at_fs: String,
    pub content_hash: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub ocr_text: Option<String>,
    pub ocr_status: String,
    pub ocr_engine: Option<String>,
    pub indexed_at: Option<String>,
}

/// Search index health diagnostics comparing FTS entries to searchable screenshots.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchIndexHealth {
    pub fts_count: usize,
    pub succeeded_count: usize,
    pub is_healthy: bool,
}

/// Persists successful OCR text and updates status to `SUCCEEDED`.
/// Synchronizes the normalized search representation to SQLite FTS5 index.
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

    // Synchronize to FTS5 index
    let search_text = crate::search::normalize::normalize_search_text(ocr_text);
    conn.execute(
        "INSERT OR REPLACE INTO screenshots_fts (rowid, filename, ocr_search_text)
         SELECT id, filename, ?2 FROM screenshots WHERE id = ?1",
        params![id, search_text],
    )
    .map_err(|e| AppError::database(format!("Failed to sync FTS on OCR success: {e}")))?;

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

    // Purge any stale FTS entry if previously succeeded
    let _ = conn.execute("DELETE FROM screenshots_fts WHERE rowid = ?1", params![id]);

    Ok(())
}

fn map_screenshot_detail_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ScreenshotDetail> {
    let file_size: i64 = row.get(5)?;
    let width: Option<i64> = row.get(8)?;
    let height: Option<i64> = row.get(9)?;

    Ok(ScreenshotDetail {
        id: row.get(0)?,
        folder_id: row.get(1)?,
        path: row.get(2)?,
        filename: row.get(3)?,
        extension: row.get(4)?,
        file_size: file_size as u64,
        modified_at_fs: row.get(6)?,
        content_hash: row.get(7)?,
        width: width.map(|w| w as u32),
        height: height.map(|h| h as u32),
        ocr_text: row.get(10)?,
        ocr_status: row.get(11)?,
        ocr_engine: row.get(12)?,
        indexed_at: row.get(13)?,
    })
}

/// Retrieves complete metadata and OCR text for a single screenshot by ID.
pub fn get_screenshot_by_id(
    conn: &Connection,
    id: i64,
) -> Result<Option<ScreenshotDetail>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT 
                id, folder_id, path, filename, extension, file_size, modified_at_fs,
                content_hash, width, height, ocr_text, ocr_status, ocr_engine, indexed_at
             FROM screenshots 
             WHERE id = ?1",
        )
        .map_err(|e| AppError::database(format!("Failed to prepare get_screenshot_by_id: {e}")))?;

    let mut rows = stmt
        .query_map(params![id], map_screenshot_detail_row)
        .map_err(|e| AppError::database(format!("Failed to query screenshot detail: {e}")))?;

    match rows.next() {
        Some(Ok(detail)) => Ok(Some(detail)),
        Some(Err(e)) => Err(AppError::database(format!(
            "Failed to read screenshot detail: {e}"
        ))),
        None => Ok(None),
    }
}

/// Retrieves complete metadata and OCR text for a single screenshot by path.
pub fn get_screenshot_by_path(
    conn: &Connection,
    path: &str,
) -> Result<Option<ScreenshotDetail>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT 
                id, folder_id, path, filename, extension, file_size, modified_at_fs,
                content_hash, width, height, ocr_text, ocr_status, ocr_engine, indexed_at
             FROM screenshots 
             WHERE path = ?1",
        )
        .map_err(|e| {
            AppError::database(format!("Failed to prepare get_screenshot_by_path: {e}"))
        })?;

    let mut rows = stmt
        .query_map(params![path], map_screenshot_detail_row)
        .map_err(|e| AppError::database(format!("Failed to query screenshot detail: {e}")))?;

    match rows.next() {
        Some(Ok(detail)) => Ok(Some(detail)),
        Some(Err(e)) => Err(AppError::database(format!(
            "Failed to read screenshot detail: {e}"
        ))),
        None => Ok(None),
    }
}

/// Rebuilds the entire FTS5 search index from scratch using source data in `screenshots`.
/// Idempotent maintenance operation ensuring complete index recoverability.
pub fn rebuild_search_index(conn: &Connection) -> Result<usize, AppError> {
    conn.execute("DELETE FROM screenshots_fts", [])
        .map_err(|e| AppError::database(format!("Failed to clear FTS index: {e}")))?;

    let mut stmt = conn
        .prepare(
            "SELECT id, filename, ocr_text 
             FROM screenshots 
             WHERE ocr_status = 'SUCCEEDED' AND ocr_text IS NOT NULL",
        )
        .map_err(|e| AppError::database(format!("Failed to prepare rebuild query: {e}")))?;

    let rows = stmt
        .query_map([], |row| {
            let id: i64 = row.get(0)?;
            let filename: String = row.get(1)?;
            let ocr_text: String = row.get(2)?;
            Ok((id, filename, ocr_text))
        })
        .map_err(|e| AppError::database(format!("Failed to query succeeded screenshots: {e}")))?;

    let mut count = 0;
    for row in rows {
        let (id, filename, ocr_text) =
            row.map_err(|e| AppError::database(format!("Failed to read screenshot row: {e}")))?;
        let search_text = crate::search::normalize::normalize_search_text(&ocr_text);
        conn.execute(
            "INSERT OR REPLACE INTO screenshots_fts (rowid, filename, ocr_search_text) 
             VALUES (?1, ?2, ?3)",
            params![id, filename, search_text],
        )
        .map_err(|e| {
            AppError::database(format!("Failed to index screenshot {id} into FTS: {e}"))
        })?;
        count += 1;
    }

    log::info!("Rebuilt search index: indexed {count} screenshot(s)");
    Ok(count)
}

/// Diagnoses search index health by comparing FTS index size with SUCCEEDED screenshot records.
pub fn check_search_index_health(conn: &Connection) -> Result<SearchIndexHealth, AppError> {
    let fts_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM screenshots_fts", [], |row| row.get(0))
        .unwrap_or(0);

    let succeeded_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM screenshots WHERE ocr_status = 'SUCCEEDED' AND ocr_text IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let is_healthy = fts_count == succeeded_count;
    Ok(SearchIndexHealth {
        fts_count: fts_count as usize,
        succeeded_count: succeeded_count as usize,
        is_healthy,
    })
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

/// Atomically renames a screenshot record in `screenshots` and `screenshots_fts` from `from_path` to `to_path`.
/// Returns Ok(true) if the record was found and renamed, or Ok(false) if from_path was not registered.
pub fn rename_screenshot(
    conn: &Connection,
    folder_id: i64,
    from_path: &str,
    to_path: &str,
) -> Result<bool, AppError> {
    let existing_opt = get_screenshot_by_path(conn, from_path)?;
    let existing = match existing_opt {
        Some(s) => s,
        None => return Ok(false),
    };

    let to_path_obj = std::path::Path::new(to_path);
    let new_filename = to_path_obj
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(to_path);
    let new_extension = to_path_obj
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    // 1. Update screenshots record with new path, filename, and extension
    conn.execute(
        "UPDATE screenshots 
         SET path = ?1, filename = ?2, extension = ?3, updated_at = datetime('now')
         WHERE id = ?4",
        params![to_path, new_filename, new_extension, existing.id],
    )
    .map_err(|e| AppError::database(format!("Failed to rename screenshot path: {e}")))?;

    // 2. Synchronize FTS5 filename
    let _ = conn.execute(
        "UPDATE screenshots_fts SET filename = ?1 WHERE rowid = ?2",
        params![new_filename, existing.id],
    );

    // 3. Remove any pending DELETE job for from_path in index_jobs
    let _ = conn.execute(
        "DELETE FROM index_jobs WHERE folder_id = ?1 AND path = ?2 AND job_type = 'DELETE_SCREENSHOT'",
        params![folder_id, from_path],
    );

    log::info!(
        "Atomically renamed screenshot record {}: {} -> {}",
        existing.id,
        from_path,
        to_path
    );

    Ok(true)
}

/// Queries a screenshot by its folder_id and content_hash.
pub fn get_screenshot_by_hash(
    conn: &Connection,
    folder_id: i64,
    content_hash: &str,
) -> Result<Option<ScreenshotDetail>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT 
                id, folder_id, path, filename, extension, file_size, 
                modified_at_fs, content_hash, width, height, ocr_text, ocr_status, 
                ocr_engine, indexed_at
             FROM screenshots 
             WHERE folder_id = ?1 AND content_hash = ?2
             ORDER BY id DESC
             LIMIT 1",
        )
        .map_err(|e| {
            AppError::database(format!("Failed to prepare get_screenshot_by_hash: {e}"))
        })?;

    let result = stmt
        .query_row(params![folder_id, content_hash], map_screenshot_detail_row)
        .optional()
        .map_err(|e| AppError::database(format!("Failed to query screenshot by hash: {e}")))?;

    Ok(result)
}
