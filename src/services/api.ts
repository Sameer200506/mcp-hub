import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppSettings,
  BackupInfo,
  ConfigInspection,
  ConnectionTestResult,
  DashboardStats,
  DefinitionPreview,
  HarnessEntries,
  HarnessInfo,
  ImportResult,
  McpServer,
  McpServerInput,
  RegistryExportRequest,
  RegistryImportRequest,
  RestoreResult,
  SyncHistoryEntry,
  SyncReport,
  SyncRequest,
  PreviewReport,
  ValidationReport,
} from "@/types";

/**
 * Thin typed wrapper around the Rust command surface. The frontend never
 * touches the filesystem directly: every mutation is validated in Rust.
 */
export const api = {
  // ------------------------------------------------------------------ registry
  listMcpServers: () => invoke<McpServer[]>("list_mcp_servers"),
  getMcpServer: (id: string) => invoke<McpServer>("get_mcp_server", { id }),
  createMcpServer: (input: McpServerInput) => invoke<McpServer>("create_mcp_server", { input }),
  updateMcpServer: (id: string, input: McpServerInput) =>
    invoke<McpServer>("update_mcp_server", { id, input }),
  deleteMcpServer: (id: string) => invoke<void>("delete_mcp_server", { id }),
  duplicateMcpServer: (id: string, newName?: string) =>
    invoke<McpServer>("duplicate_mcp_server", { id, newName: newName ?? null }),
  setMcpEnabled: (id: string, enabled: boolean) =>
    invoke<McpServer>("set_mcp_enabled", { id, enabled }),
  validateMcpConfig: (input: McpServerInput) =>
    invoke<ValidationReport>("validate_mcp_config", { input }),
  previewDefinition: (input: McpServerInput, harnessId: string) =>
    invoke<DefinitionPreview>("preview_definition", { input, harnessId }),
  testMcpConnection: (input: McpServerInput) =>
    invoke<ConnectionTestResult>("test_mcp_connection", { input }),

  // ------------------------------------------------------------------ secrets
  listSecretKeys: (mcpId: string) => invoke<string[]>("list_secret_keys", { mcpId }),
  setSecret: (mcpId: string, key: string, value: string, slot: "env" | "headers") =>
    invoke<void>("set_secret", { mcpId, key, value, slot }),
  deleteSecret: (mcpId: string, key: string) => invoke<void>("delete_secret", { mcpId, key }),
  hasSecret: (mcpId: string, key: string) => invoke<boolean>("has_secret", { mcpId, key }),

  // ----------------------------------------------------------------- harnesses
  listHarnesses: () => invoke<HarnessInfo[]>("list_harnesses"),
  detectHarnesses: () => invoke<HarnessInfo[]>("detect_harnesses"),
  getHarnessConfig: (harnessId: string) =>
    invoke<ConfigInspection>("get_harness_config", { harnessId }),
  getHarnessConfigAtPath: (harnessId: string, path: string) =>
    invoke<ConfigInspection>("get_harness_config_at_path", { harnessId, path }),
  listHarnessEntries: (harnessId: string) =>
    invoke<HarnessEntries>("list_harness_entries", { harnessId }),
  openHarnessConfigDir: (harnessId: string) => invoke<string | null>("open_harness_config_dir", { harnessId }),

  // ---------------------------------------------------------------------- sync
  previewSync: (request: SyncRequest) => invoke<PreviewReport>("preview_sync", { request }),
  syncOne: (request: SyncRequest) => invoke<SyncReport>("sync_one", { request }),
  syncAll: (request: SyncRequest) => invoke<SyncReport>("sync_all", { request }),
  getSyncHistory: (limit = 100) => invoke<SyncHistoryEntry[]>("get_sync_history", { limit }),

  // ------------------------------------------------------------------ backups
  listBackups: (harnessId?: string) =>
    invoke<BackupInfo[]>("list_backups", { harnessId: harnessId ?? null }),
  restoreBackup: (backupId: number) => invoke<RestoreResult>("restore_backup", { backupId }),

  // ----------------------------------------------------------------- registry
  exportRegistry: (request: RegistryExportRequest) =>
    invoke<string>("export_registry", { request }),
  importRegistry: (request: RegistryImportRequest) =>
    invoke<ImportResult>("import_registry", { request }),

  // ----------------------------------------------------------------- settings
  getAppSettings: () => invoke<AppSettings>("get_app_settings"),
  updateAppSettings: (settings: AppSettings) =>
    invoke<AppSettings>("update_app_settings", { settings }),
  resetAppSettings: () => invoke<AppSettings>("reset_app_settings"),
  getDashboardStats: () => invoke<DashboardStats>("get_dashboard_stats"),
  getLogFilePath: () => invoke<string>("get_log_file_path"),
};

export function onHarnessConfigChanged(cb: (harnessId: string) => void): Promise<UnlistenFn> {
  return listen<{ harnessId: string }>("harness-config-changed", (event) =>
    cb(event.payload.harnessId),
  );
}

export function onSyncProgress(cb: (payload: { harnessId: string; message: string }) => void) {
  return listen<{ harnessId: string; message: string }>("sync-progress", (event) =>
    cb(event.payload),
  );
}
