import { useEffect, useState } from "react";
import { Search, FolderOpen, Database } from "lucide-react";
import { Input } from "@/components/ui/input";
import { getTotalScreenshotCount } from "@/lib/tauri";

export function SearchPage() {
  const [totalScreenshots, setTotalScreenshots] = useState<number | null>(null);

  useEffect(() => {
    getTotalScreenshotCount()
      .then(setTotalScreenshots)
      .catch(() => setTotalScreenshots(0));
  }, []);

  return (
    <div className="flex h-full flex-col">
      {/* Header with search bar */}
      <header className="border-b border-border px-6 py-3.5">
        <div className="relative max-w-md">
          <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="search"
            placeholder={
              totalScreenshots && totalScreenshots > 0
                ? "Keyword search will be enabled in Phase 1C..."
                : "Search screenshots..."
            }
            disabled={true}
            className="pl-9 pr-4 text-sm"
          />
        </div>
      </header>

      {/* State View */}
      <div className="flex flex-1 flex-col items-center justify-center gap-4 px-6 text-center">
        {totalScreenshots && totalScreenshots > 0 ? (
          <>
            <div className="flex h-12 w-12 items-center justify-center rounded-lg bg-muted text-primary">
              <Database className="h-6 w-6" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-foreground">
                {totalScreenshots.toLocaleString()} screenshots discovered
              </h2>
              <p className="mt-1 max-w-sm text-xs text-muted-foreground">
                Screenshots have been discovered and metadata is indexed in SQLite.
                Full-text keyword search will be available once the local OCR
                pipeline (Phase 1C) is initialized.
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
