import { useState } from "react";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Sidebar, type NavSection } from "@/components/layout/sidebar";
import { SearchPage } from "@/features/search/search-page";
import { FoldersPage } from "@/features/folders/folders-page";
import { IndexingPage } from "@/features/indexing/indexing-page";
import { SettingsPage } from "@/features/settings/settings-page";

function PageContent({ section }: { section: NavSection }) {
  switch (section) {
    case "search":
      return <SearchPage />;
    case "folders":
      return <FoldersPage />;
    case "indexing":
      return <IndexingPage />;
    case "settings":
      return <SettingsPage />;
  }
}

export function AppShell() {
  const [activeSection, setActiveSection] = useState<NavSection>("search");

  return (
    <TooltipProvider delayDuration={300}>
      <div className="flex h-screen w-screen overflow-hidden">
        <Sidebar
          activeSection={activeSection}
          onSectionChange={setActiveSection}
        />
        <main className="flex-1 overflow-y-auto">
          <PageContent section={activeSection} />
        </main>
      </div>
    </TooltipProvider>
  );
}
