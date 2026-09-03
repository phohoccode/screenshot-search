import { useEffect, useState } from "react";
import { Search, FolderOpen, ScanText } from "lucide-react";
import { Input } from "@/components/ui/input";
import { getOcrStats } from "@/lib/tauri";
import type { OcrStats } from "@/types";

export function SearchPage() {
  const [stats, setStats] = useState<OcrStats | null>(null);

  useEffect(() => {
    getOcrStats()
      .then(setStats)
      .catch(() => setStats(null));
  }, []);

  const total = stats?.total ?? 0;
  const succeeded = stats?.succeeded ?? 0;

  return (
    <div className="flex h-full flex-col">
      {/* Header with search bar */}
      <header className="border-b border-border px-6 py-3.5">
        <div className="relative max-w-md">
          <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="search"
            placeholder={
              succeeded > 0
                ? `${succeeded.toLocaleString()} screenshots indexed (FTS search coming in Phase 1D)...`
                : "Full-text search enabled in Phase 1D..."
            }
            disabled={true}
            className="pl-9 pr-4 text-sm"
          />
        </div>
      </header>

      {/* State View */}
      <div className="flex flex-1 flex-col items-center justify-center gap-4 px-6 text-center">
        {succeeded > 0 ? (
          <>
            <div className="flex h-12 w-12 items-center justify-center rounded-lg bg-emerald-500/10 text-emerald-600 dark:text-emerald-400">
              <ScanText className="h-6 w-6" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-foreground">
                {succeeded.toLocaleString()} screenshots are ready for search
              </h2>
              <p className="mt-1 max-w-sm text-xs text-muted-foreground">
                Text content has been extracted and normalized locally into SQLite.
                Full-text keyword retrieval (FTS5 + BM25 ranking) will be connected in Phase 1D.
              </p>
            </div>
          </>
        ) : total > 0 ? (
          <>
            <div className="flex h-12 w-12 items-center justify-center rounded-lg bg-muted text-primary">
              <ScanText className="h-6 w-6" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-foreground">
                {total.toLocaleString()} screenshots discovered
              </h2>
              <p className="mt-1 max-w-sm text-xs text-muted-foreground">
                Run OCR in the <strong>Indexing</strong> tab to extract text from
                your screenshots before searching.
              </p>
            </div>
          </>
        ) : (
          <>
            <div className="flex h-12 w-12 items-center justify-center rounded-lg bg-muted text-muted-foreground">
              <FolderOpen className="h-6 w-6" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-foreground">
                No screenshots discovered yet
              </h2>
              <p className="mt-1 max-w-sm text-xs text-muted-foreground">
                Add a screenshot folder in the <strong>Folders</strong> tab to
                begin discovering images.
              </p>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
