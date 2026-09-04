import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Folder,
  ScanSummary,
  OcrStats,
  OcrBatchSummary,
  OcrProgressPayload,
  OcrEngineInfo,
  SearchResultPage,
  SearchResultItem,
  ScreenshotDetail,
  SearchIndexHealth,
  IndexingServiceStatus,
  IndexingJobCompletedPayload,
  SemanticModelInfo,
  EmbeddingStats,
  OcrEngineDiagnostics,
  OcrEngineMode,
  OcrEngineStats,
  AppError,
} from "@/types";

/** Detects if the app is running inside a Tauri native window */
export function isTauri(): boolean {
  return (
    typeof window !== "undefined" &&
    ("__TAURI_INTERNALS__" in window ||
      "__TAURI__" in window ||
      Boolean((window as unknown as { isTauri?: boolean }).isTauri))
  );
}

// In-memory mock storage for pure web browser preview (npm run dev)
let mockFolders: Folder[] = [
  {
    id: 1,
    path: "C:\\Users\\User\\Pictures\\Screenshots",
    enabled: true,
    recursive: true,
    createdAt: new Date(Date.now() - 3600000).toISOString(),
    updatedAt: new Date(Date.now() - 3600000).toISOString(),
    lastScannedAt: new Date(Date.now() - 1800000).toISOString(),
    screenshotCount: 142,
    ocrSucceededCount: 120,
  },
];

let mockOcrStats: OcrStats = {
  total: 142,
  pending: 22,
  processing: 0,
  succeeded: 120,
  failed: 0,
};

/** Lists all registered screenshot folders */
export async function listFolders(): Promise<Folder[]> {
  if (isTauri()) {
    return await invoke<Folder[]>("list_folders");
  }
  return [...mockFolders];
}

