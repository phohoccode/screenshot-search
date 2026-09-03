import { Database } from "lucide-react";

export function IndexingPage() {
  return (
    <div className="flex h-full flex-col">
      <header className="border-b px-5 py-3">
        <h1 className="text-lg font-semibold">Indexing</h1>
      </header>

      <div className="flex flex-1 flex-col items-center justify-center gap-4 px-5">
        <div className="flex h-12 w-12 items-center justify-center rounded-lg bg-muted">
          <Database className="h-6 w-6 text-muted-foreground" />
        </div>
        <div className="text-center">
          <h2 className="text-base font-semibold text-foreground">
            No indexing activity
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Add screenshot folders first, then indexing will begin automatically.
          </p>
        </div>
      </div>
    </div>
  );
}
