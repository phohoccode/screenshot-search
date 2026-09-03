import { useEffect, useState } from "react";
import { cn } from "@/lib/utils";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import {
  Search,
  FolderOpen,
  Database,
  Settings,
  Sun,
  Moon,
  Loader2,
  type LucideIcon,
} from "lucide-react";
import { useTheme } from "@/hooks/use-theme";
import { getIndexingStatus, onIndexingCompleted } from "@/lib/tauri";
import type { IndexingServiceStatus } from "@/types";

export type NavSection = "search" | "folders" | "indexing" | "settings";

interface SidebarProps {
  activeSection: NavSection;
  onSectionChange: (section: NavSection) => void;
}

interface NavItem {
  id: NavSection;
  label: string;
  icon: LucideIcon;
}

const navItems: NavItem[] = [
  { id: "search", label: "Search", icon: Search },
  { id: "folders", label: "Folders", icon: FolderOpen },
  { id: "indexing", label: "Indexing", icon: Database },
  { id: "settings", label: "Settings", icon: Settings },
];

export function Sidebar({ activeSection, onSectionChange }: SidebarProps) {
  const { resolvedTheme, setTheme } = useTheme();
  const [indexingStatus, setIndexingStatus] = useState<IndexingServiceStatus | null>(null);

  useEffect(() => {
    getIndexingStatus().then(setIndexingStatus).catch(() => {});
    let unlisten: (() => void) | undefined;
    onIndexingCompleted(() => {
      getIndexingStatus().then(setIndexingStatus).catch(() => {});
    }).then((fn) => {
      unlisten = fn;
    });

    const interval = setInterval(() => {
      getIndexingStatus().then(setIndexingStatus).catch(() => {});
    }, 4000);

    return () => {
      if (unlisten) unlisten();
      clearInterval(interval);
    };
  }, []);

  return (
    <aside className="flex h-full w-12 flex-col items-center border-r border-sidebar-border bg-sidebar py-2">
      <nav className="flex flex-1 flex-col items-center gap-1">
        {navItems.map((item) => {
          const Icon = item.icon;
          const isActive = activeSection === item.id;
          return (
            <Tooltip key={item.id}>
              <TooltipTrigger asChild>
                <Button
                  variant="ghost"
                  size="icon"
                  aria-label={item.label}
                  aria-current={isActive ? "page" : undefined}
                  className={cn(
                    "h-9 w-9 text-sidebar-foreground",
                    isActive &&
                      "bg-sidebar-accent text-sidebar-accent-foreground"
                  )}
                  onClick={() => onSectionChange(item.id)}
                >
                  <Icon className="h-[18px] w-[18px]" />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="right">{item.label}</TooltipContent>
            </Tooltip>
          );
        })}
      </nav>

      <div className="flex flex-col items-center gap-1 pb-1">
        {/* Subtle Background Indexing Indicator */}
        {indexingStatus?.stats.pending ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <div className="flex h-8 w-8 items-center justify-center cursor-default">
                <Loader2 className="h-3.5 w-3.5 animate-spin text-primary" />
              </div>
            </TooltipTrigger>
            <TooltipContent side="right">
              Indexing {indexingStatus.stats.pending} screenshot(s)...
            </TooltipContent>
          </Tooltip>
        ) : (
          <Tooltip>
            <TooltipTrigger asChild>
              <div className="flex h-8 w-8 items-center justify-center cursor-default">
                <span className="h-1.5 w-1.5 rounded-full bg-emerald-500" />
              </div>
            </TooltipTrigger>
            <TooltipContent side="right">All screenshots indexed</TooltipContent>
          </Tooltip>
        )}

        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon"
              aria-label={
                resolvedTheme === "dark"
                  ? "Switch to light theme"
                  : "Switch to dark theme"
              }
              className="h-9 w-9 text-sidebar-foreground"
              onClick={() =>
                setTheme(resolvedTheme === "dark" ? "light" : "dark")
              }
            >
              {resolvedTheme === "dark" ? (
                <Sun className="h-[18px] w-[18px]" />
              ) : (
                <Moon className="h-[18px] w-[18px]" />
              )}
            </Button>
          </TooltipTrigger>
          <TooltipContent side="right">
            {resolvedTheme === "dark" ? "Light theme" : "Dark theme"}
          </TooltipContent>
        </Tooltip>
      </div>
    </aside>
  );
}
