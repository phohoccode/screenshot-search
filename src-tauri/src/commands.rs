use serde::Serialize;
use tauri::AppHandle;
use tauri::Manager;

use crate::db::connection::Database;
use crate::errors::CommandResult;

/// Basic application info returned to the frontend.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub version: String,
    pub data_dir: String,
}

/// Returns basic application information.
/// Thin command — delegates to app handle for data.
#[tauri::command]
pub fn get_app_info(app: AppHandle) -> CommandResult<AppInfo> {
    let version = app
        .config()
        .version
        .clone()
        .unwrap_or_else(|| "0.1.0".to_string());

    let data_dir = app
        .path()
        .app_data_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_default();

    Ok(AppInfo { version, data_dir })
}

/// Returns database health status — confirms DB is accessible.
#[tauri::command]
pub fn check_database(db: tauri::State<'_, Database>) -> CommandResult<bool> {
    let conn = db.conn.lock().map_err(|e| {
        crate::errors::AppError::database(format!("Failed to acquire database lock: {e}"))
    })?;

    let result: i32 = conn
        .query_row("SELECT 1", [], |row| row.get(0))
        .map_err(crate::errors::AppError::from)?;

    Ok(result == 1)
}
