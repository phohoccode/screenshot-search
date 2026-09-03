import { useEffect, useState, useRef } from "react";
import {
  Search,
  FolderOpen,
  ScanText,
  X,
  ExternalLink,
  FolderSearch,
  Copy,
  Check,
  Calendar,
  HardDrive,
  Maximize2,
  FileText,
  Loader2,
  AlertCircle,
} from "lucide-react";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "@/components/ui/dialog";
import {
  searchScreenshots,
  getScreenshot,
  getScreenshotImageUrl,
  getScreenshotImageData,
  openScreenshot,
  revealScreenshot,
  getOcrStats,
} from "@/lib/tauri";
import type {
  SearchResultItem,
  ScreenshotDetail,
  OcrStats,
} from "@/types";

/** Renders snippet with highlight tokens cleanly using React text nodes (no dangerouslySetInnerHTML) */
function MatchSnippet({ snippet }: { snippet: string }) {
  const parts = snippet.split(/(\[\[match\]\].*?\[\[\/match\]\])/g);
  return (
    <p className="line-clamp-2 text-xs text-muted-foreground leading-relaxed">
      {parts.map((part, i) => {
        if (part.startsWith("[[match]]") && part.endsWith("[[/match]]")) {
          const matchText = part.slice(9, -10);
          return (
            <mark
              key={i}
              className="rounded bg-primary/25 px-1 py-0.5 font-medium text-foreground"
            >
              {matchText}
            </mark>
          );
        }
        return <span key={i}>{part}</span>;
      })}
    </p>
  );
}

