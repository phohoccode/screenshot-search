import { Settings } from "lucide-react";

export function SettingsPage() {
  return (
    <div className="flex h-full flex-col">
      <header className="border-b px-5 py-3">
        <h1 className="text-lg font-semibold">Settings</h1>
      </header>

      <div className="flex flex-1 flex-col items-center justify-center gap-4 px-5">
        <div className="flex h-12 w-12 items-center justify-center rounded-lg bg-muted">
          <Settings className="h-6 w-6 text-muted-foreground" />
        </div>
        <div className="text-center">
          <h2 className="text-base font-semibold text-foreground">
            Settings
          </h2>
          <p className="mt-1 text-sm text-muted-foreground">
            Application settings will be available here.
          </p>
        </div>
      </div>
    </div>
  );
}
