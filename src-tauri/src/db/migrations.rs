use crate::errors::AppError;
use rusqlite::Connection;

/// Migration entry: version number and SQL to execute.
struct Migration {
    version: u32,
    sql: &'static str,
}

/// All migrations in order. Append-only after release.
const MIGRATIONS: &[Migration] = &[Migration {
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
}];

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
