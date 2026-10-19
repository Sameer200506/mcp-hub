/**
 * Frontend type mirror of the Rust command payloads (src-tauri/src/models.rs).
 * Keep in sync with the backend; every field here is produced by the Rust side.
 */

export type TransportType = "stdio" | "streamableHttp" | "sse";

export type VarKind = "literal" | "envRef" | "secret";

export interface VarValue {
  /** Literal value, `{env:NAME}` reference or empty string for secrets. */
  value: string;
  kind: VarKind;
}

export interface McpServer {
  id: string;
  name: string;
  description: string;
  transport: TransportType;
  command: string | null;
  args: string[];
  env: Record<string, VarValue>;
  headers: Record<string, VarValue>;
  url: string | null;
  cwd: string | null;
  enabled: boolean;
  targets: string[];
  tags: string[];
  notes: string;
  createdAt: string;
  updatedAt: string;
  lastSyncAt: string | null;
}

export interface McpServerInput {
  id?: string;
  name: string;
  description: string;
  transport: TransportType;
  command: string | null;
  args: string[];
  env: Record<string, VarValue>;
  headers: Record<string, VarValue>;
  url: string | null;
  cwd: string | null;
  enabled: boolean;
  targets: string[];
  tags: string[];
  notes: string;
}

export type AdapterStatus = "verified" | "pendingVerification";

export interface HarnessCapabilities {
  stdio: boolean;
  streamableHttp: boolean;
  sse: boolean;
  headers: boolean;
  env: boolean;
  cwd: boolean;
  enabledFlag: boolean;
  autoApprove: boolean;
  extraEntryFields: { key: string; description: string }[];
  notes: string[];
}

export interface ReloadRequirements {
  reloadRequired: boolean;
  description: string;
}

export interface ConfigVariant {
  path: string;
  label: string;
  exists: boolean;
  scope: "global" | "project";
}

export interface HarnessInfo {
  id: string;
  name: string;
  detected: boolean;
  detectionConfidence: "verified" | "probable" | "notDetected" | "envOverride";
  configPath: string | null;
  configFormat: string;
  configExists: boolean;
  activeConfigs: ConfigVariant[];
  capabilities: HarnessCapabilities;
  reload: ReloadRequirements;
  adapterStatus: AdapterStatus;
  notes: string[];
  lastSyncAt: string | null;
  lastError: string | null;
  assignedMcpCount: number;
  externallyModified: boolean;
  installedVersion: string | null;
}

export interface ConfigInspection {
  harnessId: string;
  path: string | null;
  exists: boolean;
  malformed: boolean;
  format: string;
  raw: string | null;
  pretty: string | null;
  parseError: string | null;
  mcpEntryCount: number;
  sha256: string | null;
  sizeBytes: number;
  readOnly: boolean;
}

export interface HarnessEntryInfo {
  name: string;
  managed: boolean;
  summary: string;
  enabled: boolean;
}

export interface HarnessEntries {
  harnessId: string;
  path: string | null;
  malformed: boolean;
  entries: HarnessEntryInfo[];
}

export type DiffAction = "add" | "update" | "unchanged" | "conflict" | "skip" | "remove";

export interface DiffEntry {
  name: string;
  action: DiffAction;
  details: string;
  before: unknown | null;
  after: unknown | null;
}

export type ConflictKind =
  | "duplicateName"
  | "duplicateEndpoint"
  | "conflictingDefinition"
  | "duplicateRegistryKey"
  | "malformedExisting";

export interface SyncConflict {
  name: string;
  kind: ConflictKind;
  message: string;
  existing: unknown | null;
  incoming: unknown | null;
  resolutions: string[];
  requiresRemovalConfirmation: boolean;
}

export interface DiffLine {
  kind: "add" | "del" | "context";
  text: string;
}

export interface SyncPreview {
  harnessId: string;
  harnessName: string;
  configPath: string | null;
  configFormat: string;
  status: "ready" | "noChange" | "error" | "conflict" | "unsupported";
  entries: DiffEntry[];
  conflicts: SyncConflict[];
  warnings: string[];
  currentJson: string | null;
  proposedJson: string | null;
  diff: DiffLine[];
  malformed: boolean;
  currentHash: string | null;
  readOnly: boolean;
}

export interface PreviewReport {
  startedAt: string;
  finishedAt: string;
  overall: "ready" | "noChange" | "blocked";
  previews: SyncPreview[];
  resolutionsRequired: number;
}

export interface AppliedChange {
  name: string;
  action: string;
  message: string;
}

