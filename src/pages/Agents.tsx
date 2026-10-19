import { useCallback, useEffect, useState } from "react";
import { openPath, revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  AlertTriangle,
  CheckCircle2,
  FolderOpen,
  FileJson2,
  RefreshCcw,
  ShieldCheck,
  Unplug,
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
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { JsonPreview } from "@/components/ui/data-display";
import { useToast } from "@/components/ui/toast";
import { HarnessSyncButton, RemovalsPreviewButton } from "@/components/sync/SyncController";
import { api } from "@/services/api";
import { describeError, type RegistryState } from "@/hooks/useRegistry";
import { formatDateTime, relativeTime } from "@/lib/utils";
import type { ConfigInspection, HarnessEntries, HarnessInfo } from "@/types";

export function Agents({ registry }: { registry: RegistryState }) {
  const { toast } = useToast();
  const { harnesses, loading } = registry;
  const [detecting, setDetecting] = useState(false);
  const [inspecting, setInspecting] = useState<HarnessInfo | null>(null);

  const rescan = async () => {
    setDetecting(true);
    try {
      await api.detectHarnesses();
      await registry.refresh();
      toast({ title: "Harness detection complete", variant: "success" });
    } catch (err) {
      toast({ title: "Detection failed", description: describeError(err), variant: "error" });
    } finally {
      setDetecting(false);
    }
  };

  const openDirectory = async (harness: HarnessInfo) => {
    if (!harness.configPath) return;
    try {
      await revealItemInDir(harness.configPath);
    } catch {
      const directory = harness.configPath.replace(/[\\/][^\\/]*$/, "");
      try {
        await openPath(directory);
      } catch (err) {
        toast({ title: "Could not open directory", description: describeError(err), variant: "error" });
      }
    }
  };

  if (loading && harnesses.length === 0) {
    return (
      <div className="flex h-40 items-center justify-center">
        <Spinner label="Detecting harnesses…" />
      </div>
    );
  }

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="max-w-3xl text-xs text-muted-foreground">
          Each adapter declares which transports and fields it supports. Detection only reports what exists on
          disk — a configured entry is not evidence that a server runs or connects. Adapters marked{" "}
          <span className="font-medium text-foreground">pending verification</span> are not written to until you
          review the preview.
        </p>
        <Button variant="outline" onClick={() => void rescan()} loading={detecting}>
          <RefreshCcw />
          Rescan harnesses
        </Button>
      </div>

      <div className="grid grid-cols-1 gap-3 xl:grid-cols-2">
        {harnesses.map((harness) => (
          <Card key={harness.id} className={harness.externallyModified ? "border-amber-500/60" : undefined}>
            <CardHeader>
              <div className="flex items-start justify-between gap-3">
                <div className="min-w-0">
                  <CardTitle className="flex items-center gap-2">
                    {harness.name}
                    {harness.detected ? (
                      <Badge variant="success">
                        <CheckCircle2 className="h-3 w-3" /> detected
                      </Badge>
                    ) : (
                      <Badge variant="outline">
                        <Unplug className="h-3 w-3" /> not detected
                      </Badge>
                    )}
                    {harness.adapterStatus === "pendingVerification" ? (
                      <Badge variant="warning">
                        <ShieldCheck className="h-3 w-3" /> pending verification
                      </Badge>
                    ) : (
                      <Badge variant="secondary">verified format</Badge>
                    )}
                  </CardTitle>
                  <CardDescription className="mt-1 break-all font-mono">
                    {harness.configPath ?? "No configuration location resolved"}
                  </CardDescription>
                </div>
                <div className="shrink-0 text-right text-[11px] text-muted-foreground">
                  <p>format: {harness.configFormat}</p>
                  <p>confidence: {harness.detectionConfidence}</p>
                  {harness.installedVersion ? <p>version: {harness.installedVersion}</p> : null}
                </div>
              </div>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="grid grid-cols-2 gap-2 text-[11px] sm:grid-cols-4">
                <div className="rounded-md bg-muted/40 p-2">
                  <p className="text-muted-foreground">Assigned MCPs</p>
                  <p className="text-sm font-semibold tabular-nums">{harness.assignedMcpCount}</p>
                </div>
                <div className="rounded-md bg-muted/40 p-2">
                  <p className="text-muted-foreground">Last sync</p>
                  <p className="text-sm font-semibold">{relativeTime(harness.lastSyncAt)}</p>
                </div>
                <div className="rounded-md bg-muted/40 p-2">
                  <p className="text-muted-foreground">Config exists</p>
                  <p className="text-sm font-semibold">{harness.configExists ? "yes" : "no"}</p>
                </div>
                <div className="rounded-md bg-muted/40 p-2">
                  <p className="text-muted-foreground">Reload</p>
                  <p className="text-sm font-semibold">{harness.reload.reloadRequired ? "may be required" : "not needed"}</p>
                </div>
              </div>

              <div className="flex flex-wrap gap-1">
                {harness.capabilities.stdio ? <Badge variant="outline">stdio</Badge> : null}
                {harness.capabilities.streamableHttp ? <Badge variant="outline">streamable HTTP</Badge> : null}
                {harness.capabilities.sse ? <Badge variant="outline">SSE</Badge> : null}
                {harness.capabilities.headers ? <Badge variant="outline">headers</Badge> : null}
                {harness.capabilities.env ? <Badge variant="outline">env</Badge> : null}
                {harness.capabilities.cwd ? <Badge variant="outline">cwd</Badge> : null}
                {harness.capabilities.enabledFlag ? <Badge variant="outline">enable flag</Badge> : null}
              </div>

              <p className="text-[11px] text-muted-foreground">{harness.reload.description}</p>

              {harness.notes.length > 0 ? (
                <ul className="space-y-0.5">
                  {harness.notes.map((note, index) => (
                    <li key={index} className="text-[11px] text-muted-foreground">
                      • {note}
                    </li>
                  ))}
                </ul>
              ) : null}

              {harness.externallyModified ? (
                <p className="flex items-start gap-2 rounded-md border border-amber-500/50 bg-amber-500/5 p-2 text-[11px] text-amber-600 dark:text-amber-400">
                  <AlertTriangle className="mt-0.5 h-3 w-3 shrink-0" />
                  This file changed outside the application. Nothing was overwritten — inspect the file and
                  resynchronize when ready.
                </p>
              ) : null}

              {harness.lastError ? (
                <p className="rounded-md border border-red-500/40 bg-red-500/5 p-2 text-[11px] text-red-500">
                  {harness.lastError}
                </p>
              ) : null}

              <div className="flex flex-wrap items-center gap-2">
                <Button variant="outline" size="sm" onClick={() => setInspecting(harness)}>
                  <FileJson2 />
                  Inspect config
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => void openDirectory(harness)}
                  disabled={!harness.configPath}
                >
                  <FolderOpen />
                  Open directory
                </Button>
                <HarnessSyncButton registry={registry} harnessId={harness.id} />
              </div>
            </CardContent>
          </Card>
        ))}
      </div>

      {harnesses.length === 0 ? (
        <EmptyState
          title="No adapters registered"
          description="This build ships with OpenCode, Cline, Antigravity and a generic JSON adapter. If nothing is listed, the backend failed to initialize."
        />
      ) : null}

      <ConfigInspector
        harness={inspecting}
        onClose={() => setInspecting(null)}
        registry={registry}
      />
    </div>
  );
}

