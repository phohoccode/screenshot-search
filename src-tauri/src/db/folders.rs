use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use crate::errors::AppError;

/// Representation of a managed folder row in SQLite with screenshot count and OCR count.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct FolderRecord {
    pub id: i64,
    pub path: String,
    pub enabled: bool,
    pub recursive: bool,
    pub created_at: String,
    pub updated_at: String,
    pub last_scanned_at: Option<String>,
    pub screenshot_count: usize,
    pub ocr_succeeded_count: usize,
}

/// Inserts a new folder into the database.
/// Fails with `FolderAlreadyExists` if the path is already registered.
pub fn insert_folder(
    conn: &Connection,
    normalized_path: &str,
    recursive: bool,
) -> Result<FolderRecord, AppError> {
    // Check if path already exists
    if let Some(existing) = get_folder_by_path(conn, normalized_path)? {
        return Err(AppError::folder_already_exists(format!(
            "Folder is already registered: {}",
            existing.path
        )));
    }

    conn.execute(
        "INSERT INTO folders (path, enabled, recursive) VALUES (?1, 1, ?2)",
        params![normalized_path, if recursive { 1 } else { 0 }],
    )
    .map_err(|e| AppError::database(format!("Failed to insert folder: {e}")))?;

    let id = conn.last_insert_rowid();

    get_folder_by_id(conn, id)?.ok_or_else(|| {
        AppError::database(format!(
            "Failed to retrieve newly inserted folder with id {id}"
        ))
    })
}

/// Lists all folders along with their current screenshot count and OCR indexed count.
pub fn list_folders(conn: &Connection) -> Result<Vec<FolderRecord>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT 
                f.id, f.path, f.enabled, f.recursive, f.created_at, f.updated_at, f.last_scanned_at,
                COUNT(s.id) as screenshot_count,
                SUM(CASE WHEN s.ocr_status = 'SUCCEEDED' THEN 1 ELSE 0 END) as ocr_succeeded_count
             FROM folders f
             LEFT JOIN screenshots s ON f.id = s.folder_id
             GROUP BY f.id
             ORDER BY f.created_at DESC",
        )
        .map_err(|e| AppError::database(format!("Failed to prepare list folders query: {e}")))?;

    let rows = stmt
        .query_map([], |row| {
            let id: i64 = row.get(0)?;
            let path: String = row.get(1)?;
            let enabled_int: i32 = row.get(2)?;
            let recursive_int: i32 = row.get(3)?;
            let created_at: String = row.get(4)?;
            let updated_at: String = row.get(5)?;
            let last_scanned_at: Option<String> = row.get(6)?;
            let screenshot_count: usize = row.get(7)?;
            let ocr_succeeded_count: Option<i64> = row.get(8)?;

            Ok(FolderRecord {
                id,
                path,
                enabled: enabled_int == 1,
                recursive: recursive_int == 1,
                created_at,
                updated_at,
                last_scanned_at,
                screenshot_count,
                ocr_succeeded_count: ocr_succeeded_count.unwrap_or(0) as usize,
            })
        })
        .map_err(|e| AppError::database(format!("Failed to execute list folders query: {e}")))?;

    let mut folders = Vec::new();
    for row in rows {
        folders
            .push(row.map_err(|e| AppError::database(format!("Failed to read folder row: {e}")))?);
    }

    Ok(folders)
}

