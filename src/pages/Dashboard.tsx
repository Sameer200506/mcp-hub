import {
  AlertTriangle,
  CheckCircle2,
  Cpu,
  Database,
  RefreshCw,
  Server,
  Timer,
} from "lucide-react";
import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  EmptyState,
  Spinner,
} from "@/components/ui/primitives";
import { type RegistryState } from "@/hooks/useRegistry";
import { cn, formatDateTime, relativeTime } from "@/lib/utils";
import type { PageKey } from "@/components/layout/AppShell";

function StatCard({
  label,
  value,
  hint,
  icon,
  tone = "default",
}: {
  label: string;
  value: number | string;
  hint: string;
  icon: React.ReactNode;
  tone?: "default" | "success" | "warning" | "danger";
}) {
  return (
    <Card>
      <CardContent className="flex items-start justify-between gap-3 p-4">
        <div className="min-w-0">
          <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">{label}</p>
          <p className="mt-1 text-2xl font-semibold tabular-nums">{value}</p>
          <p className="mt-0.5 text-[11px] text-muted-foreground">{hint}</p>
        </div>
        <div
          className={cn(
            "flex h-8 w-8 shrink-0 items-center justify-center rounded-lg [&_svg]:h-4 [&_svg]:w-4",
            tone === "success" && "bg-emerald-500/15 text-emerald-500",
            tone === "warning" && "bg-amber-500/15 text-amber-500",
            tone === "danger" && "bg-red-500/15 text-red-500",
            tone === "default" && "bg-primary/15 text-primary",
          )}
        >
          {icon}
        </div>
      </CardContent>
    </Card>
  );
}

