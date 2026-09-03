use crate::errors::AppError;
use rusqlite::Connection;

/// Migration entry: version number and SQL to execute.
struct Migration {
    version: u32,
    sql: &'static str,
}

/// All migrations in order. Append-only after release.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: "
            CREATE TABLE IF NOT EXISTS folders (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                path TEXT NOT NULL UNIQUE,
                enabled INTEGER NOT NULL DEFAULT 1,
                recursive INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                last_scanned_at TEXT
            );

            CREATE TABLE IF NOT EXISTS screenshots (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                filename TEXT NOT NULL,
                extension TEXT NOT NULL,
                file_size INTEGER NOT NULL,
                modified_at_fs TEXT NOT NULL,
                content_hash TEXT,
                width INTEGER,
                height INTEGER,
                ocr_text TEXT,
                ocr_status TEXT NOT NULL DEFAULT 'PENDING',
                ocr_engine TEXT,
                indexed_at TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now'))
            );

            CREATE UNIQUE INDEX IF NOT EXISTS idx_screenshots_path ON screenshots(path);
            CREATE INDEX IF NOT EXISTS idx_screenshots_folder_id ON screenshots(folder_id);
            CREATE INDEX IF NOT EXISTS idx_screenshots_ocr_status ON screenshots(ocr_status);
        ",
    },
    Migration {
        version: 2,
        sql: "
            CREATE VIRTUAL TABLE IF NOT EXISTS screenshots_fts USING fts5(
                filename,
                ocr_search_text,
                tokenize = 'unicode61 remove_diacritics 2'
            );
        ",
    },
    Migration {
        version: 3,
        sql: "
            CREATE TABLE IF NOT EXISTS index_jobs (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                folder_id INTEGER NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
                screenshot_id INTEGER REFERENCES screenshots(id) ON DELETE CASCADE,
                path TEXT NOT NULL,
                job_type TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'PENDING',
                dedupe_key TEXT NOT NULL UNIQUE,
                attempts INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 5,
                available_at TEXT NOT NULL DEFAULT (datetime('now')),
                lease_until TEXT,
                last_error_code TEXT,
                last_error_message TEXT,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                completed_at TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_index_jobs_status_available ON index_jobs(status, available_at);
            CREATE INDEX IF NOT EXISTS idx_index_jobs_folder_id ON index_jobs(folder_id);
            CREATE INDEX IF NOT EXISTS idx_index_jobs_screenshot_id ON index_jobs(screenshot_id);
        ",
    },
];

/// Backfills existing SUCCEEDED screenshots into the FTS5 index upon migration to v2.
fn backfill_fts_index(conn: &Connection) -> Result<(), AppError> {
    let mut stmt = conn
        .prepare(
            "SELECT id, filename, ocr_text 
             FROM screenshots 
             WHERE ocr_status = 'SUCCEEDED' AND ocr_text IS NOT NULL",
        )
        .map_err(|e| AppError::migration(format!("Failed to prepare backfill query: {e}")))?;

    let rows = stmt
        .query_map([], |row| {
            let id: i64 = row.get(0)?;
            let filename: String = row.get(1)?;
            let ocr_text: String = row.get(2)?;
            Ok((id, filename, ocr_text))
        })
        .map_err(|e| {
            AppError::migration(format!("Failed to query screenshots for backfill: {e}"))
        })?;

    let mut count = 0;
    for item in rows {
        let (id, filename, ocr_text) =
            item.map_err(|e| AppError::migration(format!("Failed to read backfill row: {e}")))?;
        let search_text = crate::search::normalize::normalize_search_text(&ocr_text);
        conn.execute(
            "INSERT OR REPLACE INTO screenshots_fts (rowid, filename, ocr_search_text) 
             VALUES (?1, ?2, ?3)",
            rusqlite::params![id, filename, search_text],
        )
        .map_err(|e| AppError::migration(format!("Failed to backfill screenshot {id}: {e}")))?;
        count += 1;
    }

    if count > 0 {
        log::info!("Backfilled {count} existing OCR screenshot(s) into screenshots_fts");
    }

    Ok(())
}

/// Run all pending migrations.
/// Uses a simple user_version-based mechanism.
pub fn run_migrations(conn: &Connection) -> Result<(), AppError> {
    let current_version: u32 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|e| AppError::migration(format!("Failed to read schema version: {e}")))?;

    let pending: Vec<&Migration> = MIGRATIONS
        .iter()
        .filter(|m| m.version > current_version)
        .collect();

    if pending.is_empty() {
        log::info!("Database schema is up to date (version {current_version})");
        return Ok(());
    }

    for migration in &pending {
        log::info!(
            "Running migration v{} (current: v{current_version})",
            migration.version
        );

        conn.execute_batch(migration.sql).map_err(|e| {
            AppError::migration(format!("Migration v{} failed: {e}", migration.version))
        })?;

        if migration.version == 2 {
            backfill_fts_index(conn)?;
        }

        conn.pragma_update(None, "user_version", migration.version)
            .map_err(|e| {
                AppError::migration(format!(
                    "Failed to update schema version to {}: {e}",
                    migration.version
                ))
            })?;

        log::info!("Migration v{} completed", migration.version);
    }

    Ok(())
}
