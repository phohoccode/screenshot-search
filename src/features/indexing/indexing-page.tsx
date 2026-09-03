import { useEffect, useState } from "react";
import { Database, CheckCircle2 } from "lucide-react";
import { getTotalScreenshotCount } from "@/lib/tauri";

export function IndexingPage() {
  const [totalScreenshots, setTotalScreenshots] = useState<number | null>(null);

  useEffect(() => {
    getTotalScreenshotCount()
      .then(setTotalScreenshots)
      .catch(() => setTotalScreenshots(0));
  }, []);

  return (
    <div className="flex h-full flex-col">
      <header className="border-b border-border px-6 py-4">
        <h1 className="text-base font-semibold text-foreground">Indexing</h1>
        <p className="mt-0.5 text-xs text-muted-foreground">
          Discovery and indexing pipeline status.
        </p>
      </header>

      <div className="flex-1 overflow-y-auto px-6 py-6">
        <div className="mx-auto max-w-2xl space-y-4">
          {/* Discovery Status Card */}
          <div className="rounded-lg border border-border bg-card p-5">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-3">
                <div className="flex h-10 w-10 items-center justify-center rounded-lg bg-muted text-primary">
                  <Database className="h-5 w-5" />
                </div>
                <div>
                  <h2 className="text-sm font-medium text-foreground">
                    Filesystem Discovery
                  </h2>
                  <p className="text-xs text-muted-foreground">
                    {totalScreenshots !== null
                      ? `${totalScreenshots.toLocaleString()} images indexed in SQLite`
                      : "Checking discovery status..."}
                  </p>
                </div>
              </div>
              <div className="flex items-center gap-1.5 rounded-full bg-muted px-2.5 py-1 text-xs font-medium text-muted-foreground">
                <CheckCircle2 className="h-3.5 w-3.5 text-primary" />
                <span>Idle</span>
              </div>
            </div>
          </div>

          {/* Pipeline Note */}
          <div className="rounded-lg border border-dashed border-border p-4 text-center">
            <p className="text-xs text-muted-foreground">
              Local OCR pipeline (Phase 1C) will process discovered screenshots
              and synchronize full-text search indexes.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