export function Dashboard({
  registry,
  onNavigate,
}: {
  registry: RegistryState;
  onNavigate: (page: PageKey) => void;
}) {
  const { stats, harnesses, servers, loading, error } = registry;

  if (loading) {
    return (
      <div className="flex h-40 items-center justify-center">
        <Spinner label="Loading registry and harness state…" />
      </div>
    );
  }

  if (error || !stats) {
    return (
      <EmptyState
        icon={<AlertTriangle className="h-5 w-5" />}
        title="Could not load application state"
        description={error ?? "Unknown error"}
        action={
          <Button variant="outline" onClick={() => void registry.refresh()}>
            Retry
          </Button>
        }
      />
    );
  }

  const externallyModified = harnesses.filter((harness) => harness.externallyModified);
  const harnessWithErrors = harnesses.filter((harness) => harness.lastError);

  return (
    <div className="space-y-4">
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <StatCard
          label="Registered MCPs"
          value={stats.totalServers}
          hint={`${stats.enabledServers} enabled · ${stats.disabledServers} disabled`}
          icon={<Server />}
        />
        <StatCard
          label="Detected harnesses"
          value={stats.detectedHarnesses}
          hint={`${stats.supportedHarnesses} adapters available`}
          icon={<Cpu />}
          tone="default"
        />
        <StatCard
          label="Successful syncs"
          value={stats.successfulSyncs}
          hint={stats.lastSyncAt ? `last ${relativeTime(stats.lastSyncAt)}` : "no sync recorded yet"}
          icon={<CheckCircle2 />}
          tone="success"
        />
        <StatCard
          label="Failed / pending"
          value={`${stats.failedSyncs} / ${stats.pendingSyncs}`}
          hint="failed writes · servers never synchronized"
          icon={<AlertTriangle />}
          tone={stats.failedSyncs > 0 ? "danger" : stats.pendingSyncs > 0 ? "warning" : "default"}
        />
      </div>

      {externallyModified.length > 0 ? (
        <Card className="border-amber-500/50">
          <CardHeader>
            <CardTitle className="flex items-center gap-2 text-amber-500">
              <AlertTriangle className="h-4 w-4" />
              External configuration changes detected
            </CardTitle>
            <CardDescription>
              These files changed outside Universal MCP Manager. Nothing was overwritten automatically; inspect
              the differences before resynchronizing.
            </CardDescription>
          </CardHeader>
          <CardContent className="flex flex-wrap gap-2">
            {externallyModified.map((harness) => (
              <Button key={harness.id} size="sm" variant="outline" onClick={() => onNavigate("agents")}>
                Inspect {harness.name}
              </Button>
            ))}
          </CardContent>
        </Card>
      ) : null}

      <div className="grid grid-cols-1 gap-3 lg:grid-cols-2">
        <Card>
          <CardHeader className="flex-row items-center justify-between">
            <div>
              <CardTitle>Recently modified servers</CardTitle>
              <CardDescription>Ordered by registry modification time.</CardDescription>
            </div>
            <Button size="sm" variant="ghost" onClick={() => onNavigate("servers")}>
              Manage
            </Button>
          </CardHeader>
          <CardContent>
            {stats.recentServers.length === 0 ? (
              <EmptyState
                title="No MCP servers registered"
                description="Add your first MCP server, choose the harnesses it should be written to, then preview and synchronize."
                action={
                  <Button size="sm" onClick={() => onNavigate("servers")}>
                    Add MCP server
                  </Button>
                }
              />
            ) : (
              <ul className="divide-y divide-border">
                {stats.recentServers.slice(0, 6).map((server) => (
                  <li key={server.id} className="flex items-center justify-between gap-3 py-2">
                    <div className="min-w-0">
                      <p className="truncate font-mono text-xs">{server.name}</p>
                      <p className="text-[11px] text-muted-foreground">
                        modified {relativeTime(server.updatedAt)} · synced {relativeTime(server.lastSyncAt)}
                      </p>
                    </div>
                    {server.lastSyncAt ? (
                      <Badge variant="success">synced</Badge>
                    ) : (
                      <Badge variant="warning">never synced</Badge>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Harness health</CardTitle>
            <CardDescription>
              A harness is “detected” when its installation/configuration location exists. This is independent
              from whether a server actually runs.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {harnessWithErrors.length > 0 ? (
              <div className="mb-3 space-y-1 rounded-md border border-red-500/40 bg-red-500/5 p-2">
                {harnessWithErrors.map((harness) => (
                  <p key={harness.id} className="text-xs text-red-500">
                    {harness.name}: {harness.lastError}
                  </p>
                ))}
              </div>
            ) : null}
            <ul className="divide-y divide-border">
              {harnesses.map((harness) => (
                <li key={harness.id} className="flex items-center justify-between gap-3 py-2">
                  <div className="min-w-0">
                    <p className="truncate text-xs font-medium">{harness.name}</p>
                    <p className="truncate text-[11px] text-muted-foreground">
                      {harness.configPath ?? "not detected"} · last sync {relativeTime(harness.lastSyncAt)}
                    </p>
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    {harness.adapterStatus === "pendingVerification" ? (
                      <Badge variant="outline">adapter pending verification</Badge>
                    ) : null}
                    <Badge variant={harness.detected ? "success" : "outline"}>
                      {harness.detected ? "detected" : "not found"}
                    </Badge>
                  </div>
                </li>
              ))}
            </ul>
            {servers.length > 0 ? (
              <p className="mt-3 flex items-center gap-1.5 text-[11px] text-muted-foreground">
                <Database className="h-3 w-3" />
                {servers.length} registry definitions loaded from the local SQLite database.
              </p>
            ) : null}
            <p className="mt-2 flex items-center gap-1.5 text-[11px] text-muted-foreground">
              <Timer className="h-3 w-3" />
              Last sync activity: {formatDateTime(stats.lastSyncAt)}
            </p>
            {stats.recentErrors.length > 0 ? (
              <div className="mt-3 space-y-1">
                <p className="text-xs font-medium text-red-500">Recent errors</p>
                {stats.recentErrors.slice(0, 5).map((item, index) => (
                  <p key={index} className="text-[11px] text-muted-foreground">
                    <span className="font-mono">{item.harnessId}</span> · {relativeTime(item.at)} · {item.message}
                  </p>
                ))}
              </div>
            ) : null}
            <div className="mt-3 flex gap-2">
              <Button size="sm" variant="outline" onClick={() => void registry.refresh()}>
                <RefreshCw />
                Refresh
              </Button>
              <Button size="sm" variant="ghost" onClick={() => onNavigate("agents")}>
                Open harness details
              </Button>
            </div>
          </CardContent>
        </Card>
      </div>
    </div>
  );
}