/// Retrieves a folder by its database ID.
pub fn get_folder_by_id(conn: &Connection, id: i64) -> Result<Option<FolderRecord>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT 
                f.id, f.path, f.enabled, f.recursive, f.created_at, f.updated_at, f.last_scanned_at,
                COUNT(s.id) as screenshot_count,
                SUM(CASE WHEN s.ocr_status = 'SUCCEEDED' THEN 1 ELSE 0 END) as ocr_succeeded_count
             FROM folders f
             LEFT JOIN screenshots s ON f.id = s.folder_id
             WHERE f.id = ?1
             GROUP BY f.id",
        )
        .map_err(|e| AppError::database(format!("Failed to prepare get folder query: {e}")))?;

    let mut rows = stmt
        .query_map(params![id], |row| {
            let id: i64 = row.get(0)?;
            let path: String = row.get(1)?;
            let enabled_int: i32 = row.get(2)?;
            let recursive_int: i32 = row.get(3)?;
            let created_at: String = row.get(4)?;
            let updated_at: String = row.get(5)?;
            let last_scanned_at: Option<String> = row.get(6)?;
            let screenshot_count: usize = row.get(7)?;
            let ocr_succeeded_count: Option<i64> = row.get(8)?;

            Ok(FolderRecord {
                id,
                path,
                enabled: enabled_int == 1,
                recursive: recursive_int == 1,
                created_at,
                updated_at,
                last_scanned_at,
                screenshot_count,
                ocr_succeeded_count: ocr_succeeded_count.unwrap_or(0) as usize,
            })
        })
        .map_err(|e| AppError::database(format!("Failed to execute get folder query: {e}")))?;

    match rows.next() {
        Some(Ok(folder)) => Ok(Some(folder)),
        Some(Err(e)) => Err(AppError::database(format!("Failed to read folder: {e}"))),
        None => Ok(None),
    }
}

/// Retrieves a folder by its normalized path.
pub fn get_folder_by_path(conn: &Connection, path: &str) -> Result<Option<FolderRecord>, AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT 
                f.id, f.path, f.enabled, f.recursive, f.created_at, f.updated_at, f.last_scanned_at,
                COUNT(s.id) as screenshot_count,
                SUM(CASE WHEN s.ocr_status = 'SUCCEEDED' THEN 1 ELSE 0 END) as ocr_succeeded_count
             FROM folders f
             LEFT JOIN screenshots s ON f.id = s.folder_id
             WHERE LOWER(f.path) = LOWER(?1)
             GROUP BY f.id",
        )
        .map_err(|e| AppError::database(format!("Failed to prepare get folder by path: {e}")))?;

    let mut rows = stmt
        .query_map(params![path], |row| {
            let id: i64 = row.get(0)?;
            let path: String = row.get(1)?;
            let enabled_int: i32 = row.get(2)?;
            let recursive_int: i32 = row.get(3)?;
            let created_at: String = row.get(4)?;
            let updated_at: String = row.get(5)?;
            let last_scanned_at: Option<String> = row.get(6)?;
            let screenshot_count: usize = row.get(7)?;
            let ocr_succeeded_count: Option<i64> = row.get(8)?;

            Ok(FolderRecord {
                id,
                path,
                enabled: enabled_int == 1,
                recursive: recursive_int == 1,
                created_at,
                updated_at,
                last_scanned_at,
                screenshot_count,
                ocr_succeeded_count: ocr_succeeded_count.unwrap_or(0) as usize,
            })
        })
        .map_err(|e| AppError::database(format!("Failed to execute get folder by path: {e}")))?;

    match rows.next() {
        Some(Ok(folder)) => Ok(Some(folder)),
        Some(Err(e)) => Err(AppError::database(format!("Failed to read folder: {e}"))),
        None => Ok(None),
    }
}

/// Deletes a folder by ID. Due to CASCADE FOREIGN KEY, child screenshots are automatically removed.
/// Does NOT touch any files on the filesystem.
pub fn delete_folder(conn: &Connection, id: i64) -> Result<bool, AppError> {
    // Purge corresponding FTS records for screenshots belonging to this folder
    let _ = conn.execute(
        "DELETE FROM screenshots_fts WHERE rowid IN (SELECT id FROM screenshots WHERE folder_id = ?1)",
        params![id],
    );

    let rows_affected = conn
        .execute("DELETE FROM folders WHERE id = ?1", params![id])
        .map_err(|e| AppError::database(format!("Failed to delete folder: {e}")))?;

    Ok(rows_affected > 0)
}

/// Updates the last_scanned_at timestamp for a folder to the current time.
pub fn update_last_scanned(conn: &Connection, id: i64) -> Result<(), AppError> {
    conn.execute(
        "UPDATE folders SET last_scanned_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1",
        params![id],
    )
    .map_err(|e| AppError::database(format!("Failed to update folder last_scanned_at: {e}")))?;

    Ok(())
}