export interface StructuredError {
  code: string;
  message: string;
  remediation: string | null;
  details: unknown | null;
}

export interface HarnessSyncResult {
  harnessId: string;
  harnessName: string;
  status: "success" | "failed" | "skipped" | "noChange" | "partial";
  message: string;
  applied: AppliedChange[];
  backupPath: string | null;
  verified: boolean;
  error: StructuredError | null;
}

export interface SyncReport {
  startedAt: string;
  finishedAt: string;
  overall: "success" | "partial" | "failed" | "noChange";
  results: HarnessSyncResult[];
  dryRun: boolean;
}

export interface ResolutionSelection {
  harnessId: string;
  name: string;
  action: "update" | "rename" | "skip" | "remove" | "cancel";
  newName?: string | null;
}

export interface RemovalRequest {
  harnessId: string;
  name: string;
}

export interface SyncRequest {
  harnessIds: string[] | null;
  mcpIds: string[] | null;
  resolutions: ResolutionSelection[];
  removals: RemovalRequest[];
  confirmRemovals: boolean;
}

export interface ValidationIssue {
  severity: "error" | "warning" | "info";
  field: string;
  message: string;
  remediation: string | null;
}

export interface ValidationReport {
  valid: boolean;
  issues: ValidationIssue[];
  perHarness: { harnessId: string; harnessName: string; supported: boolean; issues: ValidationIssue[] }[];
}

export interface DefinitionPreview {
  harnessId: string;
  harnessName: string;
  entryJson: string;
  warnings: string[];
}

export interface ConnectionTestResult {
  mode: "jsonValidation" | "stdioLaunch" | "httpReachability";
  target: string;
  success: boolean;
  message: string;
  details: string[];
  durationMs: number;
  executedProcess: boolean;
  disclaimer: string;
}

export interface BackupInfo {
  id: number;
  harnessId: string;
  originalPath: string;
  backupPath: string;
  sha256: string;
  size: number;
  reason: string;
  createdAt: string;
  exists: boolean;
}

export interface RestoreResult {
  restored: boolean;
  backupPath: string;
  targetPath: string;
  safetyBackupPath: string | null;
  message: string;
}

export interface SyncHistoryEntry {
  id: number;
  harnessId: string;
  mcpId: string | null;
  mcpName: string | null;
  action: string;
  status: string;
  message: string;
  createdAt: string;
}

export type SyncMode = "manual" | "automatic";

export interface AppSettings {
  dataDir: string;
  backupDir: string;
  syncMode: SyncMode;
  autoSyncOnChange: boolean;
  confirmBeforeWrite: boolean;
  createBackups: boolean;
  watchConfigFiles: boolean;
  startupView: string;
  loggingLevel: "error" | "warn" | "info" | "debug" | "trace";
  detectProjectConfigs: boolean;
  preferEnvRefs: boolean;
  theme: "dark" | "light" | "system";
  genericProfiles: GenericProfile[];
  configOverrides: Record<string, string>;
}

export interface GenericProfile {
  id: string;
  label: string;
  configPath: string;
  mcpPath: string;
  serverNameKey: string;
  transportMapping: {
    stdioUses: "type" | "transport" | "none";
    stdioValue: string;
    remoteUses: "type" | "transport" | "none";
    streamableHttpValue: string;
    sseValue: string;
    urlField: string;
    commandField: string;
    argsField: string;
    envField: string;
    cwdField: string;
    headersField: string;
    disabledField: string;
  };
  customFields: { key: string; value: string }[];
  verified: boolean;
}

export interface RegistryExportRequest {
  path: string;
  includeDisabled: boolean;
  includeLiteralEnvValues: boolean;
}

export interface RegistryImportRequest {
  path: string;
  mode: "merge" | "replace";
}

export interface ImportResult {
  imported: number;
  updated: number;
  skipped: number;
  warnings: string[];
}

export interface DashboardStats {
  totalServers: number;
  enabledServers: number;
  disabledServers: number;
  detectedHarnesses: number;
  supportedHarnesses: number;
  successfulSyncs: number;
  failedSyncs: number;
  pendingSyncs: number;
  serversNeverSynced: number;
  externallyModifiedHarnesses: string[];
  recentServers: { id: string; name: string; updatedAt: string; lastSyncAt: string | null }[];
  recentErrors: { harnessId: string; message: string; at: string }[];
  lastSyncAt: string | null;
}

export interface SyncSettingsRequest {
  settings: AppSettings;
}
