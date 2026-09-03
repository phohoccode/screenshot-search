use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::Instant;

use crate::db;
use crate::db::folders::FolderRecord;
use crate::errors::AppError;
use crate::filesystem::fingerprint::compute_sha256;
use crate::filesystem::scanner::scan_directory;

/// Summary of discovery / rescan results returned to the frontend.
#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub folder_id: i64,
    pub discovered: usize,
    pub added: usize,
    pub updated: usize,
    pub unchanged: usize,
    pub removed: usize,
    pub failed: usize,
    pub duration_ms: u64,
}

/// Executes a discovery scan for a managed folder.
///
/// Fingerprint Semantics:
/// - Fast-path check: Compares `(path, file_size, modified_at_fs)` against SQLite.
/// - Full content hash: Streaming SHA-256 is computed only for new or modified files.
/// - Note: In-place byte mutation that maliciously or artificially preserves both exact
///   file size and modified timestamp would be treated as unchanged by design to preserve
///   extreme rescan performance on collections with 10,000+ screenshots.
///
/// Reconciles filesystem state with database records:
/// - New files -> inserted with `ocr_status = 'PENDING'`
/// - Modified files -> updated with new metadata & hash, `ocr_status` reset to 'PENDING'
/// - Unchanged files -> skipped
/// - Missing files -> safely removed ONLY if verified genuinely `NotFound` on disk and not
///   within an inaccessible traversal subtree.
pub fn execute_discovery_scan(
    conn: &Connection,
    folder: &FolderRecord,
) -> Result<ScanSummary, AppError> {
    let start_time = Instant::now();
    let root_path = Path::new(&folder.path);

    if !root_path.exists() {
        return Err(AppError::folder_not_found(format!(
            "Folder directory does not exist on disk: {}",
            folder.path
        )));
    }

    // 1. Scan filesystem for image files
    let scan_output = scan_directory(root_path, folder.recursive)
        .map_err(|e| AppError::scan_failed(format!("Filesystem scan failed: {e}")))?;

    // 2. Query existing screenshots from SQLite
    let existing_map = db::screenshots::get_existing_screenshots_for_folder(conn, folder.id)?;

    let mut discovered_paths = HashSet::new();
    let mut added = 0;
    let mut updated = 0;
    let mut unchanged = 0;
    let mut failed = scan_output.file_read_failures;

    // 3. Process discovered files
    for file in &scan_output.files {
        discovered_paths.insert(file.path.clone());

        if let Some(existing) = existing_map.get(&file.path) {
            // Quick check: has size or timestamp changed?
            if existing.file_size == file.file_size
                && existing.modified_at_fs == file.modified_at_fs
            {
                unchanged += 1;
            } else {
                // File modified: compute new hash and update database
                let file_path = Path::new(&file.path);
                match compute_sha256(file_path) {
                    Ok(hash) => {
                        if let Err(e) = db::screenshots::update_screenshot(
                            conn,
                            existing.id,
                            file.file_size,
                            &file.modified_at_fs,
                            &hash,
                        ) {
                            log::warn!("Failed to update screenshot in DB for {}: {e}", file.path);
                            failed += 1;
                        } else {
                            updated += 1;
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "Failed to compute hash for modified file {}: {e}",
                            file.path
                        );
                        failed += 1;
                    }
                }
            }
        } else {
            // New file: compute hash and insert into database
            let file_path = Path::new(&file.path);
            match compute_sha256(file_path) {
                Ok(hash) => {
                    if let Err(e) = db::screenshots::insert_screenshot(
                        conn,
                        folder.id,
                        &file.path,
                        &file.filename,
                        &file.extension,
                        file.file_size,
                        &file.modified_at_fs,
                        &hash,
                    ) {
                        log::warn!("Failed to insert screenshot into DB for {}: {e}", file.path);
                        failed += 1;
                    } else {
                        added += 1;
                    }
                }
                Err(e) => {
                    log::warn!("Failed to compute hash for new file {}: {e}", file.path);
                    failed += 1;
                }
            }
        }
    }

    // 4. Hardened deleted file reconciliation:
    // Records in DB whose files were not found in this scan
    let mut removed = 0;
    for (path, existing) in &existing_map {
        if !discovered_paths.contains(path) {
            // SAFEGUARD 1: Check if this file belongs to any subtree that was inaccessible during traversal.
            // If the parent folder or any ancestor directory failed to read, do NOT assume file was deleted!
            let in_inaccessible_scope = scan_output
                .inaccessible_paths
                .iter()
                .any(|inacc| path.starts_with(inacc));

            if in_inaccessible_scope {
                log::info!(
                    "Preserving DB record for {} because its parent scope was inaccessible during scan",
                    path
                );
                continue;
            }

            // SAFEGUARD 2: Explicitly check filesystem metadata.
            // Do NOT use Path::exists() because it returns false on PermissionDenied or I/O errors.
            match fs::metadata(path) {
                Ok(_) => {
                    // File definitely still exists on disk; preserve record!
                    log::debug!(
                        "File {} was omitted from traversal but exists on disk; preserving DB record",
                        path
                    );
                }
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    // Confirmed: File was genuinely removed from disk by user.
                    if let Err(del_err) = db::screenshots::delete_screenshot(conn, existing.id) {
                        log::warn!(
                            "Failed to delete stale screenshot record {}: {del_err}",
                            path
                        );
                    } else {
                        removed += 1;
                    }
                }
                Err(e) => {
                    // PermissionDenied, device error, etc. Existence cannot be disproved.
                    log::warn!(
                        "Cannot verify existence of {} ({}); preserving DB record for safety",
                        path,
                        e
                    );
                }
            }
        }
    }

    // 5. Update last_scanned_at timestamp on folder
    db::folders::update_last_scanned(conn, folder.id)?;

    let duration_ms = start_time.elapsed().as_millis() as u64;

    log::info!(
        "Scan complete for folder ID {}: discovered={}, added={}, updated={}, unchanged={}, removed={}, failed={}, duration={}ms",
        folder.id,
        scan_output.files.len(),
        added,
        updated,
        unchanged,
        removed,
        failed,
        duration_ms
    );

    Ok(ScanSummary {
        folder_id: folder.id,
        discovered: scan_output.files.len(),
        added,
        updated,
        unchanged,
        removed,
        failed,
        duration_ms,
    })
}
