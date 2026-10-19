import { useMemo, useState } from "react";
import {
  AlertTriangle,
  CheckCircle2,
  GitCompareArrows,
  Minus,
  RefreshCw,
  XCircle,
} from "lucide-react";
import { Badge, Button, Card, CardContent, Input, Select, Switch } from "@/components/ui/primitives";
import { Dialog, DialogContent } from "@/components/ui/dialog";
import { DiffViewer, JsonPreview, Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/data-display";
import { useToast } from "@/components/ui/toast";
import { api } from "@/services/api";
import { describeError, type RegistryState } from "@/hooks/useRegistry";
import { formatDateTime, relativeTime } from "@/lib/utils";
import type {
  DiffEntry,
  HarnessSyncResult,
  PreviewReport,
  RemovalRequest,
  ResolutionSelection,
  SyncPreview,
  SyncReport,
  SyncRequest,
} from "@/types";

const ACTION_VARIANT: Record<
  DiffEntry["action"],
  "success" | "warning" | "secondary" | "danger" | "outline"
> = {
  add: "success",
  update: "warning",
  unchanged: "secondary",
  conflict: "danger",
  skip: "outline",
  remove: "danger",
};

function buildRequest(partial: Partial<SyncRequest>): SyncRequest {
  return {
    harnessIds: null,
    mcpIds: null,
    resolutions: [],
    removals: [],
    confirmRemovals: false,
    ...partial,
  };
}

function resolutionKey(harnessId: string, name: string) {
  return `${harnessId}::${name}`;
}

function useSyncFlow(registry: RegistryState) {
  const { toast } = useToast();
  const [preview, setPreview] = useState<PreviewReport | null>(null);
  const [report, setReport] = useState<SyncReport | null>(null);
  const [pending, setPending] = useState(false);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [reportOpen, setReportOpen] = useState(false);
  const [lastRequest, setLastRequest] = useState<SyncRequest | null>(null);
  const [resolutions, setResolutions] = useState<Record<string, ResolutionSelection>>({});
  const [approvedRemovals, setApprovedRemovals] = useState<Record<string, boolean>>({});

  const startPreview = async (request: SyncRequest) => {
    setPending(true);
    try {
      const result = await api.previewSync(request);
      setLastRequest(request);
      setPreview(result);
      setResolutions({});
      setApprovedRemovals(
        Object.fromEntries(request.removals.map((item) => [resolutionKey(item.harnessId, item.name), false])),
      );
      setPreviewOpen(true);
      if (result.overall === "noChange") {
        toast({
          title: "Nothing to synchronize",
          description: "Every selected target is already up to date.",
          variant: "info",
        });
      }
    } catch (err) {
      toast({ title: "Preview failed", description: describeError(err), variant: "error" });
    } finally {
      setPending(false);
    }
  };

  const apply = async () => {
    if (!lastRequest || !preview) return;
    setPending(true);
    try {
      const removals: RemovalRequest[] = lastRequest.removals.filter(
        (item) => approvedRemovals[resolutionKey(item.harnessId, item.name)],
      );
      const request: SyncRequest = {
        ...lastRequest,
        resolutions: Object.values(resolutions),
        removals,
        confirmRemovals: removals.length > 0,
      };
      const result = await api.syncAll(request);
      setPreviewOpen(false);
      setReport(result);
      setReportOpen(true);
      await registry.refresh();
      const messages = {
        success: { title: "Synchronization complete", variant: "success" as const },
        noChange: { title: "No changes were needed", variant: "info" as const },
        partial: { title: "Partially synchronized", variant: "warning" as const },
        failed: { title: "Synchronization failed", variant: "error" as const },
      };
      const message = messages[result.overall];
      toast({ title: message.title, variant: message.variant, description: describeReport(result) });
    } catch (err) {
      toast({ title: "Synchronization failed", description: describeError(err), variant: "error" });
    } finally {
      setPending(false);
    }
  };

  return {
    preview,
    report,
    pending,
    previewOpen,
    reportOpen,
    resolvedRemovals: approvedRemovals,
    resolutions,
    setResolutions,
    setApprovedRemovals,
    setPreviewOpen,
    setReportOpen,
    startPreview,
    apply,
  };
}

function describeReport(report: SyncReport): string {
  const failed = report.results.filter((item) => item.status === "failed").length;
  const changed = report.results.filter((item) => item.applied.length > 0).length;
  if (failed > 0) return `${failed} harness(es) failed, ${changed} changed.`;
  return `${changed} harness(es) written and verified.`;
}

export type SyncFlow = ReturnType<typeof useSyncFlow>;

export function SyncAllButton({ registry }: { registry: RegistryState }) {
  const flow = useSyncFlow(registry);
  const neverSynced = registry.stats?.pendingSyncs ?? 0;
  return (
    <>
      <Button
        className="w-full"
        onClick={() => void flow.startPreview(buildRequest({}))}
        loading={flow.pending}
        title="Preview then apply all registry entries to their selected harnesses"
      >
        <RefreshCw />
        Sync All
        {neverSynced > 0 ? <Badge variant="secondary">{neverSynced}</Badge> : null}
      </Button>
      <SyncDialogs flow={flow} />
    </>
  );
}

export function HarnessSyncButton({
  registry,
  harnessId,
  label = "Synchronize",
}: {
  registry: RegistryState;
  harnessId: string;
  label?: string;
}) {
  const flow = useSyncFlow(registry);
  return (
    <>
      <Button
        variant="outline"
        size="sm"
        onClick={() => void flow.startPreview(buildRequest({ harnessIds: [harnessId] }))}
        loading={flow.pending}
      >
        <GitCompareArrows />
        {label}
      </Button>
      <SyncDialogs flow={flow} />
    </>
  );
}

export function SyncOneButton({
  registry,
  mcpId,
  label = "Sync",
}: {
  registry: RegistryState;
  mcpId: string;
  label?: string;
}) {
  const flow = useSyncFlow(registry);
  return (
    <>
      <Button
        variant="outline"
        size="sm"
        onClick={() => void flow.startPreview(buildRequest({ mcpIds: [mcpId] }))}
        loading={flow.pending}
        title="Preview and synchronize this server to the harnesses it targets"
      >
        <GitCompareArrows />
        {label}
      </Button>
      <SyncDialogs flow={flow} />
    </>
  );
}

export function RemovalsPreviewButton({
  registry,
  harnessId,
  names,
  label = "Review removals",
}: {
  registry: RegistryState;
  harnessId: string;
  names: string[];
  label?: string;
}) {
  const flow = useSyncFlow(registry);
  return (
    <>
      <Button
        variant="destructive"
        size="sm"
        onClick={() =>
          void flow.startPreview(
            buildRequest({
              harnessIds: [harnessId],
              removals: names.map((name) => ({ harnessId, name })),
            }),
          )
        }
        loading={flow.pending}
      >
        {label}
      </Button>
      <SyncDialogs flow={flow} />
    </>
  );
}

function SyncDialogs({ flow }: { flow: SyncFlow }) {
  return (
    <>
      <SyncPreviewDialog flow={flow} />
      <SyncReportDialog flow={flow} />
    </>
  );
}

function SyncPreviewDialog({ flow }: { flow: SyncFlow }) {
  const { preview } = flow;
  const actionable = useMemo(
    () => (preview?.previews ?? []).filter((item) => item.status === "ready" || item.status === "conflict"),
    [preview],
  );
  const totalChanges = actionable.reduce(
    (sum, item) => sum + item.entries.filter((entry) => entry.action !== "unchanged" && entry.action !== "skip").length,
    0,
  );
  const conflictCount = actionable.reduce((sum, item) => sum + item.conflicts.length, 0);
  const blocked = preview?.overall === "blocked";

  if (!preview) return null;

  return (
    <Dialog open={flow.previewOpen} onOpenChange={flow.setPreviewOpen}>
      <DialogContent
        title="Configuration preview"
        description={`${totalChanges} planned change(s) across ${actionable.length} harness(es). Nothing is written until you confirm.`}
        size="xl"
        footer={
          <>
            <p className="mr-auto max-w-md text-xs text-muted-foreground">
              A timestamped backup is created before every write and the result is re-read and validated
              afterwards. Malformed JSON is never overwritten.
            </p>
            <Button variant="outline" onClick={() => flow.setPreviewOpen(false)}>
              Cancel
            </Button>
            <Button
              onClick={() => void flow.apply()}
              loading={flow.pending}
              disabled={actionable.length === 0 || blocked}
              title={
                conflictCount > 0
                  ? "Resolve or skip every conflict before applying"
                  : "Write the proposed configuration"
              }
            >
              Apply {totalChanges > 0 ? `${totalChanges} change${totalChanges === 1 ? "" : "s"}` : ""}
            </Button>
          </>
        }
      >
        {preview.previews.length === 0 ? (
          <p className="text-sm text-muted-foreground">
            No detected harness matched the request. Run a rescan from Coding Agents first.
          </p>
        ) : null}
        <Tabs defaultValue={preview.previews[0]?.harnessId}>
          <TabsList className="mb-3 flex-wrap">
            {preview.previews.map((item) => (
              <TabsTrigger key={item.harnessId} value={item.harnessId}>
                {item.harnessName}
                {item.conflicts.length > 0 ? (
                  <AlertTriangle className="h-3 w-3 text-amber-500" aria-label="conflicts" />
                ) : null}
              </TabsTrigger>
            ))}
          </TabsList>
          {preview.previews.map((item) => (
            <TabsContent key={item.harnessId} value={item.harnessId} className="space-y-4">
              <HarnessPreview flow={flow} preview={item} />
            </TabsContent>
          ))}
        </Tabs>
      </DialogContent>
    </Dialog>
  );
}

function HarnessPreview({ flow, preview }: { flow: SyncFlow; preview: SyncPreview }) {
  return (
    <>
      <div className="flex flex-wrap items-center gap-2 text-xs text-muted-foreground">
        <Badge
          variant={
            preview.status === "error"
              ? "danger"
              : preview.status === "conflict"
                ? "warning"
                : preview.status === "noChange"
                  ? "secondary"
                  : preview.status === "unsupported"
                    ? "outline"
                    : "success"
          }
        >
          {preview.status}
        </Badge>
        {preview.configPath ? <span className="font-mono break-all">{preview.configPath}</span> : null}
        {preview.readOnly ? <Badge variant="danger">read-only</Badge> : null}
        {preview.malformed ? <Badge variant="danger">malformed JSON</Badge> : null}
      </div>

      {preview.warnings.map((warning, index) => (
        <p key={index} className="rounded-md border border-amber-500/40 bg-amber-500/5 p-2 text-xs text-amber-600 dark:text-amber-400">
          {warning}
        </p>
      ))}

      <Card>
        <CardContent className="space-y-2 p-3">
          <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Entries</p>
          {preview.entries.length === 0 ? (
            <p className="text-xs text-muted-foreground">No registry entries target this harness.</p>
          ) : (
            <ul className="space-y-1.5">
              {preview.entries.map((entry) => (
                <li key={`${entry.name}-${entry.action}`} className="flex items-start gap-2 text-xs">
                  <Badge variant={ACTION_VARIANT[entry.action] ?? "outline"}>{entry.action}</Badge>
                  <span className="min-w-0 flex-1">
                    <span className="font-mono">{entry.name}</span>
                    <span className="text-muted-foreground"> — {entry.details}</span>
                  </span>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      {preview.conflicts.length > 0 ? (
        <Card className="border-amber-500/50">
          <CardContent className="space-y-3 p-3">
            <p className="text-xs font-medium uppercase tracking-wide text-amber-500">
              Conflicts need a decision
            </p>
            {preview.conflicts.map((conflict) => {
              const key = resolutionKey(preview.harnessId, conflict.name);
              const current = flow.resolutions[key];
              return (
                <div key={conflict.name} className="space-y-1.5 rounded-md border border-border p-2">
                  <p className="text-xs">
                    <span className="font-mono">{conflict.name}</span>{" "}
                    <span className="text-muted-foreground">({conflict.kind})</span>
                  </p>
                  <p className="text-[11px] text-muted-foreground">{conflict.message}</p>
                  <div className="flex flex-wrap items-center gap-2">
                    <Select
                      aria-label={`Resolution for ${conflict.name}`}
                      className="h-8 w-44 text-xs"
                      value={current?.action ?? "cancel"}
                      onChange={(event) =>
                        flow.setResolutions((prev) => ({
                          ...prev,
                          [key]: {
                            harnessId: preview.harnessId,
                            name: conflict.name,
                            action: event.target.value as ResolutionSelection["action"],
                            newName: current?.newName ?? null,
                          },
                        }))
                      }
                    >
                      <option value="cancel">Do not synchronize this entry</option>
                      <option value="update">Update the existing entry</option>
                      <option value="skip">Keep existing (skip)</option>
                      <option value="rename">Add under a new name</option>
                    </Select>
                    {current?.action === "rename" ? (
                      <Input
                        className="h-8 w-52 text-xs"
                        placeholder="new-server-name"
                        aria-label={`New name for ${conflict.name}`}
                        value={current.newName ?? ""}
                        onChange={(event) =>
                          flow.setResolutions((prev) => ({
                            ...prev,
                            [key]: {
                              harnessId: preview.harnessId,
                              name: conflict.name,
                              action: "rename",
                              newName: event.target.value,
                            },
                          }))
                        }
                      />
                    ) : null}
                  </div>
                </div>
              );
            })}
          </CardContent>
        </Card>
      ) : null}

      {preview.entries.some((entry) => entry.action === "remove") ? (
        <Card className="border-red-500/50">
          <CardContent className="space-y-2 p-3">
            <p className="text-xs font-medium uppercase tracking-wide text-red-500">Confirm removals</p>
            {preview.entries
              .filter((entry) => entry.action === "remove")
              .map((entry) => {
                const key = resolutionKey(preview.harnessId, entry.name);
                return (
                  <label key={entry.name} className="flex items-center justify-between gap-3 text-xs">
                    <span>
                      Remove <span className="font-mono">{entry.name}</span> from this harness
                    </span>
                    <Switch
                      checked={flow.resolvedRemovals[key] ?? false}
                      onCheckedChange={(checked) =>
                        flow.setApprovedRemovals((prev) => ({ ...prev, [key]: checked }))
                      }
                      aria-label={`Confirm removal of ${entry.name}`}
                    />
                  </label>
                );
              })}
          </CardContent>
        </Card>
      ) : null}

      <div className="space-y-1.5">
        <p className="text-xs font-medium uppercase tracking-wide text-muted-foreground">
          Diff (current → proposed)
        </p>
        <DiffViewer lines={preview.diff} />
      </div>

      <Tabs defaultValue="proposed">
        <TabsList className="mb-2">
          <TabsTrigger value="current">Current</TabsTrigger>
          <TabsTrigger value="proposed">Proposed</TabsTrigger>
        </TabsList>
        <TabsContent value="current">
          <JsonPreview value={preview.currentJson} emptyLabel="File does not exist yet (a new file will be created)." />
        </TabsContent>
        <TabsContent value="proposed">
          <JsonPreview value={preview.proposedJson} emptyLabel="No proposal generated." />
        </TabsContent>
      </Tabs>
    </>
  );
}

function SyncReportDialog({ flow }: { flow: SyncFlow }) {
  const { report } = flow;
  if (!report) return null;
  return (
    <Dialog open={flow.reportOpen} onOpenChange={flow.setReportOpen}>
      <DialogContent
        title="Synchronization report"
        description={`${report.overall} · started ${formatDateTime(report.startedAt)} · finished ${relativeTime(report.finishedAt)}`}
        size="lg"
        footer={
          <Button variant="outline" onClick={() => flow.setReportOpen(false)}>
            Close
          </Button>
        }
      >
        <ul className="space-y-3">
          {report.results.map((item) => (
            <ResultRow key={item.harnessId} result={item} />
          ))}
        </ul>
      </DialogContent>
    </Dialog>
  );
}

function ResultRow({ result }: { result: HarnessSyncResult }) {
  const icon =
    result.status === "failed" ? (
      <XCircle className="h-4 w-4 text-red-500" aria-hidden />
    ) : result.status === "partial" ? (
      <AlertTriangle className="h-4 w-4 text-amber-500" aria-hidden />
    ) : result.status === "noChange" || result.status === "skipped" ? (
      <Minus className="h-4 w-4 text-muted-foreground" aria-hidden />
    ) : (
      <CheckCircle2 className="h-4 w-4 text-emerald-500" aria-hidden />
    );

  return (
    <li className="rounded-lg border border-border p-3">
      <div className="flex flex-wrap items-center gap-2">
        {icon}
        <span className="text-sm font-medium">{result.harnessName}</span>
        <Badge
          variant={result.status === "failed" ? "danger" : result.status === "partial" ? "warning" : "secondary"}
        >
          {result.status}
        </Badge>
        {result.verified ? <Badge variant="success">verified</Badge> : null}
      </div>
      <p className="mt-1 text-xs text-muted-foreground">{result.message}</p>
      {result.applied.length > 0 ? (
        <ul className="mt-2 space-y-1">
          {result.applied.map((change) => (
            <li key={`${change.name}-${change.action}-${change.message}`} className="flex items-center gap-2 text-xs">
              <Badge variant={ACTION_VARIANT[(change.action as DiffEntry["action"]) ?? "outline"] ?? "outline"}>
                {change.action}
              </Badge>
              <span className="font-mono">{change.name}</span>
              <span className="text-muted-foreground">{change.message}</span>
            </li>
          ))}
        </ul>
      ) : null}
      {result.backupPath ? (
        <p className="mt-2 break-all text-[11px] text-muted-foreground">Backup: {result.backupPath}</p>
      ) : null}
      {result.error ? (
        <div className="mt-2 rounded-md border border-red-500/40 bg-red-500/5 p-2">
          <p className="text-xs font-medium text-red-500">
            {result.error.code}: {result.error.message}
          </p>
          {result.error.remediation ? (
            <p className="mt-0.5 text-[11px] text-muted-foreground">{result.error.remediation}</p>
          ) : null}
        </div>
      ) : null}
    </li>
  );
}
