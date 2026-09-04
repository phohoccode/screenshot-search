import { useEffect, useState, useCallback, useRef } from "react";
import {
  ScanText,
  CheckCircle2,
  AlertCircle,
  Play,
  Pause,
  RotateCcw,
  Loader2,
  ShieldCheck,
  Languages,
  Maximize2,
  Sparkles,
  Download,
  RefreshCw,
  Cpu,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import {
  getOcrStats,
  getOcrEngineInfo,
  getIndexingStatus,
  pauseIndexing,
  resumeIndexing,
  retryFailedIndexJobs,
  onIndexingCompleted,
  getSemanticModelInfo,
  downloadSemanticModel,
  rebuildSemanticIndex,
  getEmbeddingStats,
  onSemanticModelReady,
  getOcrEngineDiagnostics,
  setOcrEngineMode,
  downloadMultilingualOcrModel,
  getReOcrEligibleCount,
  reprocessScreenshotsWithImprovedOcr,
  onOcrModelStatusChanged,
} from "@/lib/tauri";
import type {
  OcrStats,
  OcrEngineInfo,
  IndexingServiceStatus,
  SemanticModelInfo,
  EmbeddingStats,
  OcrEngineDiagnostics,
  OcrEngineMode,
} from "@/types";
import { OCR_ENGINE_MODE } from "@/types";

function getCommandErrorMessage(error: unknown, fallback: string): string {
  if (typeof error === "string") {
    try {
      const parsed: unknown = JSON.parse(error);
      if (parsed && typeof parsed === "object" && "message" in parsed) {
        const message = (parsed as { message?: unknown }).message;
        if (typeof message === "string") return message;
      }
    } catch {
      // Unstructured Tauri errors are intentionally hidden from normal UI.
    }
    return fallback;
  }

  if (error && typeof error === "object" && "message" in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === "string") return message;
  }

  return fallback;
}

