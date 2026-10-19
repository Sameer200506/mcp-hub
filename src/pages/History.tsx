import { useCallback, useEffect, useState } from "react";
import { History as HistoryIcon, RotateCcw } from "lucide-react";
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
import { ConfirmDialog } from "@/components/ui/dialog";
import { useToast } from "@/components/ui/toast";
import { api } from "@/services/api";
import { describeError, type RegistryState } from "@/hooks/useRegistry";
import { formatBytes, formatDateTime, relativeTime } from "@/lib/utils";
import type { BackupInfo } from "@/types";

export function History({ registry }: { registry: RegistryState }) {
  const { toast } = useToast();
  const { history, harnesses } = registry;
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [pendingRestore, setPendingRestore] = useState<BackupInfo | null>(null);
  const [busy, setBusy] = useState(false);

  const harnessName = useCallback(
    (id: string) => harnesses.find((item) => item.id === id)?.name ?? id,
    [harnesses],
  );

  const loadBackups = useCallback(async () => {
    setLoading(true);
    try {
      setBackups(await api.listBackups());
    } catch (err) {
      toast({ title: "Could not load backups", description: describeError(err), variant: "error" });
    } finally {
      setLoading(false);
    }
  }, [toast]);

  useEffect(() => {
    void loadBackups();
  }, [loadBackups]);

  const restore = async () => {
    if (!pendingRestore) return;
    setBusy(true);
    try {
      const result = await api.restoreBackup(pendingRestore.id);
      toast({
        title: result.restored ? "Backup restored" : "Restore did not complete",
        description: result.message,
        variant: result.restored ? "success" : "error",
      });
      setPendingRestore(null);
      await registry.refresh();
      await loadBackups();
    } catch (err) {
      toast({ title: "Restore failed", description: describeError(err), variant: "error" });
    } finally {
      setBusy(false);
    }
  };

  const statusVariant = (status: string) =>
    status === "success" ? "success" : status === "failed" ? "danger" : "secondary";

  return (
    <div className="grid grid-cols-1 gap-4 xl:grid-cols-2">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <HistoryIcon className="h-4 w-4" />
            Synchronization history
          </CardTitle>
          <CardDescription>Every write, skip and failure is recorded with its own status.</CardDescription>
        </CardHeader>
        <CardContent>
          {history.length === 0 ? (
            <EmptyState title="No synchronization recorded yet" description="Run Sync All from the sidebar to get started." />
          ) : (
            <ul className="divide-y divide-border">
              {history.map((entry) => (
                <li key={entry.id} className="flex items-start justify-between gap-3 py-2">
                  <div className="min-w-0">
                    <p className="text-xs">
                      <span className="font-mono">{entry.mcpName ?? entry.action}</span>
                      <span className="text-muted-foreground"> → {harnessName(entry.harnessId)}</span>
                    </p>
                    <p className="truncate text-[11px] text-muted-foreground">{entry.message}</p>
                  </div>
                  <div className="shrink-0 text-right">
                    <Badge variant={statusVariant(entry.status)}>{entry.status}</Badge>
                    <p className="mt-1 text-[11px] text-muted-foreground" title={formatDateTime(entry.createdAt)}>
                      {relativeTime(entry.createdAt)}
                    </p>
                  </div>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <div className="flex items-start justify-between gap-2">
            <div>
              <CardTitle className="flex items-center gap-2">
                <RotateCcw className="h-4 w-4" />
                Backups
              </CardTitle>
              <CardDescription>
                Created before every write. Restoring keeps a safety copy of the current file.
              </CardDescription>
            </div>
            <Button variant="outline" size="sm" onClick={() => void loadBackups()} loading={loading}>
              Refresh
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          {loading && backups.length === 0 ? (
            <Spinner label="Loading backups…" />
          ) : backups.length === 0 ? (
            <EmptyState
              title="No backups yet"
              description="Backups appear here after the first configuration write."
            />
          ) : (
            <ul className="divide-y divide-border">
              {backups.map((backup) => (
                <li key={backup.id} className="flex items-start justify-between gap-3 py-2">
                  <div className="min-w-0">
                    <p className="text-xs">
                      <span className="font-medium">{harnessName(backup.harnessId)}</span>
                      <span className="text-muted-foreground"> · {backup.reason}</span>
                    </p>
                    <p className="break-all text-[11px] text-muted-foreground">{backup.backupPath}</p>
                    <p className="text-[11px] text-muted-foreground">
                      {formatBytes(backup.size)} · {formatDateTime(backup.createdAt)}
                      {backup.exists ? "" : " · file missing"}
                    </p>
                  </div>
                  <Button
                    variant="outline"
                    size="sm"
                    disabled={!backup.exists}
                    onClick={() => setPendingRestore(backup)}
                  >
                    Restore
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </CardContent>
      </Card>

      <ConfirmDialog
        open={pendingRestore !== null}
        onOpenChange={(open) => {
          if (!open) setPendingRestore(null);
        }}
        title="Restore this backup?"
        description={
          <span>
            The current file will be backed up again, then replaced with{" "}
            <span className="break-all font-mono text-xs">{pendingRestore?.backupPath}</span>.
          </span>
        }
        confirmLabel="Restore"
        busy={busy}
        onConfirm={() => void restore()}
      />
    </div>
  );
}
