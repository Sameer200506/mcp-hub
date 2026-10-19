import { useMemo, useState } from "react";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { Copy, Download, Pencil, Plus, Search, Trash2, Upload } from "lucide-react";
import {
  Badge,
  Button,
  Card,
  EmptyState,
  Input,
  Select,
  Spinner,
  Switch,
} from "@/components/ui/primitives";
import { ConfirmDialog } from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { ServerFormDialog } from "@/components/servers/ServerFormDialog";
import { SyncOneButton } from "@/components/sync/SyncController";
import { api } from "@/services/api";
import { describeError, type RegistryState } from "@/hooks/useRegistry";
import { formatDateTime, relativeTime } from "@/lib/utils";
import type { McpServer } from "@/types";

type SortKey = "name" | "updated" | "sync";

export function Servers({ registry }: { registry: RegistryState }) {
  const { toast } = useToast();
  const { servers, harnesses, loading } = registry;
  const [query, setQuery] = useState("");
  const [transport, setTransport] = useState("all");
  const [target, setTarget] = useState("all");
  const [syncState, setSyncState] = useState("all");
  const [sort, setSort] = useState<SortKey>("name");
  const [formOpen, setFormOpen] = useState(false);
  const [editing, setEditing] = useState<McpServer | null>(null);
  const [pendingDelete, setPendingDelete] = useState<McpServer | null>(null);
  const [busy, setBusy] = useState(false);

  const harnessName = (id: string) => harnesses.find((item) => item.id === id)?.name ?? id;

  const filtered = useMemo(() => {
    const text = query.trim().toLowerCase();
    const list = servers.filter((server) => {
      if (text) {
        const haystack = `${server.name} ${server.description} ${server.tags.join(" ")} ${server.command ?? ""} ${server.url ?? ""}`.toLowerCase();
        if (!haystack.includes(text)) return false;
      }
      if (transport !== "all" && server.transport !== transport) return false;
      if (target !== "all" && !server.targets.includes(target)) return false;
      if (syncState === "neverSynced" && server.lastSyncAt) return false;
      if (syncState === "synced" && !server.lastSyncAt) return false;
      if (syncState === "enabled" && !server.enabled) return false;
      if (syncState === "disabled" && server.enabled) return false;
      return true;
    });
    return list.sort((a, b) => {
      if (sort === "name") return a.name.localeCompare(b.name);
      if (sort === "updated") return b.updatedAt.localeCompare(a.updatedAt);
      const aSynced = a.lastSyncAt ?? "";
      const bSynced = b.lastSyncAt ?? "";
      return aSynced.localeCompare(bSynced);
    });
  }, [servers, query, transport, target, syncState, sort]);

  const toggleEnabled = async (server: McpServer, enabled: boolean) => {
    try {
      await api.setMcpEnabled(server.id, enabled);
      await registry.refresh();
      toast({
        title: `${enabled ? "Enabled" : "Disabled"} ${server.name}`,
        description: enabled ? undefined : "Disable is written to harnesses on the next synchronization.",
        variant: "success",
      });
    } catch (err) {
      toast({ title: "Could not toggle server", description: describeError(err), variant: "error" });
    }
  };

  const duplicate = async (server: McpServer) => {
    try {
      const copy = await api.duplicateMcpServer(server.id, `${server.name}-copy`);
      await registry.refresh();
      toast({ title: `Duplicated as ${copy.name}`, variant: "success" });
    } catch (err) {
      toast({ title: "Could not duplicate", description: describeError(err), variant: "error" });
    }
  };

  const remove = async () => {
    if (!pendingDelete) return;
    setBusy(true);
    try {
      await api.deleteMcpServer(pendingDelete.id);
      toast({
        title: `Removed ${pendingDelete.name}`,
        description: "Harness config files are not touched automatically; use preview/removal to clean them up.",
        variant: "success",
      });
      setPendingDelete(null);
      await registry.refresh();
    } catch (err) {
      toast({ title: "Could not delete", description: describeError(err), variant: "error" });
    } finally {
      setBusy(false);
    }
  };

  const exportRegistry = async () => {
    try {
      const path = await saveDialog({
        title: "Export registry (secrets omitted)",
        defaultPath: "mcp-registry.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path) return;
      const written = await api.exportRegistry({
        path,
        includeDisabled: true,
        includeLiteralEnvValues: false,
      });
      toast({ title: "Registry exported", description: written, variant: "success" });
    } catch (err) {
      toast({ title: "Export failed", description: describeError(err), variant: "error" });
    }
  };

  const importRegistry = async () => {
    try {
      const path = await openDialog({
        title: "Import registry",
        multiple: false,
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!path || Array.isArray(path)) return;
      const result = await api.importRegistry({ path, mode: "merge" });
      await registry.refresh();
      toast({
        title: `Imported ${result.imported} server(s)`,
        description: `${result.updated} updated · ${result.skipped} skipped${result.warnings.length ? ` · ${result.warnings.length} warning(s)` : ""}`,
        variant: "success",
      });
    } catch (err) {
      toast({ title: "Import failed", description: describeError(err), variant: "error" });
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center gap-2">
        <div className="relative min-w-56 flex-1">
          <Search className="pointer-events-none absolute left-2.5 top-2.5 h-4 w-4 text-muted-foreground" />
          <Input
            className="pl-8"
            placeholder="Search name, command, URL, tag…"
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            aria-label="Search MCP servers"
          />
        </div>
        <Select
          aria-label="Filter by transport"
          className="w-40"
          value={transport}
          onChange={(event) => setTransport(event.target.value)}
        >
          <option value="all">All transports</option>
          <option value="stdio">stdio</option>
          <option value="streamableHttp">Streamable HTTP</option>
          <option value="sse">SSE</option>
        </Select>
        <Select
          aria-label="Filter by target harness"
          className="w-44"
          value={target}
          onChange={(event) => setTarget(event.target.value)}
        >
          <option value="all">All harnesses</option>
          {harnesses.map((harness) => (
            <option key={harness.id} value={harness.id}>
              {harness.name}
            </option>
          ))}
        </Select>
        <Select
          aria-label="Filter by state"
          className="w-40"
          value={syncState}
          onChange={(event) => setSyncState(event.target.value)}
        >
          <option value="all">Any state</option>
          <option value="neverSynced">Never synchronized</option>
          <option value="synced">Synchronized</option>
          <option value="enabled">Enabled</option>
          <option value="disabled">Disabled</option>
        </Select>
        <Select
          aria-label="Sort servers"
          className="w-40"
          value={sort}
          onChange={(event) => setSort(event.target.value as SortKey)}
        >
          <option value="name">Sort: name</option>
          <option value="updated">Sort: recently modified</option>
          <option value="sync">Sort: sync status</option>
        </Select>
        <div className="flex gap-2">
          <Button variant="outline" onClick={() => void importRegistry()}>
            <Upload />
            Import
          </Button>
          <Button variant="outline" onClick={() => void exportRegistry()}>
            <Download />
            Export
          </Button>
          <Button
            onClick={() => {
              setEditing(null);
              setFormOpen(true);
            }}
          >
            <Plus />
            Add MCP server
          </Button>
        </div>
      </div>

      {loading ? (
        <div className="flex h-40 items-center justify-center">
          <Spinner label="Loading registry…" />
        </div>
      ) : servers.length === 0 ? (
        <EmptyState
          title="No MCP servers registered yet"
          description="Add a stdio or remote server, select target harnesses, then preview and synchronize. Secrets can be stored in the Windows Credential Manager."
          action={
            <Button
              onClick={() => {
                setEditing(null);
                setFormOpen(true);
              }}
            >
              <Plus />
              Add MCP server
            </Button>
          }
        />
      ) : filtered.length === 0 ? (
        <EmptyState title="No servers match the current filters" description="Adjust search or filters." />
      ) : (
        <Card className="overflow-hidden">
          <div className="overflow-x-auto">
            <table className="w-full text-sm">
              <thead className="bg-muted/50 text-left text-xs text-muted-foreground">
                <tr>
                  <th className="px-3 py-2 font-medium">Server</th>
                  <th className="px-3 py-2 font-medium">Transport</th>
                  <th className="px-3 py-2 font-medium">Targets</th>
                  <th className="px-3 py-2 font-medium">Sync</th>
                  <th className="px-3 py-2 font-medium">Enabled</th>
                  <th className="px-3 py-2 text-right font-medium">Actions</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-border">
                {filtered.map((server) => (
                  <tr key={server.id} className="align-top hover:bg-accent/40">
                    <td className="max-w-72 px-3 py-2">
                      <p className="truncate font-mono text-xs font-medium">{server.name}</p>
                      {server.description ? (
                        <p className="truncate text-[11px] text-muted-foreground">{server.description}</p>
                      ) : null}
                      <p className="truncate text-[11px] text-muted-foreground">
                        {server.transport === "stdio"
                          ? `${server.command ?? ""} ${server.args.join(" ")}`.trim()
                          : server.url}
                      </p>
                      {server.tags.length > 0 ? (
                        <div className="mt-1 flex flex-wrap gap-1">
                          {server.tags.map((tag) => (
                            <Badge key={tag} variant="outline">
                              {tag}
                            </Badge>
                          ))}
                        </div>
                      ) : null}
                    </td>
                    <td className="px-3 py-2 text-xs">
                      <Badge variant="secondary">{server.transport}</Badge>
                    </td>
                    <td className="px-3 py-2 text-xs">
                      {server.targets.length === 0 ? (
                        <span className="text-muted-foreground">none selected</span>
                      ) : (
                        <div className="flex flex-wrap gap-1">
                          {server.targets.map((id) => (
                            <Badge key={id} variant="outline">
                              {harnessName(id)}
                            </Badge>
                          ))}
                        </div>
                      )}
                    </td>
                    <td className="px-3 py-2 text-xs">
                      {server.lastSyncAt ? (
                        <span title={formatDateTime(server.lastSyncAt)}>{relativeTime(server.lastSyncAt)}</span>
                      ) : (
                        <Badge variant="warning">never</Badge>
                      )}
                    </td>
                    <td className="px-3 py-2">
                      <Switch
                        checked={server.enabled}
                        onCheckedChange={(checked) => void toggleEnabled(server, checked)}
                        aria-label={`Enable ${server.name}`}
                      />
                    </td>
                    <td className="px-3 py-2">
                      <div className="flex items-center justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label={`Edit ${server.name}`}
                          title="Edit"
                          onClick={() => {
                            setEditing(server);
                            setFormOpen(true);
                          }}
                        >
                          <Pencil />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label={`Duplicate ${server.name}`}
                          title="Duplicate"
                          onClick={() => void duplicate(server)}
                        >
                          <Copy />
                        </Button>
                        <SyncOneButton registry={registry} mcpId={server.id} label="" />
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label={`Delete ${server.name}`}
                          title="Delete"
                          onClick={() => setPendingDelete(server)}
                        >
                          <Trash2 />
                        </Button>
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}

      <ServerFormDialog
        open={formOpen}
        onOpenChange={setFormOpen}
        server={editing}
        harnesses={harnesses}
        onSaved={registry.refresh}
      />

      <ConfirmDialog
        open={pendingDelete !== null}
        onOpenChange={(open) => {
          if (!open) setPendingDelete(null);
        }}
        title={`Delete ${pendingDelete?.name ?? "server"}?`}
        description="The registry entry is removed. Existing harness configuration files are not modified automatically — use the Coding Agents page to preview and remove entries."
        confirmLabel="Delete definition"
        destructive
        busy={busy}
        onConfirm={() => void remove()}
      />
    </div>
  );
}