function ConfigInspector({
  harness,
  onClose,
  registry,
}: {
  harness: HarnessInfo | null;
  onClose: () => void;
  registry: RegistryState;
}) {
  const [inspection, setInspection] = useState<ConfigInspection | null>(null);
  const [entries, setEntries] = useState<HarnessEntries | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    if (!harness) return;
    setLoading(true);
    try {
      const [config, parsed] = await Promise.all([
        api.getHarnessConfig(harness.id),
        api.listHarnessEntries(harness.id),
      ]);
      setInspection(config);
      setEntries(parsed);
      setError(null);
    } catch (err) {
      setError(describeError(err));
    } finally {
      setLoading(false);
    }
  }, [harness]);

  useEffect(() => {
    if (harness) {
      setInspection(null);
      setEntries(null);
      void load();
    }
  }, [harness, load]);

  const unmanaged = entries?.entries.filter((entry) => !entry.managed).map((entry) => entry.name) ?? [];

  return (
    <Dialog open={harness !== null} onOpenChange={(open) => (!open ? onClose() : undefined)}>
      <DialogContent
        title={harness ? `${harness.name} configuration` : "Configuration"}
        description={inspection?.path ?? harness?.configPath ?? undefined}
        size="lg"
        footer={
          <>
            {harness && unmanaged.length > 0 ? (
              <RemovalsPreviewButton
                registry={registry}
                harnessId={harness.id}
                names={unmanaged}
                label={`Remove unmanaged entries (${unmanaged.length})`}
              />
            ) : null}
            <Button variant="outline" onClick={() => void load()} loading={loading}>
              Refresh
            </Button>
            <Button onClick={onClose}>Close</Button>
          </>
        }
      >
        {error ? <p className="text-sm text-red-500">{error}</p> : null}
        {loading && !inspection ? <Spinner label="Reading configuration…" /> : null}
        {inspection ? (
          <div className="space-y-3">
            <div className="grid grid-cols-2 gap-2 text-[11px] sm:grid-cols-4">
              <div className="rounded-md bg-muted/40 p-2">
                <p className="text-muted-foreground">Exists</p>
                <p className="font-semibold">{inspection.exists ? "yes" : "no"}</p>
              </div>
              <div className="rounded-md bg-muted/40 p-2">
                <p className="text-muted-foreground">MCP entries</p>
                <p className="font-semibold">{inspection.mcpEntryCount}</p>
              </div>
              <div className="rounded-md bg-muted/40 p-2">
                <p className="text-muted-foreground">Format</p>
                <p className="font-semibold">{inspection.format}</p>
              </div>
              <div className="rounded-md bg-muted/40 p-2">
                <p className="text-muted-foreground">Read-only</p>
                <p className="font-semibold">{inspection.readOnly ? "yes" : "no"}</p>
              </div>
            </div>
            {inspection.malformed ? (
              <p className="rounded-md border border-red-500/40 bg-red-500/5 p-2 text-xs text-red-500">
                Malformed configuration: {inspection.parseError}. The file will not be overwritten automatically.
              </p>
            ) : null}
            {entries && entries.entries.length > 0 ? (
              <div className="space-y-1">
                <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
                  Entries in this file
                </p>
                <ul className="divide-y divide-border rounded-md border border-border">
                  {entries.entries.map((entry) => (
                    <li key={entry.name} className="flex items-center justify-between gap-2 p-2 text-xs">
                      <span className="min-w-0">
                        <span className="font-mono">{entry.name}</span>
                        <span className="block truncate text-[11px] text-muted-foreground">{entry.summary}</span>
                      </span>
                      <span className="flex shrink-0 items-center gap-1">
                        {entry.enabled ? (
                          <Badge variant="secondary">enabled</Badge>
                        ) : (
                          <Badge variant="outline">disabled</Badge>
                        )}
                        <Badge variant={entry.managed ? "success" : "outline"}>
                          {entry.managed ? "managed" : "unmanaged"}
                        </Badge>
                      </span>
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
            <div className="space-y-1">
              <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Raw file</p>
              <JsonPreview value={inspection.pretty ?? inspection.raw} />
            </div>
            {inspection.sha256 ? (
              <p className="break-all text-[11px] text-muted-foreground">sha256: {inspection.sha256}</p>
            ) : null}
            <p className="text-[11px] text-muted-foreground">
              Last registry sync for this harness: {formatDateTime(harness?.lastSyncAt)}
            </p>
          </div>
        ) : null}
      </DialogContent>
    </Dialog>
  );
}
