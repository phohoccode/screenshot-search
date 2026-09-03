/** Application info returned from Rust backend */
export interface AppInfo {
  version: string;
  dataDir: string;
}

/** Managed folder model */
export interface Folder {
  id: number;
  path: string;
  enabled: boolean;
  recursive: boolean;
  createdAt: string;
  updatedAt: string;
  lastScannedAt: string | null;
  screenshotCount: number;
}

/** Result of a discovery scan on a folder */
export interface ScanSummary {
  folderId: number;
  discovered: number;
  added: number;
  updated: number;
  unchanged: number;
  removed: number;
  failed: number;
  durationMs: number;
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
  | "FOLDER_NOT_FOUND"
  | "FOLDER_ALREADY_EXISTS"
  | "FOLDER_PERMISSION_DENIED"
  | "FOLDER_SCAN_FAILED"
  | "FILE_METADATA_FAILED"
  | "FILE_HASH_FAILED"
  | "OCR_FAILED"
  | "INDEX_JOB_FAILED"
  | "SETTINGS_FAILED"
  | "UNKNOWN";