export function IndexingPage() {
  const [ocrStats, setOcrStats] = useState<OcrStats | null>(null);
  const [engineInfo, setEngineInfo] = useState<OcrEngineInfo | null>(null);
  const [serviceStatus, setServiceStatus] = useState<IndexingServiceStatus | null>(null);
  const [modelInfo, setModelInfo] = useState<SemanticModelInfo | null>(null);
  const [embeddingStats, setEmbeddingStats] = useState<EmbeddingStats | null>(null);
  const [ocrDiagnostics, setOcrDiagnostics] = useState<OcrEngineDiagnostics | null>(null);
  const [reOcrEligibleCount, setReOcrEligibleCount] = useState<number>(0);

  const [isTogglingPause, setIsTogglingPause] = useState(false);
  const [isRetrying, setIsRetrying] = useState(false);
  const [isDownloadingModel, setIsDownloadingModel] = useState(false);
  const [isRebuildingEmbeddings, setIsRebuildingEmbeddings] = useState(false);
  const [isDownloadingOcrModel, setIsDownloadingOcrModel] = useState(false);
  const [isChangingEngineMode, setIsChangingEngineMode] = useState(false);
  const [isReprocessing, setIsReprocessing] = useState(false);
  const [isReprocessDialogOpen, setIsReprocessDialogOpen] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const pollingRef = useRef<number | null>(null);
  const refreshRequestRef = useRef(0);

  const refreshAll = useCallback(async () => {
    const requestId = ++refreshRequestRef.current;
    try {
      const [statsData, statusData, modelData, embData, ocrDiagData, eligibleData] =
        await Promise.all([
          getOcrStats(),
          getIndexingStatus().catch(() => null),
          getSemanticModelInfo().catch(() => null),
          getEmbeddingStats().catch(() => null),
          getOcrEngineDiagnostics().catch(() => null),
          getReOcrEligibleCount().catch(() => 0),
        ]);

      // A poll/event refresh can overlap a mode switch refresh. Only the newest
      // response may update diagnostics, keeping the backend-confirmed mode visible.
      if (requestId !== refreshRequestRef.current) return;

      setOcrStats(statsData);
      if (statusData) {
        setServiceStatus(statusData);
      }
      if (modelData) {
        setModelInfo(modelData);
        if (modelData.isAvailable) {
          setIsDownloadingModel(false);
        }
      }
      if (embData) {
        setEmbeddingStats(embData);
      }
      if (ocrDiagData) {
        setOcrDiagnostics(ocrDiagData);
        if (ocrDiagData.isMultilingualReady) {
          setIsDownloadingOcrModel(false);
        }
      }
      setReOcrEligibleCount(eligibleData ?? 0);
    } catch (err) {
      console.error("Failed to load indexing stats", err);
    }
  }, []);

  useEffect(() => {
    refreshAll();
    getOcrEngineInfo().then(setEngineInfo).catch(console.error);

    // Listen to background indexing completion events
    let unlistenIndexing: (() => void) | undefined;
    onIndexingCompleted(() => {
      refreshAll();
    }).then((fn) => {
      unlistenIndexing = fn;
    });

    // Listen to semantic model download readiness events
    let unlistenSemantic: (() => void) | undefined;
    onSemanticModelReady(() => {
      refreshAll();
    }).then((fn) => {
      unlistenSemantic = fn;
    });

    // Listen to multilingual OCR model status updates
    let unlistenOcrModel: (() => void) | undefined;
    onOcrModelStatusChanged(() => {
      refreshAll();
    }).then((fn) => {
      unlistenOcrModel = fn;
    });

    // Periodic lightweight refresh every 3 seconds to keep metrics fresh
    pollingRef.current = window.setInterval(refreshAll, 3000);

    return () => {
      if (unlistenIndexing) unlistenIndexing();
      if (unlistenSemantic) unlistenSemantic();
      if (unlistenOcrModel) unlistenOcrModel();
      if (pollingRef.current) clearInterval(pollingRef.current);
    };
  }, [refreshAll]);

  const handleTogglePause = async () => {
    if (!serviceStatus) return;
    setIsTogglingPause(true);
    setErrorMessage(null);
    try {
      if (serviceStatus.isPaused) {
        await resumeIndexing();
      } else {
        await pauseIndexing();
      }
      await refreshAll();
    } catch (err: unknown) {
      const msg =
        err && typeof err === "object" && "message" in err
          ? (err as { message: string }).message
          : "Failed to toggle pause state";
      setErrorMessage(msg);
    } finally {
      setIsTogglingPause(false);
    }
  };

  const handleRetryFailed = async () => {
    setIsRetrying(true);
    setErrorMessage(null);
    try {
      await retryFailedIndexJobs();
      await refreshAll();
    } catch (err: unknown) {
      const msg =
        err && typeof err === "object" && "message" in err
          ? (err as { message: string }).message
          : "Failed to retry failed jobs";
      setErrorMessage(msg);
    } finally {
      setIsRetrying(false);
    }
  };

  const handleDownloadModel = async () => {
    setIsDownloadingModel(true);
    setErrorMessage(null);
    try {
      await downloadSemanticModel();
      await refreshAll();
    } catch (err: unknown) {
      const msg =
        err && typeof err === "object" && "message" in err
          ? (err as { message: string }).message
          : "Failed to initiate model download";
      setErrorMessage(msg);
      setIsDownloadingModel(false);
    }
  };

  const handleRebuildEmbeddings = async () => {
    setIsRebuildingEmbeddings(true);
    setErrorMessage(null);
    try {
      await rebuildSemanticIndex();
      await refreshAll();
    } catch (err: unknown) {
      const msg =
        err && typeof err === "object" && "message" in err
          ? (err as { message: string }).message
          : "Failed to rebuild semantic embeddings";
      setErrorMessage(msg);
    } finally {
      setIsRebuildingEmbeddings(false);
    }
  };

  const handleSetEngineMode = async (mode: OcrEngineMode) => {
    const previousMode = ocrDiagnostics?.mode ?? OCR_ENGINE_MODE.AUTO;
    setErrorMessage(null);
    if (mode === previousMode) return;

    setIsChangingEngineMode(true);
    try {
      await setOcrEngineMode(mode);
      // The diagnostics response remains the source of truth; do not optimistically
      // update the selected mode before the backend confirms the switch.
      await refreshAll();
    } catch (err: unknown) {
      setErrorMessage(getCommandErrorMessage(err, "Failed to set OCR engine mode"));
    } finally {
      setIsChangingEngineMode(false);
    }
  };

  const handleDownloadOcrModel = async () => {
    setIsDownloadingOcrModel(true);
    setErrorMessage(null);
    try {
      await downloadMultilingualOcrModel();
      await refreshAll();
    } catch (err: unknown) {
      const msg =
        err && typeof err === "object" && "message" in err
          ? (err as { message: string }).message
          : "Failed to start multilingual OCR model download";
      setErrorMessage(msg);
      setIsDownloadingOcrModel(false);
    }
  };

  const handleReprocessOcr = async () => {
    setIsReprocessing(true);
    setIsReprocessDialogOpen(false);
    setErrorMessage(null);
    try {
      await reprocessScreenshotsWithImprovedOcr();
      await refreshAll();
    } catch (err: unknown) {
      const msg =
        err && typeof err === "object" && "message" in err
          ? (err as { message: string }).message
          : "Failed to queue screenshots for re-OCR";
      setErrorMessage(msg);
    } finally {
      setIsReprocessing(false);
    }
  };

  const activeMode = ocrDiagnostics?.mode ?? OCR_ENGINE_MODE.AUTO;
  const isMultilingualReady = ocrDiagnostics?.isMultilingualReady ?? false;
  const isMultilingualDownloading =
    isDownloadingOcrModel ||
    ocrDiagnostics?.multilingualInfo.status.status === "downloading";

  // Metric computations
  const total = ocrStats?.total ?? 0;
  const succeeded = ocrStats?.succeeded ?? 0;
  const pending = ocrStats?.pending ?? 0;
  const failed = ocrStats?.failed ?? 0;
  const isPaused = serviceStatus?.isPaused ?? false;
  const isActivelyIndexing = pending > 0 && !isPaused;
  const percent = total > 0 ? Math.min(100, Math.round((succeeded / total) * 100)) : 0;

  const embeddedCount = embeddingStats?.embeddedCount ?? 0;
  const pendingEmbeddings = embeddingStats?.pendingCount ?? 0;
  const totalPendingAll = pending + pendingEmbeddings;
  const semanticPercent =
    succeeded > 0 ? Math.min(100, Math.round((embeddedCount / succeeded) * 100)) : 0;

  const isModelReady = modelInfo?.isAvailable ?? false;
  const isModelDownloading =
    isDownloadingModel || modelInfo?.status.status === "downloading";

  return (
    <div className="flex h-full flex-col">
      <header className="border-b border-border px-6 py-4 flex items-center justify-between">
        <div>
          <h1 className="text-base font-semibold text-foreground">Background Indexing</h1>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Automated filesystem watcher, local OCR, and hybrid semantic vector index.
          </p>
        </div>

        {/* Global Pipeline Badge */}
        <div>
          {isPaused ? (
            <span className="inline-flex items-center gap-1.5 rounded-full bg-amber-500/10 px-2.5 py-1 text-xs font-medium text-amber-600 dark:text-amber-400 border border-amber-500/20">
              <Pause className="h-3 w-3" />
              Paused
            </span>
          ) : isActivelyIndexing || pendingEmbeddings > 0 ? (
            <span className="inline-flex items-center gap-1.5 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary border border-primary/20">
              <Loader2 className="h-3 w-3 animate-spin" />
              Indexing {totalPendingAll} item{totalPendingAll === 1 ? "" : "s"}
            </span>
          ) : (
            <span className="inline-flex items-center gap-1.5 rounded-full bg-emerald-500/10 px-2.5 py-1 text-xs font-medium text-emerald-600 dark:text-emerald-400 border border-emerald-500/20">
              <CheckCircle2 className="h-3 w-3" />
              Up to date
            </span>
          )}
        </div>
      </header>

      <div className="flex-1 overflow-y-auto px-6 py-6">
        <div className="mx-auto max-w-2xl space-y-5">
          {/* Main Indexing Card */}
          <div className="rounded-lg border border-border bg-card p-6 shadow-xs">
            <div className="flex items-center justify-between pb-4 border-b border-border">
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-muted text-primary">
                  <ScanText className="h-5 w-5" />
                </div>
                <div>
                  <h2 className="text-sm font-semibold text-foreground">
                    Continuous Local Pipeline
                  </h2>
                  <p className="text-xs text-muted-foreground">
                    Screenshots added to watched folders index automatically without manual intervention
                  </p>
                </div>
              </div>
            </div>

            {/* Progress Section */}
            <div className="mt-5 space-y-2">
              <div className="flex items-center justify-between text-xs">
                <span className="font-medium text-foreground">
                  {succeeded.toLocaleString()} / {total.toLocaleString()} searchable
                </span>
                <span className="text-muted-foreground font-mono">{percent}%</span>
              </div>
              <Progress value={percent} className="h-2" />
            </div>

            {/* Compact Metric Chips */}
            <div className="mt-5 grid grid-cols-4 gap-3">
              <div className="rounded-md border border-border bg-muted/40 p-2.5 text-center">
                <div className="text-xs text-muted-foreground">Total</div>
                <div className="mt-0.5 text-base font-semibold text-foreground">
                  {total.toLocaleString()}
                </div>
              </div>

              <div className="rounded-md border border-border bg-muted/40 p-2.5 text-center">
                <div className="text-xs text-muted-foreground">Searchable</div>
                <div className="mt-0.5 text-base font-semibold text-emerald-600 dark:text-emerald-400">
                  {succeeded.toLocaleString()}
                </div>
              </div>

              <div className="rounded-md border border-border bg-muted/40 p-2.5 text-center">
                <div className="text-xs text-muted-foreground">Semantic Ready</div>
                <div className="mt-0.5 text-base font-semibold text-foreground">
                  {embeddedCount.toLocaleString()}
                </div>
              </div>

              <div className="rounded-md border border-border bg-muted/40 p-2.5 text-center">
                <div className="text-xs text-muted-foreground">Pending</div>
                <div className="mt-0.5 text-base font-semibold text-foreground">
                  {totalPendingAll.toLocaleString()}
                </div>
              </div>
            </div>

            {/* Failed Notice */}
            {failed > 0 && (
              <div className="mt-3 flex items-center justify-between rounded-md border border-rose-200 bg-rose-50/50 p-2.5 text-xs text-rose-700 dark:border-rose-900/40 dark:bg-rose-950/20 dark:text-rose-400">
                <div className="flex items-center gap-2">
                  <AlertCircle className="h-4 w-4 shrink-0" />
                  <span>{failed} screenshot(s) failed OCR indexing</span>
                </div>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleRetryFailed}
                  disabled={isRetrying}
                  className="h-7 gap-1 text-xs border-rose-300 dark:border-rose-800"
                >
                  <RotateCcw className={`h-3 w-3 ${isRetrying ? "animate-spin" : ""}`} />
                  {isRetrying ? "Retrying..." : "Retry"}
                </Button>
              </div>
            )}

            {/* Error Message */}
            {errorMessage && (
              <div className="mt-4 flex items-center gap-2 rounded-md border border-rose-200 bg-rose-50 p-3 text-xs text-rose-700 dark:border-rose-900/50 dark:bg-rose-950/30 dark:text-rose-400">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <span>{errorMessage}</span>
              </div>
            )}

            {/* Pipeline Controls */}
            <div className="mt-6 flex items-center justify-between pt-4 border-t border-border">
              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                <Sparkles className="h-3.5 w-3.5 text-primary" />
                <span>Watcher active on {serviceStatus?.activeWatchersCount ?? 1} folder(s)</span>
              </div>

              <div className="flex items-center gap-2">
                <Button
                  variant={isPaused ? "default" : "outline"}
                  size="sm"
                  onClick={handleTogglePause}
                  disabled={isTogglingPause}
                  className="gap-1.5"
                >
                  {isTogglingPause ? (
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  ) : isPaused ? (
                    <>
                      <Play className="h-3.5 w-3.5" />
                      Resume Indexing
                    </>
                  ) : (
                    <>
                      <Pause className="h-3.5 w-3.5" />
                      Pause Indexing
                    </>
                  )}
                </Button>
              </div>
            </div>
          </div>

          {/* Phase 3.5: OCR Engine Router & Vietnamese Quality Card */}
          <div className="rounded-lg border border-border bg-card p-6 shadow-xs">
            <div className="flex items-center justify-between pb-4 border-b border-border">
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-muted text-primary">
                  <Languages className="h-5 w-5" />
                </div>
                <div>
                  <h2 className="text-sm font-semibold text-foreground">
                    OCR Engine &amp; Vietnamese Recognition
                  </h2>
                  <p className="text-xs text-muted-foreground">
                    Uses Windows OCR for technical text and a local Vietnamese model for natural text.
                  </p>
                </div>
              </div>

              <div className="flex items-center gap-1 bg-muted p-0.5 rounded-lg border border-border">
                <button
                  type="button"
                  onClick={() => handleSetEngineMode(OCR_ENGINE_MODE.AUTO)}
                  disabled={isChangingEngineMode}
                  className={`px-2.5 py-1 text-xs font-medium rounded-md transition-colors ${
                    activeMode === OCR_ENGINE_MODE.AUTO
                      ? "bg-background text-foreground shadow-xs"
                      : "text-muted-foreground hover:text-foreground"
                  }`}
                >
                  Auto (Recommended)
                </button>
                <button
                  type="button"
                  onClick={() => handleSetEngineMode(OCR_ENGINE_MODE.WINDOWS_NATIVE)}
                  disabled={isChangingEngineMode}
                  className={`px-2.5 py-1 text-xs font-medium rounded-md transition-colors ${
                    activeMode === OCR_ENGINE_MODE.WINDOWS_NATIVE
                      ? "bg-background text-foreground shadow-xs"
                      : "text-muted-foreground hover:text-foreground"
                  }`}
                >
                  Windows Native
                </button>
                <button
                  type="button"
                  onClick={() => handleSetEngineMode(OCR_ENGINE_MODE.HYBRID_VIETNAMESE)}
                  disabled={isChangingEngineMode}
                  className={`px-2.5 py-1 text-xs font-medium rounded-md transition-colors ${
                    activeMode === OCR_ENGINE_MODE.HYBRID_VIETNAMESE
                      ? "bg-background text-foreground shadow-xs"
                      : "text-muted-foreground hover:text-foreground"
                  }`}
                >
                  Hybrid Vietnamese
                </button>
              </div>
            </div>

            {/* Language & Engine Diagnostics */}
            <div className="mt-5 space-y-3">
              <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 text-xs">
                <div className="flex items-center justify-between bg-muted/30 p-2.5 rounded border border-border/50">
                  <span className="text-muted-foreground">Windows vi-VN OCR:</span>
                  {ocrDiagnostics?.windowsSupportsVietnamese ? (
                    <span className="font-medium text-emerald-600 dark:text-emerald-400">
                      Installed
                    </span>
                  ) : (
                    <span className="text-muted-foreground">Not installed in Windows</span>
                  )}
                </div>

                <div className="flex items-center justify-between bg-muted/30 p-2.5 rounded border border-border/50">
                  <span className="text-muted-foreground">Vietnamese OCR:</span>
                  {isMultilingualReady ? (
                    <span className="font-medium text-emerald-600 dark:text-emerald-400">
                      Ready — Hybrid local OCR
                    </span>
                  ) : isMultilingualDownloading ? (
                    <span className="font-medium text-primary flex items-center gap-1">
                      <Loader2 className="h-3 w-3 animate-spin" />
                      Downloading
                    </span>
                  ) : (
                    <span className="text-muted-foreground">Enhanced model not installed</span>
                  )}
                </div>
              </div>

              {!isMultilingualReady && (
                <div className="flex items-center justify-between bg-primary/5 border border-primary/20 rounded-md p-3 text-xs">
                  <span className="text-muted-foreground">
                    Enhanced model not installed. Runs locally. Screenshots are never uploaded (~15 MB).
                  </span>
                  <Button
                    size="sm"
                    onClick={handleDownloadOcrModel}
                    disabled={isMultilingualDownloading}
                    className="h-7 gap-1.5 text-xs shrink-0 ml-3"
                  >
                    {isMultilingualDownloading ? (
                      <Loader2 className="h-3 w-3 animate-spin" />
                    ) : (
                      <Download className="h-3 w-3" />
                    )}
                    Download Vietnamese OCR Model
                  </Button>
                </div>
              )}

              {/* Reprocess / Re-OCR Section */}
              <div className="pt-3 border-t border-border flex items-center justify-between text-xs">
                <div>
                  <span className="text-muted-foreground">
                    Re-OCR eligible:{" "}
                    <strong className="text-foreground font-mono font-medium">
                      {reOcrEligibleCount}
                    </strong>{" "}
                    screenshot(s)
                  </span>
                </div>

                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => setIsReprocessDialogOpen(true)}
                  disabled={reOcrEligibleCount === 0 || isReprocessing}
                  className="h-7 gap-1.5 text-xs"
                >
                  <RefreshCw className={`h-3 w-3 ${isReprocessing ? "animate-spin" : ""}`} />
                  {isReprocessing ? "Queueing..." : "Reprocess with improved OCR"}
                </Button>
              </div>
            </div>
          </div>

          {/* Re-OCR Confirmation Dialog */}
          <AlertDialog open={isReprocessDialogOpen} onOpenChange={setIsReprocessDialogOpen}>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>Reprocess with improved OCR</AlertDialogTitle>
                <AlertDialogDescription>
                  This will re-run OCR for existing screenshots using Hybrid OCR.
                  Search text and semantic indexes will be refreshed.
                  Original screenshots will not be modified.
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>Cancel</AlertDialogCancel>
                <AlertDialogAction onClick={handleReprocessOcr}>
                  Start Re-processing
                </AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>

          {/* Phase 3: Semantic Embeddings Card */}
          <div className="rounded-lg border border-border bg-card p-6 shadow-xs">
            <div className="flex items-center justify-between pb-4 border-b border-border">
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-muted text-primary">
                  <Sparkles className="h-5 w-5" />
                </div>
                <div>
                  <h2 className="text-sm font-semibold text-foreground">
                    Local Semantic Search (AI Embeddings)
                  </h2>
                  <p className="text-xs text-muted-foreground">
                    Natural-language understanding running 100% locally on CPU without external APIs
                  </p>
                </div>
              </div>

              <div>
                {isModelReady ? (
                  <span className="inline-flex items-center gap-1.5 rounded-full bg-emerald-500/10 px-2.5 py-1 text-xs font-medium text-emerald-600 dark:text-emerald-400 border border-emerald-500/20">
                    <CheckCircle2 className="h-3 w-3" />
                    Ready
                  </span>
                ) : isModelDownloading ? (
                  <span className="inline-flex items-center gap-1.5 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary border border-primary/20">
                    <Loader2 className="h-3 w-3 animate-spin" />
                    Downloading
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1.5 rounded-full bg-muted px-2.5 py-1 text-xs font-medium text-muted-foreground border border-border">
                    Not Installed
                  </span>
                )}
              </div>
            </div>

            {!isModelReady ? (
              /* Model Download Callout */
              <div className="mt-5 space-y-4">
                <p className="text-xs leading-relaxed text-muted-foreground">
                  To search screenshots conceptually (e.g. searching <em>"lỗi database"</em> to find a <em>PrismaClientKnownRequestError</em>), download the compact multilingual model (~{modelInfo?.approximateSizeMb ?? 135} MB). It runs entirely on your CPU with zero network inference.
                </p>
                <div className="flex items-center gap-3">
                  <Button
                    size="sm"
                    onClick={handleDownloadModel}
                    disabled={isModelDownloading}
                    className="gap-2 text-xs"
                  >
                    {isModelDownloading ? (
                      <>
                        <Loader2 className="h-3.5 w-3.5 animate-spin" />
                        Downloading model files...
                      </>
                    ) : (
                      <>
                        <Download className="h-3.5 w-3.5" />
                        Download Model (~{modelInfo?.approximateSizeMb ?? 135} MB)
                      </>
                    )}
                  </Button>
                </div>
              </div>
            ) : (
              /* Model Installed & Ready */
              <div className="mt-5 space-y-4">
                <div className="space-y-2">
                  <div className="flex items-center justify-between text-xs">
                    <span className="font-medium text-foreground">
                      {embeddedCount.toLocaleString()} / {succeeded.toLocaleString()} screenshots vectorized
                    </span>
                    <span className="text-muted-foreground font-mono">{semanticPercent}%</span>
                  </div>
                  <Progress value={semanticPercent} className="h-2" />
                </div>

                <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 text-xs text-muted-foreground">
                  <div className="flex items-center justify-between bg-muted/30 p-2.5 rounded">
                    <span>Model Architecture:</span>
                    <span className="font-mono text-foreground font-medium">
                      multilingual-e5-small (384-d)
                    </span>
                  </div>
                  <div className="flex items-center justify-between bg-muted/30 p-2.5 rounded">
                    <span>Local Runtime:</span>
                    <span className="font-medium text-foreground flex items-center gap-1">
                      <Cpu className="h-3 w-3 text-primary" />
                      CPU In-Process
                    </span>
                  </div>
                </div>

                <div className="flex items-center justify-between pt-3 border-t border-border text-xs">
                  <span className="text-muted-foreground">
                    Vectors update automatically after OCR completes.
                  </span>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleRebuildEmbeddings}
                    disabled={isRebuildingEmbeddings}
                    className="h-7 gap-1.5 text-xs"
                  >
                    <RefreshCw className={`h-3 w-3 ${isRebuildingEmbeddings ? "animate-spin" : ""}`} />
                    {isRebuildingEmbeddings ? "Rebuilding..." : "Rebuild Embeddings"}
                  </Button>
                </div>
              </div>
            )}
          </div>

          {/* Engine Diagnostics & Language Support */}
          {engineInfo && (
            <div className="rounded-lg border border-border bg-card p-4 space-y-3 text-xs">
              <div className="flex items-center justify-between border-b border-border pb-2.5">
                <div className="flex items-center gap-2 text-foreground font-medium">
                  <Languages className="h-4 w-4 text-primary" />
                  <span>OCR Engine &amp; Language Diagnostics</span>
                </div>
                <span className="font-mono text-[11px] text-muted-foreground">
                  {engineInfo.engineName} ({engineInfo.engineVersion})
                </span>
              </div>

              <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 text-muted-foreground">
                <div className="flex items-center justify-between bg-muted/30 p-2 rounded">
                  <span>Active Recognizer:</span>
                  <span className="font-medium text-foreground font-mono">
                    {engineInfo.activeLanguage}
                  </span>
                </div>

                <div className="flex items-center justify-between bg-muted/30 p-2 rounded">
                  <span>Vietnamese (vi-VN):</span>
                  {engineInfo.supportsVietnamese ? (
                    <span className="font-medium text-emerald-600 dark:text-emerald-400">
                      Installed
                    </span>
                  ) : (
                    <span className="text-muted-foreground">
                      Not installed in Windows
                    </span>
                  )}
                </div>
              </div>

              <div className="flex items-center gap-2 text-[11px] text-muted-foreground bg-muted/20 p-2 rounded">
                <Maximize2 className="h-3.5 w-3.5 shrink-0 text-primary" />
                <span>
                  Max Dimension: {engineInfo.maxImageDimension}px. 4K and ultra-wide screenshots are automatically downscaled or tiled maintaining aspect ratio.
                </span>
              </div>
            </div>
          )}

          {/* Privacy & Zero-Cloud Guarantee */}
          <div className="rounded-lg border border-border bg-muted/20 p-4">
            <div className="flex items-start gap-3">
              <ShieldCheck className="h-5 w-5 text-emerald-600 dark:text-emerald-400 shrink-0 mt-0.5" />
              <div className="space-y-1">
                <h3 className="text-xs font-semibold text-foreground">
                  Zero Cloud Transmission &amp; Local Execution
                </h3>
                <p className="text-xs leading-relaxed text-muted-foreground">
                  OCR recognition, vector generation, and search indexing run 100% on your local machine.
                  No screenshot images, extracted text, embeddings, or queries are ever sent to the cloud.
                </p>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