/** Adds a new folder for indexing */
export async function addFolder(
  path: string,
  recursive = true
): Promise<Folder> {
  if (isTauri()) {
    return await invoke<Folder>("add_folder", { path, recursive });
  }

  // Web fallback simulation
  const normalized = path.replace(/\//g, "\\");
  if (
    mockFolders.some((f) => f.path.toLowerCase() === normalized.toLowerCase())
  ) {
    const error: AppError = {
      code: "FOLDER_ALREADY_EXISTS",
      message: `Folder is already registered: ${normalized}`,
    };
    throw error;
  }

  const newFolder: Folder = {
    id: Date.now(),
    path: normalized,
    enabled: true,
    recursive,
    createdAt: new Date().toISOString(),
    updatedAt: new Date().toISOString(),
    lastScannedAt: null,
    screenshotCount: 0,
    ocrSucceededCount: 0,
  };
  mockFolders.unshift(newFolder);
  return newFolder;
}

/** Removes a folder from management */
export async function removeFolder(id: number): Promise<boolean> {
  if (isTauri()) {
    return await invoke<boolean>("remove_folder", { id });
  }

  mockFolders = mockFolders.filter((f) => f.id !== id);
  return true;
}

/** Triggers a discovery scan on a folder */
export async function scanFolder(id: number): Promise<ScanSummary> {
  if (isTauri()) {
    return await invoke<ScanSummary>("scan_folder", { id });
  }

  // Web fallback simulation
  await new Promise((resolve) => setTimeout(resolve, 800));
  const folder = mockFolders.find((f) => f.id === id);
  if (folder) {
    folder.screenshotCount += 15;
    folder.lastScannedAt = new Date().toISOString();
    mockOcrStats.total += 15;
    mockOcrStats.pending += 15;
  }

  return {
    folderId: id,
    discovered: folder?.screenshotCount ?? 15,
    added: 15,
    updated: 0,
    unchanged: (folder?.screenshotCount ?? 15) - 15,
    removed: 0,
    failed: 0,
    durationMs: 780,
  };
}

/** Opens native OS directory picker dialog */
export async function pickFolder(): Promise<string | null> {
  if (isTauri()) {
    return await invoke<string | null>("pick_folder");
  }

  // Web browser fallback
  return window.prompt(
    "Browser preview: Enter folder path to test (e.g. D:\\Screenshots)",
    "D:\\Screenshots"
  );
}

/** Gets the total count of discovered screenshots across all folders */
export async function getTotalScreenshotCount(): Promise<number> {
  if (isTauri()) {
    return await invoke<number>("get_total_screenshot_count");
  }
  return mockFolders.reduce((sum, f) => sum + f.screenshotCount, 0);
}

/** Starts OCR indexing on pending screenshots */
export async function startOcrIndexing(
  folderId?: number,
  limit?: number
): Promise<OcrBatchSummary> {
  if (isTauri()) {
    return await invoke<OcrBatchSummary>("start_ocr_indexing", {
      folderId: folderId ?? null,
      limit: limit ?? null,
    });
  }

  // Web browser preview simulation
  mockOcrStats.processing = 1;
  await new Promise((resolve) => setTimeout(resolve, 1200));

  const count = mockOcrStats.pending;
  mockOcrStats.succeeded += count;
  mockOcrStats.pending = 0;
  mockOcrStats.processing = 0;

  for (const f of mockFolders) {
    f.ocrSucceededCount = f.screenshotCount;
  }

  return {
    totalCandidates: count,
    processed: count,
    succeeded: count,
    failed: 0,
    durationMs: 1200,
  };
}

/** Retrieves aggregated OCR metrics */
export async function getOcrStats(): Promise<OcrStats> {
  if (isTauri()) {
    return await invoke<OcrStats>("get_ocr_stats");
  }
  return { ...mockOcrStats };
}

/** Requests cancellation of the ongoing OCR indexing batch */
export async function cancelOcrIndexing(): Promise<boolean> {
  if (isTauri()) {
    return await invoke<boolean>("cancel_ocr_indexing");
  }
  mockOcrStats.processing = 0;
  return true;
}

/** Retrieves diagnostic information about the local OCR engine */
export async function getOcrEngineInfo(): Promise<OcrEngineInfo> {
  if (isTauri()) {
    return await invoke<OcrEngineInfo>("get_ocr_engine_info");
  }
  return {
    engineName: "windows_media_ocr",
    engineVersion: "winrt_v1",
    activeLanguage: "en-US",
    availableLanguages: ["en-US"],
    supportsVietnamese: false,
    maxImageDimension: 2600,
  };
}

/** Listens to real-time OCR progress updates */
export async function onOcrProgress(
  callback: (payload: OcrProgressPayload) => void
): Promise<UnlistenFn> {
  if (isTauri()) {
    return await listen<OcrProgressPayload>("ocr_progress", (event) => {
      callback(event.payload);
    });
  }
  // Web preview: no-op unlisten
  return () => {};
}

/** Local zero-network SVG placeholder for offline / browser preview fallback */
export function getLocalPlaceholderSvg(label: string): string {
  const safeText = label.replace(/[<>&"]/g, "");
  return `data:image/svg+xml;utf8,<svg xmlns="http://www.w3.org/2000/svg" width="600" height="400" viewBox="0 0 600 400"><rect width="100%" height="100%" fill="%2318181b"/><text x="50%" y="50%" fill="%23a1a1aa" font-family="sans-serif" font-size="14" text-anchor="middle" dominant-baseline="middle">${encodeURIComponent(safeText)}</text></svg>`;
}

/**
 * Generates the secure URL to render a screenshot.
 * Uses the database-backed custom protocol `http://screenshot.localhost/<id>`.
 * Appends `?v=<content_hash>` for instant image cache invalidation upon file modifications.
 * Does NOT require universal filesystem scopes or asset protocol wildcards.
 */
export function getScreenshotImageUrl(
  id: number,
  filePath?: string,
  contentHash?: string | null
): string {
  const v = contentHash ? `?v=${encodeURIComponent(contentHash)}` : "";
  if (isTauri()) {
    return `http://screenshot.localhost/${id}${v}`;
  }
  // Browser preview local SVG placeholder
  const name = filePath ? filePath.split(/[\\/]/).pop() : `Screenshot #${id}`;
  return getLocalPlaceholderSvg(name || "Screenshot");
}

/**
 * Converts a native filesystem path or screenshot id to a safe image URL.
 */
export function getFileAssetUrl(
  filePath: string,
  id?: number,
  contentHash?: string | null
): string {
  if (id !== undefined) {
    return getScreenshotImageUrl(id, filePath, contentHash);
  }
  if (isTauri()) {
    const normalized = filePath.replace(/\\/g, "/");
    return convertFileSrc(normalized);
  }
  const name = filePath.split(/[\\/]/).pop() || "Screenshot";
  return getLocalPlaceholderSvg(name);
}

/** Directly retrieves base64-encoded image data URL for a screenshot by ID */
export async function getScreenshotImageData(id: number): Promise<string> {
  if (isTauri()) {
    return await invoke<string>("get_screenshot_image", { id });
  }
  return getLocalPlaceholderSvg(`Screenshot #${id}`);
}

// Mock search results for pure browser preview mode
const mockSearchScreenshots: SearchResultItem[] = [
  {
    id: 101,
    folderId: 1,
    path: "C:\\Users\\User\\Pictures\\Screenshots\\terminal_error.png",
    filename: "terminal_error.png",
    modifiedAtFs: new Date(Date.now() - 7200000).toISOString(),
    width: 1920,
    height: 1080,
    matchSnippet: "PrismaClientKnownRequestError: Transaction already closed [[match]]P2028[[/match]]",
    score: 8.5,
  },
  {
    id: 102,
    folderId: 1,
    path: "C:\\Users\\User\\Pictures\\Screenshots\\npm_build_failure.png",
    filename: "npm_build_failure.png",
    modifiedAtFs: new Date(Date.now() - 14400000).toISOString(),
    width: 1920,
    height: 1080,
    matchSnippet: "npm ERR! code [[match]]ERR_MODULE_NOT_FOUND[[/match]] Cannot find module on [[match]]localhost:3000[[/match]]",
    score: 6.2,
  },
  {
    id: 103,
    folderId: 1,
    path: "C:\\Users\\User\\Pictures\\Screenshots\\invoice_september.png",
    filename: "invoice_september.png",
    modifiedAtFs: new Date(Date.now() - 86400000).toISOString(),
    width: 1200,
    height: 900,
    matchSnippet: "Invoice #2026-09 total $150.00 USD paid via Stripe [[match]]HTTP 500[[/match]] internal server error",
    score: 4.8,
  },
];

/** Searches screenshots using SQLite FTS5 with BM25 ranking */
export async function searchScreenshots(
  query: string,
  folderId?: number,
  limit?: number,
  offset?: number
): Promise<SearchResultPage> {
  if (isTauri()) {
    return await invoke<SearchResultPage>("search_screenshots", {
      query,
      folderId: folderId ?? null,
      limit: limit ?? null,
      offset: offset ?? null,
    });
  }

  // Web browser fallback simulation
  await new Promise((r) => setTimeout(r, 120));
  const q = query.toLowerCase().trim();
  if (!q) {
    return {
      items: mockSearchScreenshots,
      totalMatches: mockSearchScreenshots.length,
      hasMore: false,
    };
  }

  const filtered = mockSearchScreenshots.filter(
    (item) =>
      item.filename.toLowerCase().includes(q) ||
      (item.matchSnippet && item.matchSnippet.toLowerCase().includes(q))
  );

  return {
    items: filtered,
    totalMatches: filtered.length,
    hasMore: false,
  };
}

/** Retrieves complete detail of a single screenshot */
export async function getScreenshot(id: number): Promise<ScreenshotDetail> {
  if (isTauri()) {
    return await invoke<ScreenshotDetail>("get_screenshot", { id });
  }

  const mock = mockSearchScreenshots.find((s) => s.id === id) ?? mockSearchScreenshots[0];
  if (!mock) {
    throw new Error(`Screenshot ${id} not found`);
  }

  return {
    id: mock.id,
    folderId: mock.folderId,
    path: mock.path,
    filename: mock.filename,
    extension: "png",
    fileSize: 1024 * 512,
    modifiedAtFs: mock.modifiedAtFs,
    width: mock.width,
    height: mock.height,
    ocrText: mock.matchSnippet?.replace(/\[\[\/?match\]\]/g, "") || "Sample OCR text",
    ocrStatus: "SUCCEEDED",
    ocrEngine: "windows_media_ocr",
    indexedAt: new Date().toISOString(),
  };
}

/** Opens the screenshot using the default OS application */
export async function openScreenshot(id: number): Promise<boolean> {
  if (isTauri()) {
    return await invoke<boolean>("open_screenshot", { id });
  }
  alert(`Browser preview: opened screenshot #${id}`);
  return true;
}

/** Highlights the screenshot in Windows File Explorer */
export async function revealScreenshot(id: number): Promise<boolean> {
  if (isTauri()) {
    return await invoke<boolean>("reveal_screenshot", { id });
  }
  alert(`Browser preview: revealed screenshot #${id} in file explorer`);
  return true;
}

/** Rebuilds the search index */
export async function rebuildSearchIndex(): Promise<number> {
  if (isTauri()) {
    return await invoke<number>("rebuild_search_index");
  }
  return mockSearchScreenshots.length;
}

/** Checks search index health */
export async function checkSearchIndexHealth(): Promise<SearchIndexHealth> {
  if (isTauri()) {
    return await invoke<SearchIndexHealth>("check_search_index_health");
  }
  return {
    ftsCount: mockSearchScreenshots.length,
    succeededCount: mockSearchScreenshots.length,
    isHealthy: true,
  };
}

/** Retrieves status of the automatic background indexing service */
export async function getIndexingStatus(): Promise<IndexingServiceStatus> {
  if (isTauri()) {
    return await invoke<IndexingServiceStatus>("get_indexing_status");
  }
  return {
    isRunning: true,
    isPaused: false,
    activeWatchersCount: mockFolders.length,
    stats: {
      pending: mockOcrStats.pending,
      processing: mockOcrStats.processing,
      succeeded: mockOcrStats.succeeded,
      failed: mockOcrStats.failed,
      total: mockOcrStats.total,
    },
  };
}

/** Pauses background indexing */
export async function pauseIndexing(): Promise<void> {
  if (isTauri()) {
    await invoke<void>("pause_indexing");
  }
}

/** Resumes background indexing */
export async function resumeIndexing(): Promise<void> {
  if (isTauri()) {
    await invoke<void>("resume_indexing");
  }
}

/** Retries all failed index jobs */
export async function retryFailedIndexJobs(): Promise<number> {
  if (isTauri()) {
    return await invoke<number>("retry_failed_index_jobs");
  }
  return 0;
}

/** Subscribes to events when an indexing job finishes */
export async function onIndexingCompleted(
  callback: (payload: IndexingJobCompletedPayload) => void
): Promise<UnlistenFn> {
  if (isTauri()) {
    return await listen<IndexingJobCompletedPayload>(
      "indexing_job_completed",
      (event) => callback(event.payload)
    );
  }
  return () => {};
}

/** Retrieves status of the local semantic embedding model */
export async function getSemanticModelInfo(): Promise<SemanticModelInfo> {
  if (isTauri()) {
    return await invoke<SemanticModelInfo>("get_semantic_model_info");
  }
  return {
    modelId: "multilingual-e5-small",
    modelVersion: "v1",
    dimension: 384,
    status: { status: "ready" },
    isAvailable: true,
    approximateSizeMb: 135,
  };
}

/** Initiates user-requested download of the semantic embedding model */
export async function downloadSemanticModel(): Promise<boolean> {
  if (isTauri()) {
    return await invoke<boolean>("download_semantic_model");
  }
  return true;
}

/** Rebuilds the semantic embedding index for the active model without re-running OCR */
export async function rebuildSemanticIndex(): Promise<number> {
  if (isTauri()) {
    return await invoke<number>("rebuild_semantic_index");
  }
  return 0;
}

/** Retrieves aggregated metrics regarding semantic embedding coverage */
export async function getEmbeddingStats(): Promise<EmbeddingStats> {
  if (isTauri()) {
    return await invoke<EmbeddingStats>("get_embedding_stats");
  }
  return {
    totalSucceeded: 3,
    embeddedCount: 3,
    pendingCount: 0,
    activeModelId: "multilingual-e5-small",
    activeModelVersion: "v1",
  };
}

/** Subscribes to events when the semantic model completes downloading and initialization */
export async function onSemanticModelReady(
  callback: () => void
): Promise<UnlistenFn> {
  if (isTauri()) {
    return await listen("semantic_model_ready", () => callback());
  }
  return () => {};
}

/** Retrieves OCR router diagnostics including Windows language packs and multilingual fallback status */
export async function getOcrEngineDiagnostics(): Promise<OcrEngineDiagnostics> {
  if (isTauri()) {
    return await invoke<OcrEngineDiagnostics>("get_ocr_engine_diagnostics");
  }
  return {
    mode: "Auto",
    activeEngineName: "windows_media_ocr",
    windowsInfo: {
      engineName: "windows_media_ocr",
      engineVersion: "winrt_v1",
      activeLanguage: "en-US",
      availableLanguages: ["en-US"],
      supportsVietnamese: false,
      maxImageDimension: 2600,
    },
    multilingualInfo: {
      modelId: "multilingual-ocr",
      modelVersion: "ppocr_v4",
      status: { status: "notInstalled" },
      isAvailable: false,
      approximateSizeMb: 16,
    },
    windowsSupportsVietnamese: false,
    isMultilingualReady: false,
  };
}

/** Sets the active OCR Engine Router mode */
export async function setOcrEngineMode(mode: OcrEngineMode): Promise<void> {
  if (isTauri()) {
    await invoke("set_ocr_engine_mode", { mode });
  }
}

/** Triggers background download of the local multilingual OCR model */
export async function downloadMultilingualOcrModel(): Promise<void> {
  if (isTauri()) {
    await invoke("download_multilingual_ocr_model");
  }
}

/** Retrieves aggregate OCR engine diagnostic statistics */
export async function getOcrEngineStats(): Promise<OcrEngineStats> {
  if (isTauri()) {
    return await invoke<OcrEngineStats>("get_ocr_engine_stats");
  }
  return {
    totalSucceeded: 3,
    windowsCount: 3,
    multilingualCount: 0,
    outdatedPipelineCount: 0,
    failedCount: 0,
  };
}

/** Returns the count of screenshots eligible for re-OCR with an improved engine */
export async function getReOcrEligibleCount(): Promise<number> {
  if (isTauri()) {
    return await invoke<number>("get_re_ocr_eligible_count");
  }
  return 0;
}

/** Enqueues screenshots for background re-OCR with the improved OCR engine */
export async function reprocessScreenshotsWithImprovedOcr(
  limit?: number
): Promise<number> {
  if (isTauri()) {
    return await invoke<number>("reprocess_screenshots_with_improved_ocr", {
      limit,
    });
  }
  return 0;
}

/** Subscribes to events when the multilingual OCR model download status updates */
export async function onOcrModelStatusChanged(
  callback: () => void
): Promise<UnlistenFn> {
  if (isTauri()) {
    return await listen("ocr_model_status_changed", () => callback());
  }
  return () => {};
}


