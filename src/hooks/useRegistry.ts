import { useCallback, useEffect, useMemo, useState } from "react";
import { api, onHarnessConfigChanged } from "@/services/api";
import type { AppSettings, DashboardStats, HarnessInfo, McpServer, SyncHistoryEntry } from "@/types";

/**
 * Central data hook. Deliberately simple: refetch on demand, no global store.
 * All data originates from the Rust backend.
 */
export function useRegistry() {
  const [servers, setServers] = useState<McpServer[]>([]);
  const [harnesses, setHarnesses] = useState<HarnessInfo[]>([]);
  const [settings, setSettings] = useState<AppSettings | null>(null);
  const [stats, setStats] = useState<DashboardStats | null>(null);
  const [history, setHistory] = useState<SyncHistoryEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const [nextServers, nextHarnesses, nextSettings, nextStats, nextHistory] = await Promise.all([
        api.listMcpServers(),
        api.listHarnesses(),
        api.getAppSettings(),
        api.getDashboardStats(),
        api.getSyncHistory(50),
      ]);
      setServers(nextServers);
      setHarnesses(nextHarnesses);
      setSettings(nextSettings);
      setStats(nextStats);
      setHistory(nextHistory);
      setError(null);
    } catch (err) {
      setError(describeError(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void onHarnessConfigChanged(() => {
      void refresh();
    }).then((fn) => {
      unlisten = fn;
    });
    return () => unlisten?.();
  }, [refresh]);

  const detectedCount = useMemo(() => harnesses.filter((item) => item.detected).length, [harnesses]);

  return {
    servers,
    harnesses,
    settings,
    stats,
    history,
    loading,
    error,
    refresh,
    detectedCount,
    setError,
  };
}

export type RegistryState = ReturnType<typeof useRegistry>;

export function describeError(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object") {
    const maybe = err as { message?: string; code?: string; remediation?: string };
    if (maybe.message) {
      return maybe.remediation ? `${maybe.message} (${maybe.remediation})` : maybe.message;
    }
  }
  if (err instanceof Error) return err.message;
  return "Unexpected error";
}
