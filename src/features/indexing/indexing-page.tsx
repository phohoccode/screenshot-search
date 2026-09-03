import { useEffect, useState, useCallback, useRef } from "react";
import {
  ScanText,
  CheckCircle2,
  AlertCircle,
  Clock,
  Play,
  Square,
  Loader2,
  ShieldCheck,
  FileCheck,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import {
  getOcrStats,
  startOcrIndexing,
  cancelOcrIndexing,
  onOcrProgress,
} from "@/lib/tauri";
import type { OcrStats, OcrBatchSummary } from "@/types";

export function IndexingPage() {
  const [stats, setStats] = useState<OcrStats | null>(null);
  const [isRunning, setIsRunning] = useState(false);
  const [isStarting, setIsStarting] = useState(false);
  const [isCancelling, setIsCancelling] = useState(false);
  const [lastSummary, setLastSummary] = useState<OcrBatchSummary | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const pollingRef = useRef<number | null>(null);

  const refreshStats = useCallback(async () => {
    try {
      const data = await getOcrStats();
      setStats(data);
    } catch (err) {
      console.error("Failed to load OCR stats", err);
    }
  }, []);

  useEffect(() => {
    refreshStats();

    // Listen to real-time progress events from backend
    let unlisten: (() => void) | undefined;
    onOcrProgress((payload) => {
      setIsRunning(payload.isRunning);
      setStats({
        total: payload.total,
        succeeded: payload.succeeded,
        failed: payload.failed,
        pending: Math.max(0, payload.total - payload.processed),
        processing: payload.isRunning ? 1 : 0,
      });
      if (!payload.isRunning) {
        setIsStarting(false);
        setIsCancelling(false);
      }
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      if (unlisten) unlisten();
      if (pollingRef.current) clearInterval(pollingRef.current);
    };
  }, [refreshStats]);

  const handleStartOcr = async () => {
    setIsStarting(true);
    setErrorMessage(null);
    setLastSummary(null);

    try {
      setIsRunning(true);
      const summary = await startOcrIndexing();
      setLastSummary(summary);
      await refreshStats();
    } catch (err: unknown) {
      const msg =
        err && typeof err === "object" && "message" in err
          ? (err as { message: string }).message
          : "Failed to start OCR indexing";
      setErrorMessage(msg);
    } finally {
      setIsRunning(false);
      setIsStarting(false);
    }
  };

  const handleCancelOcr = async () => {
    setIsCancelling(true);
    try {
      await cancelOcrIndexing();
    } catch (err) {
      console.error("Failed to cancel OCR", err);
    } finally {
      setIsCancelling(false);
    }
  };

  const total = stats?.total ?? 0;
  const processed = (stats?.succeeded ?? 0) + (stats?.failed ?? 0);
  const percent = total > 0 ? Math.min(100, Math.round((processed / total) * 100)) : 0;
  const pending = stats?.pending ?? 0;

  return (
    <div className="flex h-full flex-col">
      <header className="border-b border-border px-6 py-4">
        <h1 className="text-base font-semibold text-foreground">Indexing</h1>
        <p className="mt-0.5 text-xs text-muted-foreground">
          Local OCR pipeline and search index status.
        </p>
      </header>

      <div className="flex-1 overflow-y-auto px-6 py-6">
        <div className="mx-auto max-w-2xl space-y-5">
          {/* OCR Indexing Card */}
          <div className="rounded-lg border border-border bg-card p-6 shadow-sm">
            <div className="flex items-center justify-between pb-4 border-b border-border">
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-muted text-primary">
                  <ScanText className="h-5 w-5" />
                </div>
                <div>
                  <h2 className="text-sm font-semibold text-foreground">
                    OCR Indexing
                  </h2>
                  <p className="text-xs text-muted-foreground">
                    Windows-native offline text recognition
                  </p>
                </div>
              </div>

              <div>
                {isRunning ? (
                  <span className="inline-flex items-center gap-1.5 rounded-full bg-primary/10 px-2.5 py-1 text-xs font-medium text-primary">
                    <Loader2 className="h-3.5 w-3.5 animate-spin" />
                    Processing
                  </span>
                ) : pending === 0 && total > 0 ? (
                  <span className="inline-flex items-center gap-1.5 rounded-full bg-emerald-500/10 px-2.5 py-1 text-xs font-medium text-emerald-600 dark:text-emerald-400">
                    <CheckCircle2 className="h-3.5 w-3.5" />
                    Up to date
                  </span>
                ) : (
                  <span className="inline-flex items-center gap-1.5 rounded-full bg-muted px-2.5 py-1 text-xs font-medium text-muted-foreground">
                    <Clock className="h-3.5 w-3.5" />
                    Idle
                  </span>
                )}
              </div>
            </div>

            {/* Progress Section */}
            <div className="mt-5 space-y-2">
              <div className="flex items-center justify-between text-xs">
                <span className="font-medium text-foreground">
                  {processed.toLocaleString()} / {total.toLocaleString()} processed
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
                <div className="text-xs text-muted-foreground">Succeeded</div>
                <div className="mt-0.5 text-base font-semibold text-emerald-600 dark:text-emerald-400">
                  {(stats?.succeeded ?? 0).toLocaleString()}
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
                  {(stats?.failed ?? 0).toLocaleString()}
                </div>
              </div>
            </div>

            {/* Error Message if any */}
            {errorMessage && (
              <div className="mt-4 flex items-center gap-2 rounded-md border border-rose-200 bg-rose-50 p-3 text-xs text-rose-700 dark:border-rose-900/50 dark:bg-rose-950/30 dark:text-rose-400">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <span>{errorMessage}</span>
              </div>
            )}

            {/* Last Batch Summary Banner */}
            {lastSummary && (
              <div className="mt-4 flex items-center gap-2 rounded-md border border-border bg-muted/50 p-3 text-xs text-muted-foreground">
                <FileCheck className="h-4 w-4 shrink-0 text-primary" />
                <span>
                  Batch finished in {lastSummary.durationMs}ms: {lastSummary.succeeded} indexed,{" "}
                  {lastSummary.failed} failed.
                </span>
              </div>
            )}

            {/* Action Buttons */}
            <div className="mt-6 flex items-center justify-end gap-3">
              {isRunning ? (
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleCancelOcr}
                  disabled={isCancelling}
                  className="gap-1.5 text-rose-600 dark:text-rose-400"
                >
                  <Square className="h-3.5 w-3.5" />
                  {isCancelling ? "Stopping..." : "Stop OCR"}
                </Button>
              ) : (
                <Button
                  size="sm"
                  onClick={handleStartOcr}
                  disabled={pending === 0 || isStarting}
                  className="gap-1.5"
                >
                  {isStarting ? (
                    <>
                      <Loader2 className="h-3.5 w-3.5 animate-spin" />
                      Starting...
                    </>
                  ) : (
                    <>
                      <Play className="h-3.5 w-3.5" />
                      Start OCR ({pending.toLocaleString()})
                    </>
                  )}
                </Button>
              )}
            </div>
          </div>

          {/* Privacy & Engine Information */}
          <div className="rounded-lg border border-border bg-muted/20 p-4">
            <div className="flex items-start gap-3">
              <ShieldCheck className="h-5 w-5 text-emerald-600 dark:text-emerald-400 shrink-0 mt-0.5" />
              <div className="space-y-1">
                <h3 className="text-xs font-semibold text-foreground">
                  Zero Cloud Transmission &amp; Local Execution
                </h3>
                <p className="text-xs leading-relaxed text-muted-foreground">
                  Text recognition is executed 100% on your local CPU via the
                  Windows Media OCR API. Screenshots, images, and extracted text never leave your
                  computer.
                </p>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
