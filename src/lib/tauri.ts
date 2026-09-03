import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Folder,
  ScanSummary,
  OcrStats,
  OcrBatchSummary,
  OcrProgressPayload,
  AppError,
} from "@/types";

/** Detects if the app is running inside a Tauri native window */
export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
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
