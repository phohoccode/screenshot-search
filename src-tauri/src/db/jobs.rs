use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::errors::AppError;

/// Types of jobs supported by the durable background indexing queue.
pub const JOB_TYPE_UPSERT: &str = "UPSERT_SCREENSHOT";
pub const JOB_TYPE_DELETE: &str = "DELETE_SCREENSHOT";

/// Lifecycle status of an index job.
pub const JOB_STATUS_PENDING: &str = "PENDING";
pub const JOB_STATUS_PROCESSING: &str = "PROCESSING";
pub const JOB_STATUS_SUCCEEDED: &str = "SUCCEEDED";
pub const JOB_STATUS_FAILED: &str = "FAILED";

/// In-memory representation of a durable index job record from SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexJobRecord {
    pub id: i64,
    pub folder_id: i64,
    pub screenshot_id: Option<i64>,
    pub path: String,
    pub job_type: String,
    pub status: String,
    pub dedupe_key: String,
    pub attempts: i32,
    pub max_attempts: i32,
    pub available_at: String,
    pub lease_until: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

/// Aggregated diagnostic metrics for the durable indexing queue.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IndexJobStats {
    pub pending: usize,
    pub processing: usize,
    pub succeeded: usize,
    pub failed: usize,
    pub total: usize,
}

/// Enqueues a job into the durable SQLite index_jobs table.
/// Deduplication invariant: If a job with the same `dedupe_key` already exists in PENDING or PROCESSING,
/// the insert is silently ignored to prevent duplicate work.
/// If an existing job with the same dedupe_key previously FAILED, it is re-opened as PENDING.
pub fn enqueue_job(
    conn: &Connection,
    folder_id: i64,
    path: &str,
    job_type: &str,
    dedupe_key: &str,
) -> Result<Option<i64>, AppError> {
    // Check if an active (PENDING or PROCESSING) job with this dedupe_key exists
    let existing_active: Option<i64> = conn
        .query_row(
            "SELECT id FROM index_jobs 
             WHERE dedupe_key = ?1 AND status IN ('PENDING', 'PROCESSING')",
            params![dedupe_key],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| AppError::database(format!("Failed to check existing active job: {e}")))?;

    if existing_active.is_some() {
        return Ok(None);
    }

    // Insert or re-activate failed job
    let mut stmt = conn
        .prepare(
            "INSERT INTO index_jobs (folder_id, path, job_type, dedupe_key, status, available_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 'PENDING', datetime('now'), datetime('now'))
             ON CONFLICT(dedupe_key) DO UPDATE SET
                 status = 'PENDING',
                 attempts = 0,
                 available_at = datetime('now'),
                 lease_until = NULL,
                 last_error_code = NULL,
                 last_error_message = NULL,
                 updated_at = datetime('now')
             WHERE status = 'FAILED'
             RETURNING id",
        )
        .map_err(|e| AppError::database(format!("Failed to prepare enqueue statement: {e}")))?;

    let inserted_id: Option<i64> = stmt
        .query_row(params![folder_id, path, job_type, dedupe_key], |row| {
            row.get(0)
        })
        .optional()
        .map_err(|e| AppError::database(format!("Failed to enqueue index job for {path}: {e}")))?;

    Ok(inserted_id)
}

/// Atomically claims the next available PENDING job using SQLite conditional update with RETURNING.
/// Sets status = 'PROCESSING' and extends lease_until by lease_seconds.
/// Returns Ok(None) if no available PENDING jobs exist.
pub fn claim_next_job(
    conn: &Connection,
    lease_seconds: u64,
) -> Result<Option<IndexJobRecord>, AppError> {
    let lease_str = format!("+{lease_seconds} seconds");
    let mut stmt = conn
        .prepare(
            "UPDATE index_jobs
             SET status = 'PROCESSING',
                 lease_until = datetime('now', ?1),
                 attempts = attempts + 1,
                 updated_at = datetime('now')
             WHERE id = (
                 SELECT id FROM index_jobs
                 WHERE status = 'PENDING' AND available_at <= datetime('now')
                 ORDER BY id ASC LIMIT 1
             )
             RETURNING id, folder_id, screenshot_id, path, job_type, status, dedupe_key,
                       attempts, max_attempts, available_at, lease_until,
                       last_error_code, last_error_message, created_at, updated_at, completed_at",
        )
        .map_err(|e| AppError::database(format!("Failed to prepare claim statement: {e}")))?;

    let job: Option<IndexJobRecord> = stmt
        .query_row(params![lease_str], |row| {
            Ok(IndexJobRecord {
                id: row.get(0)?,
                folder_id: row.get(1)?,
                screenshot_id: row.get(2)?,
                path: row.get(3)?,
                job_type: row.get(4)?,
                status: row.get(5)?,
                dedupe_key: row.get(6)?,
                attempts: row.get(7)?,
                max_attempts: row.get(8)?,
                available_at: row.get(9)?,
                lease_until: row.get(10)?,
                last_error_code: row.get(11)?,
                last_error_message: row.get(12)?,
                created_at: row.get(13)?,
                updated_at: row.get(14)?,
                completed_at: row.get(15)?,
            })
        })
        .optional()
        .map_err(|e| AppError::database(format!("Failed to claim next job: {e}")))?;

    Ok(job)
}

