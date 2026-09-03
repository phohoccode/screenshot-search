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
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import {
  getOcrStats,
  getOcrEngineInfo,
  getIndexingStatus,
  pauseIndexing,
  resumeIndexing,
  retryFailedIndexJobs,
  onIndexingCompleted,
} from "@/lib/tauri";
import type {
  OcrStats,
  OcrEngineInfo,
  IndexingServiceStatus,
} from "@/types";

export function IndexingPage() {
  const [ocrStats, setOcrStats] = useState<OcrStats | null>(null);
  const [engineInfo, setEngineInfo] = useState<OcrEngineInfo | null>(null);
  const [serviceStatus, setServiceStatus] = useState<IndexingServiceStatus | null>(null);
  const [isTogglingPause, setIsTogglingPause] = useState(false);
  const [isRetrying, setIsRetrying] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const pollingRef = useRef<number | null>(null);

  const refreshAll = useCallback(async () => {
    try {
      const [statsData, statusData] = await Promise.all([
        getOcrStats(),
        getIndexingStatus().catch(() => null),
      ]);
      setOcrStats(statsData);
      if (statusData) {
        setServiceStatus(statusData);
      }
    } catch (err) {
      console.error("Failed to load indexing stats", err);
    }
  }, []);

  useEffect(() => {
    refreshAll();
    getOcrEngineInfo().then(setEngineInfo).catch(console.error);

    // Listen to background indexing completion events
    let unlisten: (() => void) | undefined;
    onIndexingCompleted(() => {
      refreshAll();
    }).then((fn) => {
      unlisten = fn;
    });

    // Periodic lightweight refresh every 3 seconds to keep metrics fresh
    pollingRef.current = window.setInterval(refreshAll, 3000);

    return () => {
      if (unlisten) unlisten();
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

  // Metric computations
  const total = ocrStats?.total ?? 0;
  const succeeded = ocrStats?.succeeded ?? 0;
  const pending = ocrStats?.pending ?? 0;
  const failed = ocrStats?.failed ?? 0;
  const isPaused = serviceStatus?.isPaused ?? false;
  const isActivelyIndexing = pending > 0 && !isPaused;
  const percent = total > 0 ? Math.min(100, Math.round((succeeded / total) * 100)) : 0;

  return (
    <div className="flex h-full flex-col">
      <header className="border-b border-border px-6 py-4 flex items-center justify-between">
        <div>
          <h1 className="text-base font-semibold text-foreground">Background Indexing</h1>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Automated filesystem watcher, local OCR, and SQLite FTS5 index.
          </p>
        </div>

        {/* Global Pipeline Badge */}
        <div>
          {isPaused ? (
            <span className="inline-flex items-center gap-1.5 rounded-full bg-amber-500/10 px-2.5 py-1 text-xs font-medium text-amber-600 dark:text-amber-400 border border-amber-500/20">
              <Pause className="h-3 w-3" />
              Paused
            </span>
          ) : isActivelyIndexing ? (
            <span className="inline-flex items-center gap-1.5 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary border border-primary/20">
              <Loader2 className="h-3 w-3 animate-spin" />
              Indexing {pending} new
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

            {/* Metric Chips */}
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
                <div className="text-xs text-muted-foreground">Pending</div>
                <div className="mt-0.5 text-base font-semibold text-foreground">
                  {pending.toLocaleString()}
                </div>
              </div>

              <div className="rounded-md border border-border bg-muted/40 p-2.5 text-center">
                <div className="text-xs text-muted-foreground">Failed</div>
                <div className="mt-0.5 text-base font-semibold text-rose-500">
                  {failed.toLocaleString()}
                </div>
              </div>
            </div>

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
                {failed > 0 && (
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={handleRetryFailed}
                    disabled={isRetrying}
                    className="gap-1.5"
                  >
                    <RotateCcw className={`h-3.5 w-3.5 ${isRetrying ? "animate-spin" : ""}`} />
                    {isRetrying ? "Retrying..." : `Retry Failed (${failed})`}
                  </Button>
                )}

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

          {/* Privacy & Engine Information */}
          <div className="rounded-lg border border-border bg-muted/20 p-4">
            <div className="flex items-start gap-3">
              <ShieldCheck className="h-5 w-5 text-emerald-600 dark:text-emerald-400 shrink-0 mt-0.5" />
              <div className="space-y-1">
                <h3 className="text-xs font-semibold text-foreground">
                  Zero Cloud Transmission &amp; Local Execution
                </h3>
                <p className="text-xs leading-relaxed text-muted-foreground">
                  Text recognition and search indexing run 100% on your local machine.
                  No screenshot bytes, extracted text, or search queries ever leave your computer.
                </p>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