export function SearchPage() {
  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [results, setResults] = useState<SearchResultItem[]>([]);
  const [totalMatches, setTotalMatches] = useState(0);
  const [hasMore, setHasMore] = useState(false);
  const [offset, setOffset] = useState(0);
  const [isLoading, setIsLoading] = useState(false);
  const [isLoadingMore, setIsLoadingMore] = useState(false);
  const [stats, setStats] = useState<OcrStats | null>(null);

  // Preview Dialog State
  const [selectedScreenshot, setSelectedScreenshot] = useState<ScreenshotDetail | null>(null);
  const [isPreviewOpen, setIsPreviewOpen] = useState(false);
  const [copiedText, setCopiedText] = useState(false);

  // Preview Modal Image Loading & Fallback State
  const [modalImageSrc, setModalImageSrc] = useState<string | null>(null);
  const [isModalImageLoading, setIsModalImageLoading] = useState(true);
  const [modalImageFailed, setModalImageFailed] = useState(false);

  useEffect(() => {
    if (selectedScreenshot) {
      setIsModalImageLoading(true);
      setModalImageFailed(false);
      setModalImageSrc(getScreenshotImageUrl(selectedScreenshot.id, selectedScreenshot.path));
    } else {
      setModalImageSrc(null);
    }
  }, [selectedScreenshot]);

  const handleModalImageError = async () => {
    if (!selectedScreenshot) return;
    try {
      const base64Data = await getScreenshotImageData(selectedScreenshot.id);
      setModalImageSrc(base64Data);
      setIsModalImageLoading(false);
    } catch {
      setModalImageFailed(true);
      setIsModalImageLoading(false);
    }
  };

  const inputRef = useRef<HTMLInputElement>(null);

  // Load stats initially
  useEffect(() => {
    getOcrStats()
      .then(setStats)
      .catch(() => setStats(null));
  }, []);

  // Keyboard shortcut: "/" to focus search input
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (
        e.key === "/" &&
        document.activeElement !== inputRef.current &&
        !isPreviewOpen
      ) {
        e.preventDefault();
        inputRef.current?.focus();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isPreviewOpen]);

  // Debounce query input by 200ms
  useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedQuery(query);
    }, 200);
    return () => clearTimeout(timer);
  }, [query]);

  // Execute search when debounced query changes
  useEffect(() => {
    let isCurrent = true;
    setIsLoading(true);

    searchScreenshots(debouncedQuery, undefined, 40, 0)
      .then((page) => {
        if (!isCurrent) return;
        setResults(page.items);
        setTotalMatches(page.totalMatches);
        setHasMore(page.hasMore);
        setOffset(page.items.length);
      })
      .catch((err) => {
        console.error("Search error:", err);
        if (isCurrent) {
          setResults([]);
          setTotalMatches(0);
          setHasMore(false);
        }
      })
      .finally(() => {
        if (isCurrent) setIsLoading(false);
      });

    return () => {
      isCurrent = false;
    };
  }, [debouncedQuery]);

  // Load more results
  const handleLoadMore = async () => {
    if (isLoadingMore || !hasMore) return;
    setIsLoadingMore(true);
    try {
      const page = await searchScreenshots(debouncedQuery, undefined, 40, offset);
      setResults((prev) => [...prev, ...page.items]);
      setHasMore(page.hasMore);
      setOffset((prev) => prev + page.items.length);
    } catch (err) {
      console.error("Failed to load more results:", err);
    } finally {
      setIsLoadingMore(false);
    }
  };

  // Open full preview modal
  const handleCardClick = async (item: SearchResultItem) => {
    try {
      const detail = await getScreenshot(item.id);
      setSelectedScreenshot(detail);
      setIsPreviewOpen(true);
      setCopiedText(false);
    } catch (err) {
      console.error("Failed to fetch screenshot detail:", err);
    }
  };

  // Copy OCR text to clipboard
  const handleCopyOcrText = () => {
    if (!selectedScreenshot?.ocrText) return;
    navigator.clipboard.writeText(selectedScreenshot.ocrText).then(() => {
      setCopiedText(true);
      setTimeout(() => setCopiedText(false), 2000);
    });
  };

  const total = stats?.total ?? 0;
  const succeeded = stats?.succeeded ?? 0;

  return (
    <div className="flex h-full flex-col">
      {/* Search Header Bar */}
      <header className="border-b border-border bg-card px-6 py-3.5">
        <div className="flex items-center justify-between gap-4">
          <div className="relative flex-1 max-w-xl">
            <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
            <Input
              ref={inputRef}
              type="text"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Search screenshots by text, code, error, or filename... (Press '/' to focus)"
              className="pl-9 pr-9 text-sm"
              autoFocus
            />
            {query && (
              <button
                type="button"
                onClick={() => {
                  setQuery("");
                  inputRef.current?.focus();
                }}
                className="absolute right-3 top-1/2 -translate-y-1/2 text-muted-foreground hover:text-foreground"
                aria-label="Clear search"
              >
                <X className="h-4 w-4" />
              </button>
            )}
          </div>

          {/* Match counter / status */}
          <div className="flex items-center gap-2 text-xs text-muted-foreground whitespace-nowrap">
            {isLoading ? (
              <span className="flex items-center gap-1.5">
                <Loader2 className="h-3.5 w-3.5 animate-spin" />
                Searching...
              </span>
            ) : debouncedQuery ? (
              <span>
                <strong>{totalMatches.toLocaleString()}</strong> match
                {totalMatches === 1 ? "" : "es"}
              </span>
            ) : succeeded > 0 ? (
              <span>
                Showing {results.length} of {succeeded.toLocaleString()} recent screenshots
              </span>
            ) : null}
          </div>
        </div>
      </header>

      {/* Main Content Area */}
      <div className="flex-1 overflow-y-auto p-6">
        {/* Loading Skeletons */}
        {isLoading && results.length === 0 ? (
          <div className="grid grid-cols-2 gap-4 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
            {Array.from({ length: 10 }).map((_, i) => (
              <div key={i} className="flex flex-col gap-2 rounded-lg border border-border p-3">
                <Skeleton className="aspect-video w-full rounded" />
                <Skeleton className="h-4 w-3/4" />
                <Skeleton className="h-3 w-1/2" />
              </div>
            ))}
          </div>
        ) : total === 0 ? (
          /* Empty State: No Folders */
          <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
            <div className="flex h-12 w-12 items-center justify-center rounded-lg bg-muted text-muted-foreground">
              <FolderOpen className="h-6 w-6" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-foreground">
                No screenshot folders added
              </h2>
              <p className="mt-1 max-w-sm text-xs text-muted-foreground">
                Add a folder in the <strong>Folders</strong> tab to begin discovering screenshots.
              </p>
            </div>
          </div>
        ) : succeeded === 0 ? (
          /* Empty State: Screenshots Discovered, but OCR Pending */
          <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
            <div className="flex h-12 w-12 items-center justify-center rounded-lg bg-muted text-primary">
              <ScanText className="h-6 w-6" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-foreground">
                {total.toLocaleString()} screenshots waiting for OCR
              </h2>
              <p className="mt-1 max-w-sm text-xs text-muted-foreground">
                Go to the <strong>Indexing</strong> tab and click "Start Indexing" to extract
                text and enable full-text search.
              </p>
            </div>
          </div>
        ) : results.length === 0 ? (
          /* Empty State: No Results Match */
          <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
            <div className="flex h-12 w-12 items-center justify-center rounded-lg bg-muted text-muted-foreground">
              <Search className="h-6 w-6" />
            </div>
            <div>
              <h2 className="text-sm font-semibold text-foreground">
                No screenshots match "{debouncedQuery}"
              </h2>
              <p className="mt-1 max-w-sm text-xs text-muted-foreground">
                Try searching for a different keyword, error code, or clear the search field.
              </p>
            </div>
            <Button variant="outline" size="sm" onClick={() => setQuery("")}>
              Clear search
            </Button>
          </div>
        ) : (
          /* Results Grid */
          <div className="space-y-6">
            <div className="grid grid-cols-2 gap-4 md:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
              {results.map((item) => (
                <div
                  key={item.id}
                  onClick={() => handleCardClick(item)}
                  className="group relative flex cursor-pointer flex-col overflow-hidden rounded-lg border border-border bg-card transition-all hover:border-primary/50 hover:shadow-sm"
                >
                  {/* Thumbnail */}
                  <div className="relative aspect-video w-full overflow-hidden bg-muted/40">
                    <img
                      src={getScreenshotImageUrl(item.id, item.path)}
                      alt={item.filename}
                      loading="lazy"
                      className="h-full w-full object-cover transition-transform duration-200 group-hover:scale-[1.02]"
                      onError={(e) => {
                        const target = e.target as HTMLImageElement;
                        target.onerror = null;
                        target.src = `https://placehold.co/600x400/18181b/ffffff?text=${encodeURIComponent(item.filename)}`;
                      }}
                    />
                    {/* Hover Overlay Action Bar */}
                    <div className="absolute inset-0 flex items-center justify-center gap-2 bg-black/40 opacity-0 transition-opacity group-hover:opacity-100">
                      <span className="flex items-center gap-1 rounded bg-background/90 px-2 py-1 text-[11px] font-medium text-foreground backdrop-blur-sm shadow">
                        <Maximize2 className="h-3 w-3" />
                        Preview
                      </span>
                    </div>
                  </div>

                  {/* Card Content */}
                  <div className="flex flex-1 flex-col gap-1.5 p-3">
                    <div className="flex items-start justify-between gap-1">
                      <h3
                        className="truncate text-xs font-semibold text-foreground"
                        title={item.filename}
                      >
                        {item.filename}
                      </h3>
                    </div>

                    {/* Highlighted Match Snippet */}
                    {item.matchSnippet ? (
                      <MatchSnippet snippet={item.matchSnippet} />
                    ) : (
                      <p className="text-[11px] text-muted-foreground">
                        {new Date(item.modifiedAtFs).toLocaleDateString()}
                      </p>
                    )}
                  </div>
                </div>
              ))}
            </div>

            {/* Load More Button */}
            {hasMore && (
              <div className="flex justify-center pb-6">
                <Button
                  variant="outline"
                  size="sm"
                  onClick={handleLoadMore}
                  disabled={isLoadingMore}
                  className="text-xs"
                >
                  {isLoadingMore ? (
                    <>
                      <Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
                      Loading more...
                    </>
                  ) : (
                    "Load more results"
                  )}
                </Button>
              </div>
            )}
          </div>
        )}
      </div>

      {/* Screenshot Preview Modal */}
      <Dialog open={isPreviewOpen} onOpenChange={setIsPreviewOpen}>
        <DialogContent className="max-w-3xl max-h-[90vh] flex flex-col p-0 overflow-hidden">
          {selectedScreenshot && (
            <>
              {/* Modal Header */}
              <DialogHeader className="border-b border-border px-6 py-4">
                <DialogTitle className="truncate text-base font-semibold" title={selectedScreenshot.filename}>
                  {selectedScreenshot.filename}
                </DialogTitle>
                <DialogDescription className="truncate text-xs text-muted-foreground" title={selectedScreenshot.path}>
                  {selectedScreenshot.path}
                </DialogDescription>
              </DialogHeader>

              {/* Modal Body: Scrollable */}
              <div className="flex-1 overflow-y-auto p-6 space-y-5">
                {/* Large Preview Image */}
                <div className="relative flex min-h-[240px] max-h-[420px] w-full items-center justify-center rounded-lg border border-border bg-muted/10 p-2 overflow-hidden">
                  {isModalImageLoading && !modalImageFailed && (
                    <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 bg-background/60 backdrop-blur-xs z-10">
                      <Loader2 className="h-6 w-6 animate-spin text-primary" />
                      <span className="text-xs text-muted-foreground">Loading preview...</span>
                    </div>
                  )}
                  {modalImageFailed ? (
                    <div className="flex flex-col items-center justify-center gap-2 p-6 text-center text-muted-foreground">
                      <AlertCircle className="h-8 w-8 text-destructive" />
                      <p className="text-xs font-semibold text-foreground">
                        Screenshot file could not be loaded
                      </p>
                      <p className="max-w-md text-[11px] text-muted-foreground">
                        The file may have been moved, renamed, or deleted from disk:
                        <br />
                        <code className="mt-1.5 inline-block rounded bg-muted px-2 py-1 font-mono text-[10px] text-foreground break-all">
                          {selectedScreenshot.path}
                        </code>
                      </p>
                    </div>
                  ) : (
                    <img
                      src={
                        modalImageSrc ||
                        getScreenshotImageUrl(selectedScreenshot.id, selectedScreenshot.path)
                      }
                      alt={selectedScreenshot.filename}
                      onLoad={() => setIsModalImageLoading(false)}
                      onError={handleModalImageError}
                      className="max-h-[400px] w-auto max-w-full rounded object-contain shadow-xs transition-opacity duration-150"
                    />
                  )}
                </div>

                {/* Metadata details row */}
                <div className="grid grid-cols-2 sm:grid-cols-4 gap-3 rounded-lg border border-border bg-card p-3 text-xs">
                  <div>
                    <span className="text-muted-foreground flex items-center gap-1">
                      <Calendar className="h-3 w-3" /> Modified
                    </span>
                    <p className="mt-0.5 font-medium truncate">
                      {new Date(selectedScreenshot.modifiedAtFs).toLocaleString()}
                    </p>
                  </div>
                  <div>
                    <span className="text-muted-foreground flex items-center gap-1">
                      <HardDrive className="h-3 w-3" /> Size
                    </span>
                    <p className="mt-0.5 font-medium">
                      {(selectedScreenshot.fileSize / 1024).toFixed(1)} KB
                    </p>
                  </div>
                  <div>
                    <span className="text-muted-foreground flex items-center gap-1">
                      <Maximize2 className="h-3 w-3" /> Dimensions
                    </span>
                    <p className="mt-0.5 font-medium">
                      {selectedScreenshot.width && selectedScreenshot.height
                        ? `${selectedScreenshot.width} × ${selectedScreenshot.height}`
                        : "Unknown"}
                    </p>
                  </div>
                  <div>
                    <span className="text-muted-foreground flex items-center gap-1">
                      <ScanText className="h-3 w-3" /> Status
                    </span>
                    <p className="mt-0.5 font-medium text-emerald-600 dark:text-emerald-400">
                      {selectedScreenshot.ocrStatus}
                    </p>
                  </div>
                </div>

                {/* Extracted OCR Text Section */}
                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <span className="flex items-center gap-1.5 text-xs font-semibold text-foreground">
                      <FileText className="h-3.5 w-3.5 text-muted-foreground" />
                      Extracted OCR Text
                    </span>
                    {selectedScreenshot.ocrText && (
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={handleCopyOcrText}
                        className="h-7 text-xs"
                      >
                        {copiedText ? (
                          <>
                            <Check className="mr-1 h-3 w-3 text-emerald-500" />
                            Copied!
                          </>
                        ) : (
                          <>
                            <Copy className="mr-1 h-3 w-3" />
                            Copy text
                          </>
                        )}
                      </Button>
                    )}
                  </div>
                  <div className="max-h-48 overflow-y-auto rounded-lg border border-border bg-muted/40 p-3 font-mono text-xs leading-relaxed text-foreground whitespace-pre-wrap select-text">
                    {selectedScreenshot.ocrText || (
                      <span className="italic text-muted-foreground">
                        No text was detected in this screenshot.
                      </span>
                    )}
                  </div>
                </div>
              </div>

              {/* Modal Actions Footer */}
              <DialogFooter className="border-t border-border bg-card px-6 py-3.5 sm:justify-between">
                <div className="flex items-center gap-2">
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => openScreenshot(selectedScreenshot.id)}
                    className="text-xs"
                  >
                    <ExternalLink className="mr-1.5 h-3.5 w-3.5" />
                    Open with default app
                  </Button>
                  <Button
                    variant="outline"
                    size="sm"
                    onClick={() => revealScreenshot(selectedScreenshot.id)}
                    className="text-xs"
                  >
                    <FolderSearch className="mr-1.5 h-3.5 w-3.5" />
                    Reveal in Explorer
                  </Button>
                </div>
                <Button
                  variant="default"
                  size="sm"
                  onClick={() => setIsPreviewOpen(false)}
                  className="text-xs"
                >
                  Close
                </Button>
              </DialogFooter>
            </>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
