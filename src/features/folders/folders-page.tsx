import { useEffect, useState, useCallback } from "react";
import {
  Folder as FolderIcon,
  FolderOpen,
  FolderPlus,
  RefreshCw,
  Trash2,
  AlertCircle,
  CheckCircle2,
  Clock,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
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
import { Skeleton } from "@/components/ui/skeleton";
import {
  listFolders,
  addFolder,
  removeFolder,
  scanFolder,
  pickFolder,
} from "@/lib/tauri";
import type { Folder, ScanSummary, AppError } from "@/types";

function formatTimestamp(isoString: string | null): string {
  if (!isoString) return "Never";
  try {
    const date = new Date(isoString);
    if (isNaN(date.getTime())) return "Never";

    const now = new Date();
    const diffMs = now.getTime() - date.getTime();
    const diffMins = Math.floor(diffMs / 60000);

    if (diffMins < 1) return "Just now";
    if (diffMins < 60) return `${diffMins}m ago`;
    const diffHours = Math.floor(diffMins / 60);
    if (diffHours < 24) return `${diffHours}h ago`;

    return date.toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return "Never";
  }
}

function getFolderName(path: string): string {
  const normalized = path.replace(/[\\/]+$/, "");
  const parts = normalized.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

export function FoldersPage() {
  const [folders, setFolders] = useState<Folder[]>([]);
  const [loading, setLoading] = useState(true);
  const [scanningFolderIds, setScanningFolderIds] = useState<Set<number>>(
    new Set()
  );
  const [folderPendingDelete, setFolderPendingDelete] = useState<Folder | null>(
    null
  );
  const [scanSummary, setScanSummary] = useState<ScanSummary | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [isAddingFolder, setIsAddingFolder] = useState(false);

  const fetchFolders = useCallback(async () => {
    try {
      const data = await listFolders();
      setFolders(data);
    } catch (err) {
      const error = err as AppError;
      setErrorMessage(error.message || "Failed to load folders");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchFolders();
  }, [fetchFolders]);

  const handleAddFolder = async () => {
    setErrorMessage(null);
    setIsAddingFolder(true);
    try {
      const selectedPath = await pickFolder();
      if (!selectedPath) {
        setIsAddingFolder(false);
        return;
      }

      const newFolder = await addFolder(selectedPath);
      await fetchFolders();

      // Trigger automatic initial scan
      setScanningFolderIds((prev) => new Set(prev).add(newFolder.id));
      try {
        const summary = await scanFolder(newFolder.id);
        setScanSummary(summary);
      } catch (scanErr) {
        const error = scanErr as AppError;
        setErrorMessage(error.message || "Initial scan failed");
      } finally {
        setScanningFolderIds((prev) => {
          const next = new Set(prev);
          next.delete(newFolder.id);
          return next;
        });
        await fetchFolders();
      }
    } catch (err) {
      const error = err as AppError;
      setErrorMessage(error.message || "Failed to add folder");
    } finally {
      setIsAddingFolder(false);
    }
  };

  const handleRescan = async (folderId: number) => {
    setErrorMessage(null);
    setScanSummary(null);
    setScanningFolderIds((prev) => new Set(prev).add(folderId));

    try {
      const summary = await scanFolder(folderId);
      setScanSummary(summary);
    } catch (err) {
      const error = err as AppError;
      setErrorMessage(error.message || "Rescan failed");
    } finally {
      setScanningFolderIds((prev) => {
        const next = new Set(prev);
        next.delete(folderId);
        return next;
      });
      await fetchFolders();
    }
  };

  const handleConfirmRemove = async () => {
    if (!folderPendingDelete) return;

    try {
      await removeFolder(folderPendingDelete.id);
      setFolderPendingDelete(null);
      await fetchFolders();
    } catch (err) {
      const error = err as AppError;
      setErrorMessage(error.message || "Failed to remove folder");
    }
  };

  return (
    <div className="flex h-full flex-col">
      {/* Header */}
      <header className="flex items-center justify-between border-b border-border px-6 py-4">
        <div>
          <h1 className="text-base font-semibold text-foreground">Folders</h1>
          <p className="mt-0.5 text-xs text-muted-foreground">
            Manage folders Screenshot Search can index.
          </p>
        </div>
        <Button
          onClick={handleAddFolder}
          disabled={isAddingFolder}
          size="sm"
          className="gap-1.5"
        >
          <FolderPlus className="h-4 w-4" />
          Add folder
        </Button>
      </header>

      {/* Content */}
      <div className="flex-1 overflow-y-auto px-6 py-5">
        <div className="mx-auto max-w-4xl space-y-4">
          {/* Error Banner */}
          {errorMessage && (
            <div className="flex items-start justify-between rounded-lg border border-destructive/20 bg-destructive/10 p-3 text-sm text-destructive">
              <div className="flex items-center gap-2">
                <AlertCircle className="h-4 w-4 shrink-0" />
                <span>{errorMessage}</span>
              </div>
              <button
                onClick={() => setErrorMessage(null)}
                className="rounded p-0.5 hover:bg-destructive/20"
                aria-label="Dismiss error"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
          )}

          {/* Scan Summary Banner */}
          {scanSummary && (
            <div className="flex items-start justify-between rounded-lg border border-border bg-muted/50 p-3 text-sm">
              <div className="flex items-center gap-2 text-foreground">
                <CheckCircle2 className="h-4 w-4 text-primary shrink-0" />
                <span>
                  Scan complete:{" "}
                  <strong>{scanSummary.discovered.toLocaleString()}</strong>{" "}
                  files discovered ({scanSummary.added} added,{" "}
                  {scanSummary.updated} updated, {scanSummary.removed} removed)
                  in {(scanSummary.durationMs / 1000).toFixed(2)}s.
                </span>
              </div>
              <button
                onClick={() => setScanSummary(null)}
                className="rounded p-0.5 text-muted-foreground hover:bg-muted"
                aria-label="Dismiss notification"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
          )}

          {/* Loading State */}
          {loading ? (
            <div className="space-y-3">
              <Skeleton className="h-20 w-full rounded-lg" />
              <Skeleton className="h-20 w-full rounded-lg" />
            </div>
          ) : folders.length === 0 ? (
            /* Empty State */
            <div className="flex min-h-[380px] flex-col items-center justify-center rounded-xl border border-dashed border-border p-8 text-center">
              <div className="flex h-12 w-12 items-center justify-center rounded-lg bg-muted text-muted-foreground">
                <FolderOpen className="h-6 w-6" />
              </div>
              <h2 className="mt-4 text-sm font-semibold text-foreground">
                No screenshot folders yet
              </h2>
              <p className="mt-1 max-w-sm text-xs text-muted-foreground">
                Choose a folder to start discovering screenshots. Supported
                formats include PNG, JPG, JPEG, and WebP.
              </p>
              <Button
                onClick={handleAddFolder}
                variant="outline"
                size="sm"
                className="mt-5 gap-1.5"
              >
                <FolderPlus className="h-4 w-4" />
                Choose folder
              </Button>
            </div>
          ) : (
            /* Folder Cards */
            <div className="space-y-2.5">
              {folders.map((folder) => {
                const isScanning = scanningFolderIds.has(folder.id);
                const folderName = getFolderName(folder.path);

                return (
                  <div
                    key={folder.id}
                    className="flex flex-col gap-3 rounded-lg border border-border bg-card p-4 transition-colors hover:border-border/80 sm:flex-row sm:items-center sm:justify-between"
                  >
                    {/* Folder Info */}
                    <div className="flex items-start gap-3 min-w-0">
                      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-md bg-muted text-foreground">
                        <FolderIcon className="h-4 w-4 text-primary" />
                      </div>
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="truncate text-sm font-medium text-foreground">
                            {folderName}
                          </span>
                          <span className="shrink-0 rounded-full bg-muted px-2 py-0.5 text-[11px] font-medium text-muted-foreground">
                            {folder.screenshotCount.toLocaleString()} images
                            {folder.ocrSucceededCount > 0 &&
                              ` (${folder.ocrSucceededCount.toLocaleString()} OCR indexed)`}
                          </span>
                        </div>
                        <p
                          className="mt-0.5 truncate font-mono text-xs text-muted-foreground"
                          title={folder.path}
                        >
                          {folder.path}
                        </p>
                        <div className="mt-2 flex items-center gap-1.5 text-[11px] text-muted-foreground">
                          <Clock className="h-3 w-3" />
                          <span>
                            Last scanned: {formatTimestamp(folder.lastScannedAt)}
                          </span>
                        </div>
                      </div>
                    </div>

                    {/* Actions */}
                    <div className="flex items-center gap-2 self-end sm:self-center">
                      <Button
                        variant="outline"
                        size="sm"
                        disabled={isScanning}
                        onClick={() => handleRescan(folder.id)}
                        className="gap-1.5"
                      >
                        <RefreshCw
                          className={`h-3.5 w-3.5 ${
                            isScanning ? "animate-spin" : ""
                          }`}
                        />
                        {isScanning ? "Scanning..." : "Rescan"}
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        disabled={isScanning}
                        onClick={() => setFolderPendingDelete(folder)}
                        aria-label={`Remove folder ${folderName}`}
                        className="h-8 w-8 text-muted-foreground hover:text-destructive hover:bg-destructive/10"
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>

      {/* Remove Confirmation Dialog */}
      <AlertDialog
        open={Boolean(folderPendingDelete)}
        onOpenChange={(open) => !open && setFolderPendingDelete(null)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Remove this folder?</AlertDialogTitle>
            <AlertDialogDescription className="space-y-2 text-sm text-muted-foreground">
              <span>
                Screenshot Search will stop managing this folder and remove its
                local index metadata.
              </span>
              <strong className="block text-foreground">
                Your original screenshots will not be deleted.
              </strong>
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={handleConfirmRemove}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              Remove folder
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
