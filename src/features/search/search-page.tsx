import { Search } from "lucide-react";
import { Input } from "@/components/ui/input";
import { FolderOpen } from "lucide-react";
import { Button } from "@/components/ui/button";

export function SearchPage() {
  return (
    <div className="flex h-full flex-col">
      {/* Header with search bar */}
      <header className="border-b px-5 py-3">
        <div className="relative">
          <Search className="absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted-foreground" />
          <Input
            type="search"
            placeholder="Search screenshots..."
            className="pl-9 pr-4"
            data-selectable
          />
        </div>
      </header>

      {/* Empty state */}
      <div className="flex flex-1 flex-col items-center justify-center gap-4 px-5">
        <div className="flex h-12 w-12 items-center justify-center rounded-lg bg-muted">
          <FolderOpen className="h-6 w-6 text-muted-foreground" />
        </div>
        <div className="text-center">
          <h2 className="text-base font-semibold text-foreground">
            No screenshot folders yet
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Choose a folder to start indexing screenshots.
          </p>
        </div>
        <Button variant="outline" size="sm">
          <FolderOpen className="h-4 w-4" />
          Choose folder
        </Button>
      </div>
    </div>
  );
}