/// Marks a claimed job as SUCCEEDED upon successful processing.
pub fn complete_job(
    conn: &Connection,
    job_id: i64,
    screenshot_id: Option<i64>,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE index_jobs
         SET status = 'SUCCEEDED',
             screenshot_id = COALESCE(?2, screenshot_id),
             completed_at = datetime('now'),
             updated_at = datetime('now'),
             lease_until = NULL
         WHERE id = ?1",
        params![job_id, screenshot_id],
    )
    .map_err(|e| AppError::database(format!("Failed to complete index job {job_id}: {e}")))?;

    Ok(())
}

/// Retries a recoverable error with exponential backoff, or transitions to FAILED if max_attempts reached.
pub fn retry_or_fail_job(
    conn: &Connection,
    job_id: i64,
    error_code: &str,
    error_message: &str,
    backoff_seconds: u64,
) -> Result<bool, AppError> {
    let (attempts, max_attempts): (i32, i32) = conn
        .query_row(
            "SELECT attempts, max_attempts FROM index_jobs WHERE id = ?1",
            params![job_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| AppError::database(format!("Failed to query job attempts: {e}")))?;

    if attempts >= max_attempts {
        fail_job(conn, job_id, error_code, error_message)?;
        Ok(false) // Not retrying; marked failed
    } else {
        let backoff_str = format!("+{backoff_seconds} seconds");
        conn.execute(
            "UPDATE index_jobs
             SET status = 'PENDING',
                 available_at = datetime('now', ?2),
                 last_error_code = ?3,
                 last_error_message = ?4,
                 updated_at = datetime('now'),
                 lease_until = NULL
             WHERE id = ?1",
            params![job_id, backoff_str, error_code, error_message],
        )
        .map_err(|e| AppError::database(format!("Failed to schedule job retry: {e}")))?;
        Ok(true) // Scheduled for retry
    }
}

/// Transitions a job directly to terminal FAILED status.
pub fn fail_job(
    conn: &Connection,
    job_id: i64,
    error_code: &str,
    error_message: &str,
) -> Result<(), AppError> {
    conn.execute(
        "UPDATE index_jobs
         SET status = 'FAILED',
             last_error_code = ?2,
             last_error_message = ?3,
             completed_at = datetime('now'),
             updated_at = datetime('now'),
             lease_until = NULL
         WHERE id = ?1",
        params![job_id, error_code, error_message],
    )
    .map_err(|e| AppError::database(format!("Failed to fail index job {job_id}: {e}")))?;

    Ok(())
}

/// Crash Recovery: Recovers orphaned `PROCESSING` jobs whose leases have expired back to `PENDING`.
pub fn recover_stale_leases(conn: &Connection) -> Result<usize, AppError> {
    let recovered = conn
        .execute(
            "UPDATE index_jobs
             SET status = 'PENDING',
                 lease_until = NULL,
                 updated_at = datetime('now')
             WHERE status = 'PROCESSING'
               AND (lease_until IS NULL OR lease_until < datetime('now'))",
            [],
        )
        .map_err(|e| AppError::database(format!("Failed to recover stale job leases: {e}")))?;

    if recovered > 0 {
        log::info!("Recovered {recovered} stale processing index job(s) back to PENDING");
    }

    Ok(recovered)
}

/// Retention Policy: Deletes SUCCEEDED jobs older than max_age_hours to prevent database bloat.
pub fn cleanup_completed_jobs(conn: &Connection, max_age_hours: u32) -> Result<usize, AppError> {
    let age_str = format!("-{max_age_hours} hours");
    let deleted = conn
        .execute(
            "DELETE FROM index_jobs
             WHERE status = 'SUCCEEDED'
               AND completed_at < datetime('now', ?1)",
            params![age_str],
        )
        .map_err(|e| AppError::database(format!("Failed to cleanup old completed jobs: {e}")))?;

    Ok(deleted)
}

/// Retrieves aggregated counts for queue monitoring.
pub fn get_job_stats(conn: &Connection) -> Result<IndexJobStats, AppError> {
    let mut stats = IndexJobStats::default();

    let mut stmt = conn
        .prepare(
            "SELECT status, COUNT(*) 
             FROM index_jobs 
             GROUP BY status",
        )
        .map_err(|e| AppError::database(format!("Failed to prepare job stats query: {e}")))?;

    let rows = stmt
        .query_map([], |row| {
            let status: String = row.get(0)?;
            let count: usize = row.get(1)?;
            Ok((status, count))
        })
        .map_err(|e| AppError::database(format!("Failed to query job stats: {e}")))?;

    for item in rows {
        let (status, count) =
            item.map_err(|e| AppError::database(format!("Failed to read job stat row: {e}")))?;
        match status.as_str() {
            JOB_STATUS_PENDING => stats.pending = count,
            JOB_STATUS_PROCESSING => stats.processing = count,
            JOB_STATUS_SUCCEEDED => stats.succeeded = count,
            JOB_STATUS_FAILED => stats.failed = count,
            _ => {}
        }
        stats.total += count;
    }

    Ok(stats)
}

/// Resets all FAILED jobs back to PENDING with attempts = 0 for manual retry.
pub fn retry_all_failed_jobs(conn: &Connection) -> Result<usize, AppError> {
    let count = conn
        .execute(
            "UPDATE index_jobs
             SET status = 'PENDING',
                 attempts = 0,
                 available_at = datetime('now'),
                 lease_until = NULL,
                 last_error_code = NULL,
                 last_error_message = NULL,
                 updated_at = datetime('now')
             WHERE status = 'FAILED'",
            [],
        )
        .map_err(|e| AppError::database(format!("Failed to reset failed jobs: {e}")))?;

    log::info!("Reset {count} failed index job(s) back to PENDING for re-indexing");
    Ok(count)
}
