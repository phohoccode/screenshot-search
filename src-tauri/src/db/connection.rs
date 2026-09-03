use crate::errors::AppError;
use rusqlite::Connection;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use std::sync::Arc;

/// Thread-safe wrapper around the SQLite connection.
/// Tauri manages this as application state.
#[derive(Clone)]
pub struct Database {
    pub conn: Arc<Mutex<Connection>>,
}

/// Resolves the application data directory for ScreenshotSearch.
/// Uses the Tauri-provided app data dir path.
/// Creates the directory if it does not exist.
pub fn resolve_app_data_dir(app_data_dir: &PathBuf) -> Result<PathBuf, AppError> {
    if !app_data_dir.exists() {
        fs::create_dir_all(app_data_dir)
            .map_err(|e| AppError::database(format!("Failed to create app data directory: {e}")))?;
    }
    Ok(app_data_dir.clone())
}

/// Returns the path to the SQLite database file within the app data directory.
pub fn database_path(app_data_dir: &PathBuf) -> PathBuf {
    app_data_dir.join("database.sqlite")
}

/// Initializes the SQLite connection with recommended pragmas.
pub fn init_connection(db_path: &PathBuf) -> Result<Connection, AppError> {
    let conn = Connection::open(db_path).map_err(|e| {
        AppError::database(format!(
            "Failed to open database at {}: {e}",
            db_path.display()
        ))
    })?;

    // Set recommended pragmas for performance and safety
    conn.execute_batch(
        "PRAGMA journal_mode = WAL;
         PRAGMA synchronous = NORMAL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;",
    )
    .map_err(|e| AppError::database(format!("Failed to set database pragmas: {e}")))?;

    Ok(conn)
}

/// Creates a fully initialized Database instance.
/// Resolves paths, opens connection, runs migrations.
pub fn initialize(app_data_dir: &PathBuf) -> Result<Database, AppError> {
    let data_dir = resolve_app_data_dir(app_data_dir)?;
    let db_path = database_path(&data_dir);

    log::info!("Initializing database at: {}", db_path.display());

    let conn = init_connection(&db_path)?;
    super::migrations::run_migrations(&conn)?;

    Ok(Database {
        conn: Arc::new(Mutex::new(conn)),
    })
}
