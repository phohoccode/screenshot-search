/** Application info returned from Rust backend */
export interface AppInfo {
  version: string;
  dataDir: string;
}

/** Error response from Tauri commands */
export interface AppError {
  code: ErrorCode;
  message: string;
}

/** Stable error codes matching Rust ErrorCode enum */
export type ErrorCode =
  | "DATABASE_FAILED"
  | "DATABASE_MIGRATION_FAILED"
  | "FILE_NOT_FOUND"
  | "FILE_PERMISSION_DENIED"
  | "INVALID_PATH"
  | "OCR_FAILED"
  | "INDEX_JOB_FAILED"
  | "SETTINGS_FAILED"
  | "UNKNOWN";
