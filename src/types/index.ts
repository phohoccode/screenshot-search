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

/** Diagnostic info about the active local OCR engine and language support */
export interface OcrEngineInfo {
  engineName: string;
  engineVersion: string;
  activeLanguage: string;
  availableLanguages: string[];
  supportsVietnamese: boolean;
  maxImageDimension: number;
}

export interface SearchResultItem {
  id: number;
  folderId: number;
  path: string;
  filename: string;
  modifiedAtFs: string;
  contentHash?: string | null;
  width?: number | null;
  height?: number | null;
  matchSnippet?: string | null;
  score: number;
  matchType?: "exact" | "keyword" | "semantic" | "hybrid" | null;
}

/** Aggregated metrics for semantic embedding coverage */
export interface EmbeddingStats {
  totalSucceeded: number;
  embeddedCount: number;
  pendingCount: number;
  activeModelId: string;
  activeModelVersion: string;
}

export type SemanticModelStatus =
  | { status: "notInstalled" }
  | { status: "downloading"; percent: number }
  | { status: "ready" }
  | { status: "error"; message: string };

/** Diagnostic info about local semantic embedding model */
export interface SemanticModelInfo {
  modelId: string;
  modelVersion: string;
  dimension: number;
  status: SemanticModelStatus;
  isAvailable: boolean;
  approximateSizeMb: number;
}

/** Paginated search response */
export interface SearchResultPage {
  items: SearchResultItem[];
  totalMatches: number;
  hasMore: boolean;
}

/** Complete detail of a single screenshot */
export interface ScreenshotDetail {
  id: number;
  folderId: number;
  path: string;
  filename: string;
  extension: string;
  fileSize: number;
  modifiedAtFs: string;
  contentHash?: string | null;
  width?: number | null;
  height?: number | null;
  ocrText?: string | null;
  ocrStatus: string;
  ocrEngine?: string | null;
  indexedAt?: string | null;
}

/** Durable background queue diagnostic metrics */
export interface IndexJobStats {
  pending: number;
  processing: number;
  succeeded: number;
  failed: number;
  total: number;
}

/** High-level status of the automatic background indexing service */
export interface IndexingServiceStatus {
  isRunning: boolean;
  isPaused: boolean;
  activeWatchersCount: number;
  stats: IndexJobStats;
  semanticStats?: EmbeddingStats | null;
}

export interface IndexingJobCompletedPayload {
  jobId: number;
  path: string;
}

/** Search index health diagnostic metrics */
export interface SearchIndexHealth {
  ftsCount: number;
  succeededCount: number;
  isHealthy: boolean;
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
