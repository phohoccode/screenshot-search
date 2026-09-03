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
  type LucideIcon,
} from "lucide-react";
import { useTheme } from "@/hooks/use-theme";

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
