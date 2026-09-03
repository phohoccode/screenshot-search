use serde::Serialize;
use std::fmt;

/// Domain error codes exposed to the frontend via Tauri IPC.
/// These provide stable, typed error identifiers that the UI
/// can match on without parsing error message strings.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    DatabaseFailed,
    DatabaseMigrationFailed,
    FileNotFound,
    FilePermissionDenied,
    InvalidPath,
    FolderNotFound,
    FolderAlreadyExists,
    FolderPermissionDenied,
    FolderScanFailed,
    FileMetadataFailed,
    FileHashFailed,
    OcrFailed,
    IndexJobFailed,
    SettingsFailed,
    Unknown,
}

/// Application error type that maps to structured frontend-safe responses.
/// Never exposes raw stack traces or panics to the UI.
#[derive(Debug, Serialize)]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{:?}] {}", self.code, self.message)
    }
}

impl std::error::Error for AppError {}

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn database(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::DatabaseFailed, message)
    }

    pub fn migration(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::DatabaseMigrationFailed, message)
    }

    pub fn invalid_path(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidPath, message)
    }

    pub fn folder_not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::FolderNotFound, message)
    }

    pub fn folder_already_exists(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::FolderAlreadyExists, message)
    }

    pub fn scan_failed(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::FolderScanFailed, message)
    }
}

impl From<rusqlite::Error> for AppError {
    fn from(err: rusqlite::Error) -> Self {
        log::error!("Database error: {err}");
        Self::database(format!("Database operation failed: {err}"))
    }
}

/// Result type alias for Tauri commands.
/// Serializes the error as a structured JSON object to the frontend.
pub type CommandResult<T> = Result<T, AppError>;
