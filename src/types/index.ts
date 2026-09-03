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
  ocrSucceededCount: number;
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

/** Summary of an OCR indexing batch */
export interface OcrBatchSummary {
  totalCandidates: number;
  processed: number;
  succeeded: number;
  failed: number;
  durationMs: number;
}

/** Aggregated OCR status metrics */
export interface OcrStats {
  total: number;
  pending: number;
  processing: number;
  succeeded: number;
  failed: number;
}

/** Real-time progress payload emitted by backend */
export interface OcrProgressPayload {
  total: number;
  processed: number;
  succeeded: number;
  failed: number;
  isRunning: boolean;
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
  | "OCR_ENGINE_UNAVAILABLE"
  | "OCR_IMAGE_DECODE_FAILED"
  | "OCR_RECOGNITION_FAILED"
  | "OCR_MODEL_LOAD_FAILED"
  | "INDEX_JOB_FAILED"
  | "SETTINGS_FAILED"
  | "UNKNOWN";
