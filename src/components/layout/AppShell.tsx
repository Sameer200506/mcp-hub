import type { ReactNode } from "react";
import {
  Activity,
  Cpu,
  LayoutDashboard,
  Moon,
  RefreshCw,
  Server,
  Settings as SettingsIcon,
  Store as StoreIcon,
  Sun,
  RefreshCcw,
} from "lucide-react";
import { Badge, Button } from "@/components/ui/primitives";
import { cn } from "@/lib/utils";
import type { RegistryState } from "@/hooks/useRegistry";
import type { Theme } from "@/hooks/useTheme";
import { SyncAllButton } from "@/components/sync/SyncController";

export type PageKey = "dashboard" | "servers" | "store" | "agents" | "history" | "settings";

const NAV: { key: PageKey; label: string; icon: ReactNode }[] = [
  { key: "dashboard", label: "Dashboard", icon: <LayoutDashboard /> },
  { key: "servers", label: "MCP Servers", icon: <Server /> },
  { key: "store", label: "MCP Store", icon: <StoreIcon /> },
  { key: "agents", label: "Coding Agents", icon: <Cpu /> },
  { key: "history", label: "Sync History", icon: <Activity /> },
  { key: "settings", label: "Settings", icon: <SettingsIcon /> },
];

export function AppShell({
  page,
  onNavigate,
  registry,
  theme,
  onThemeChange,
  children,
}: {
  page: PageKey;
  onNavigate: (page: PageKey) => void;
  registry: RegistryState;
  theme: Theme;
  onThemeChange: (theme: Theme) => void;
  children: ReactNode;
}) {
  const { stats, harnesses, detectedCount, refresh } = registry;
  const pending = stats?.pendingSyncs ?? 0;

  return (
    <div className="flex h-full min-h-0">
      <aside className="flex w-56 shrink-0 flex-col border-r border-border bg-card/60">
        <div className="flex items-center gap-2 px-4 py-4">
          <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-primary text-primary-foreground">
            <Server className="h-4 w-4" />
          </div>
          <div className="min-w-0">
            <p className="truncate text-sm font-semibold leading-tight">MCP Manager</p>
            <p className="truncate text-[11px] text-muted-foreground">Universal, local-first</p>
          </div>
        </div>

        <nav aria-label="Main navigation" className="flex flex-1 flex-col gap-0.5 px-2">
          {NAV.map((item) => (
            <button
              key={item.key}
              type="button"
              onClick={() => onNavigate(item.key)}
              aria-current={page === item.key ? "page" : undefined}
              className={cn(
                "flex items-center gap-2 rounded-md px-2.5 py-2 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-accent-foreground [&_svg]:size-4",
                page === item.key && "bg-accent font-medium text-foreground",
              )}
            >
              {item.icon}
              <span className="flex-1 text-left">{item.label}</span>
              {item.key === "agents" && detectedCount > 0 ? (
                <Badge variant="secondary">{detectedCount}</Badge>
              ) : null}
              {item.key === "servers" && (stats?.totalServers ?? 0) > 0 ? (
                <Badge variant="secondary">{stats?.totalServers}</Badge>
              ) : null}
            </button>
          ))}
        </nav>

        <div className="space-y-2 border-t border-border p-3">
          <SyncAllButton registry={registry} />
          {pending > 0 ? (
            <p className="text-[11px] text-amber-500">
              {pending} server{pending === 1 ? "" : "s"} never synchronized
            </p>
          ) : (
            <p className="text-[11px] text-muted-foreground">
              {harnesses.length} harness{harnesses.length === 1 ? "" : "es"} known
            </p>
          )}
          <Button variant="ghost" size="sm" className="w-full justify-start" onClick={() => void refresh()}>
            <RefreshCw />
            Refresh data
          </Button>
        </div>
      </aside>

      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex h-14 shrink-0 items-center justify-between gap-4 border-b border-border px-4">
          <div className="min-w-0">
            <h1 className="truncate text-sm font-semibold">
              {NAV.find((item) => item.key === page)?.label ?? "Universal MCP Manager"}
            </h1>
            <p className="truncate text-[11px] text-muted-foreground">
              Registry and sync engine are stored locally in SQLite; no cloud account required.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => onThemeChange(theme === "dark" ? "light" : "dark")}
              aria-label={`Switch to ${theme === "dark" ? "light" : "dark"} theme`}
            >
              {theme === "dark" ? <Sun /> : <Moon />}
              {theme === "dark" ? "Light" : "Dark"}
            </Button>
            <Button
              variant="outline"
              size="sm"
              onClick={() => void refresh()}
              aria-label="Rescan"
            >
              <RefreshCcw />
              Rescan
            </Button>
          </div>
        </header>
        <main className="min-h-0 flex-1 overflow-y-auto p-4">{children}</main>
      </div>
    </div>
  );
}
