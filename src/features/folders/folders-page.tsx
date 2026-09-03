import { FolderOpen } from "lucide-react";
import { Button } from "@/components/ui/button";

export function FoldersPage() {
  return (
    <div className="flex h-full flex-col">
      <header className="border-b px-5 py-3">
        <h1 className="text-lg font-semibold">Folders</h1>
      </header>

      <div className="flex flex-1 flex-col items-center justify-center gap-4 px-5">
        <div className="flex h-12 w-12 items-center justify-center rounded-lg bg-muted">
          <FolderOpen className="h-6 w-6 text-muted-foreground" />
        </div>
        <div className="text-center">
          <h2 className="text-base font-semibold text-foreground">
            No screenshot folders yet
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Add a folder containing screenshots to get started.
          </p>
        </div>
        <Button variant="outline" size="sm">
          <FolderOpen className="h-4 w-4" />
          Add folder
        </Button>
      </div>
    </div>
  );
}
