//! Typed data model exchanged with the React frontend.
//!
//! Field naming is camelCase on the wire to match the TypeScript mirror in
//! `src/types/index.ts`. Registry definitions never contain raw credential
//! bytes: secret values live in the OS credential store and are represented
//! here as `{ kind: "secret", value: "" }`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Sentinel written into previews/logs whenever a secret would have been shown.
pub const REDACTED: &str = "••••••••";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TransportType {
    Stdio,
    StreamableHttp,
    Sse,
}

impl TransportType {
    pub fn as_str(&self) -> &'static str {
        match self {
            TransportType::Stdio => "stdio",
            TransportType::StreamableHttp => "streamableHttp",
            TransportType::Sse => "sse",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VarKind {
    /// Plain value written verbatim into the harness configuration.
    Literal,
    /// `{env:NAME}` / `{file:path}` reference resolved by the harness at runtime.
    EnvRef,
    /// Value stored in the OS credential store, resolved only when writing.
    Secret,
}

impl Default for VarKind {
    fn default() -> Self {
        VarKind::Literal
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct VarValue {
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub kind: VarKind,
}

impl VarValue {
    pub fn literal(value: impl Into<String>) -> Self {
        Self { value: value.into(), kind: VarKind::Literal }
    }

    /// Whether the value should be masked in previews, exports and logs.
    pub fn looks_sensitive(&self) -> bool {
        self.kind == VarKind::Secret
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub transport: TransportType,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, VarValue>,
    #[serde(default)]
    pub headers: BTreeMap<String, VarValue>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub last_sync_at: Option<String>,
}

fn default_true() -> bool {
    true
}

impl McpServer {
    /// Stable identity used for duplicate detection: same endpoint or command.
    pub fn signature(&self) -> String {
        match self.transport {
            TransportType::Stdio => format!(
                "stdio|{}|{}",
                self.command.clone().unwrap_or_default(),
                self.args.join("\u{1f}")
            ),
            _ => format!("remote|{}", self.url.clone().unwrap_or_default()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerInput {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub transport: TransportType,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, VarValue>,
    #[serde(default)]
    pub headers: BTreeMap<String, VarValue>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub targets: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AdapterStatus {
    /// Schema verified against the vendor's documented configuration format.
    Verified,
    /// Location/schema not fully verified: the UI must confirm before writing.
    PendingVerification,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraEntryField {
    pub key: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessCapabilities {
    pub stdio: bool,
    pub streamable_http: bool,
    pub sse: bool,
    pub headers: bool,
    pub env: bool,
    pub cwd: bool,
    pub enabled_flag: bool,
    pub auto_approve: bool,
    #[serde(default)]
    pub extra_entry_fields: Vec<ExtraEntryField>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReloadRequirements {
    pub reload_required: bool,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigVariant {
    pub path: String,
    pub label: String,
    pub exists: bool,
    pub scope: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessInfo {
    pub id: String,
    pub name: String,
    pub detected: bool,
    pub detection_confidence: String,
    pub config_path: Option<String>,
    pub config_format: String,
    pub config_exists: bool,
    pub active_configs: Vec<ConfigVariant>,
    pub capabilities: HarnessCapabilities,
    pub reload: ReloadRequirements,
    pub adapter_status: AdapterStatus,
    pub notes: Vec<String>,
    pub last_sync_at: Option<String>,
    pub last_error: Option<String>,
    pub assigned_mcp_count: usize,
    pub externally_modified: bool,
    pub installed_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigInspection {
    pub harness_id: String,
    pub path: Option<String>,
    pub exists: bool,
    pub malformed: bool,
    pub format: String,
    pub raw: Option<String>,
    pub pretty: Option<String>,
    pub parse_error: Option<String>,
    pub mcp_entry_count: usize,
    pub sha256: Option<String>,
    pub size_bytes: u64,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessEntryInfo {
    pub name: String,
    pub managed: bool,
    pub summary: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessEntries {
    pub harness_id: String,
    pub path: Option<String>,
    pub malformed: bool,
    pub entries: Vec<HarnessEntryInfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffAction {
    Add,
    Update,
    Unchanged,
    Conflict,
    Skip,
    Remove,
}

impl DiffAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            DiffAction::Add => "add",
            DiffAction::Update => "update",
            DiffAction::Unchanged => "unchanged",
            DiffAction::Conflict => "conflict",
            DiffAction::Skip => "skip",
            DiffAction::Remove => "remove",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffEntry {
    pub name: String,
    pub action: DiffAction,
    pub details: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConflict {
    pub name: String,
    pub kind: String,
    pub message: String,
    pub existing: Option<Value>,
    pub incoming: Option<Value>,
    pub resolutions: Vec<String>,
    pub requires_removal_confirmation: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPreview {
    pub harness_id: String,
    pub harness_name: String,
    pub config_path: Option<String>,
    pub config_format: String,
    pub status: String,
    pub entries: Vec<DiffEntry>,
    pub conflicts: Vec<SyncConflict>,
    pub warnings: Vec<String>,
    pub current_json: Option<String>,
    pub proposed_json: Option<String>,
    pub diff: Vec<DiffLine>,
    pub malformed: bool,
    pub current_hash: Option<String>,
    pub read_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewReport {
    pub started_at: String,
    pub finished_at: String,
    pub overall: String,
    pub previews: Vec<SyncPreview>,
    pub resolutions_required: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppliedChange {
    pub name: String,
    pub action: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessSyncResult {
    pub harness_id: String,
    pub harness_name: String,
    pub status: String,
    pub message: String,
    pub applied: Vec<AppliedChange>,
    pub backup_path: Option<String>,
    pub verified: bool,
    pub error: Option<crate::errors::StructuredError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncReport {
    pub started_at: String,
    pub finished_at: String,
    pub overall: String,
    pub results: Vec<HarnessSyncResult>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolutionSelection {
    pub harness_id: String,
    pub name: String,
    pub action: String,
    #[serde(default)]
    pub new_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovalRequest {
    pub harness_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SyncRequest {
    #[serde(default)]
    pub harness_ids: Option<Vec<String>>,
    #[serde(default)]
    pub mcp_ids: Option<Vec<String>>,
    #[serde(default)]
    pub resolutions: Vec<ResolutionSelection>,
    #[serde(default)]
    pub removals: Vec<RemovalRequest>,
    #[serde(default)]
    pub confirm_removals: bool,
}

impl SyncRequest {
    pub fn resolution(&self, harness_id: &str, name: &str) -> Option<&ResolutionSelection> {
        self.resolutions
            .iter()
            .find(|item| item.harness_id == harness_id && item.name == name)
    }

    pub fn removal_approved(&self, harness_id: &str, name: &str) -> bool {
        self.confirm_removals
            && self
                .removals
                .iter()
                .any(|item| item.harness_id == harness_id && item.name == name)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationIssue {
    pub severity: String,
    pub field: String,
    pub message: String,
    pub remediation: Option<String>,
}

impl ValidationIssue {
    pub fn error(field: &str, message: impl Into<String>) -> Self {
        Self {
            severity: "error".into(),
            field: field.into(),
            message: message.into(),
            remediation: None,
        }
    }

    pub fn warning(field: &str, message: impl Into<String>) -> Self {
        Self {
            severity: "warning".into(),
            field: field.into(),
            message: message.into(),
            remediation: None,
        }
    }

    pub fn info(field: &str, message: impl Into<String>) -> Self {
        Self {
            severity: "info".into(),
            field: field.into(),
            message: message.into(),
            remediation: None,
        }
    }

    pub fn with_remediation(mut self, remediation: impl Into<String>) -> Self {
        self.remediation = Some(remediation.into());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessValidation {
    pub harness_id: String,
    pub harness_name: String,
    pub supported: bool,
    pub issues: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub valid: bool,
    pub issues: Vec<ValidationIssue>,
    pub per_harness: Vec<HarnessValidation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DefinitionPreview {
    pub harness_id: String,
    pub harness_name: String,
    pub entry_json: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTestResult {
    pub mode: String,
    pub target: String,
    pub success: bool,
    pub message: String,
    pub details: Vec<String>,
    pub duration_ms: u64,
    pub executed_process: bool,
    pub disclaimer: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupInfo {
    pub id: i64,
    pub harness_id: String,
    pub original_path: String,
    pub backup_path: String,
    pub sha256: String,
    pub size: u64,
    pub reason: String,
    pub created_at: String,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub restored: bool,
    pub backup_path: String,
    pub target_path: String,
    pub safety_backup_path: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncHistoryEntry {
    pub id: i64,
    pub harness_id: String,
    pub mcp_id: Option<String>,
    pub mcp_name: Option<String>,
    pub action: String,
    pub status: String,
    pub message: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentServer {
    pub id: String,
    pub name: String,
    pub updated_at: String,
    pub last_sync_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentError {
    pub harness_id: String,
    pub message: String,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardStats {
    pub total_servers: usize,
    pub enabled_servers: usize,
    pub disabled_servers: usize,
    pub detected_harnesses: usize,
    pub supported_harnesses: usize,
    pub successful_syncs: usize,
    pub failed_syncs: usize,
    pub pending_syncs: usize,
    pub servers_never_synced: usize,
    pub externally_modified_harnesses: Vec<String>,
    pub recent_servers: Vec<RecentServer>,
    pub recent_errors: Vec<RecentError>,
    pub last_sync_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HarnessState {
    pub harness_id: String,
    pub last_sync_at: Option<String>,
    pub last_error: Option<String>,
    pub last_known_hash: Option<String>,
    pub externally_modified: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportMapping {
    #[serde(default = "default_stdio_uses")]
    pub stdio_uses: String,
    #[serde(default = "default_stdio_value")]
    pub stdio_value: String,
    #[serde(default = "default_remote_uses")]
    pub remote_uses: String,
    #[serde(default = "default_streamable_http_value")]
    pub streamable_http_value: String,
    #[serde(default = "default_sse_value")]
    pub sse_value: String,
    #[serde(default = "default_url_field")]
    pub url_field: String,
    #[serde(default = "default_command_field")]
    pub command_field: String,
    #[serde(default = "default_args_field")]
    pub args_field: String,
    #[serde(default = "default_env_field")]
    pub env_field: String,
    #[serde(default = "default_cwd_field")]
    pub cwd_field: String,
    #[serde(default = "default_headers_field")]
    pub headers_field: String,
    #[serde(default = "default_disabled_field")]
    pub disabled_field: String,
}

/// The Claude/Cline-style JSON layout (`type: stdio | streamableHttp | sse`),
/// which is what the settings form pre-fills for a new profile.
///
/// This default is implemented by hand on purpose: the derived `Default` would
/// leave every field *name* empty, and an empty field name would make a
/// programmatically created profile write `"": "npx"` into the target file
/// instead of a usable entry. The serde defaults below mirror these values so
/// that a hand-written `{ "id": ..., "configPath": ... }` profile behaves like
/// one created through the UI.
impl Default for TransportMapping {
    fn default() -> Self {
        Self {
            stdio_uses: default_stdio_uses(),
            stdio_value: default_stdio_value(),
            remote_uses: default_remote_uses(),
            streamable_http_value: default_streamable_http_value(),
            sse_value: default_sse_value(),
            url_field: default_url_field(),
            command_field: default_command_field(),
            args_field: default_args_field(),
            env_field: default_env_field(),
            cwd_field: default_cwd_field(),
            headers_field: default_headers_field(),
            disabled_field: default_disabled_field(),
        }
    }
}

fn default_stdio_uses() -> String {
    "type".into()
}
fn default_stdio_value() -> String {
    "stdio".into()
}
fn default_remote_uses() -> String {
    "type".into()
}
fn default_streamable_http_value() -> String {
    "streamableHttp".into()
}
fn default_sse_value() -> String {
    "sse".into()
}
fn default_url_field() -> String {
    "url".into()
}
fn default_command_field() -> String {
    "command".into()
}
fn default_args_field() -> String {
    "args".into()
}
fn default_env_field() -> String {
    "env".into()
}
fn default_cwd_field() -> String {
    "cwd".into()
}
fn default_headers_field() -> String {
    "headers".into()
}
fn default_disabled_field() -> String {
    "disabled".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomField {
    pub key: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GenericProfile {
    pub id: String,
    pub label: String,
    pub config_path: String,
    #[serde(default = "default_mcp_path")]
    pub mcp_path: String,
    #[serde(default = "default_name_key")]
    pub server_name_key: String,
    #[serde(default)]
    pub transport_mapping: TransportMapping,
    #[serde(default)]
    pub custom_fields: Vec<CustomField>,
    #[serde(default)]
    pub verified: bool,
}

fn default_mcp_path() -> String {
    "mcpServers".into()
}
fn default_name_key() -> String {
    "name".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub data_dir: String,
    pub backup_dir: String,
    pub sync_mode: String,
    pub auto_sync_on_change: bool,
    pub confirm_before_write: bool,
    pub create_backups: bool,
    pub watch_config_files: bool,
    pub startup_view: String,
    pub logging_level: String,
    pub detect_project_configs: bool,
    pub prefer_env_refs: bool,
    pub theme: String,
    #[serde(default)]
    pub generic_profiles: Vec<GenericProfile>,
    #[serde(default)]
    pub config_overrides: BTreeMap<String, String>,
}

impl AppSettings {
    pub fn defaults() -> Self {
        let data_dir = crate::database::default_data_dir();
        let backup_dir = data_dir.join("backups");
        Self {
            data_dir: data_dir.to_string_lossy().to_string(),
            backup_dir: backup_dir.to_string_lossy().to_string(),
            sync_mode: "manual".into(),
            auto_sync_on_change: false,
            confirm_before_write: true,
            create_backups: true,
            watch_config_files: true,
            startup_view: "dashboard".into(),
            logging_level: "info".into(),
            detect_project_configs: false,
            prefer_env_refs: true,
            theme: "dark".into(),
            generic_profiles: Vec::new(),
            config_overrides: BTreeMap::new(),
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self::defaults()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryExportRequest {
    pub path: String,
    #[serde(default)]
    pub include_disabled: bool,
    #[serde(default)]
    pub include_literal_env_values: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryImportRequest {
    pub path: String,
    #[serde(default = "default_merge_mode")]
    pub mode: String,
}

fn default_merge_mode() -> String {
    "merge".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    pub imported: usize,
    pub updated: usize,
    pub skipped: usize,
    pub warnings: Vec<String>,
}

/// Portable registry document produced by `export_registry`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryFile {
    pub schema_version: u32,
    pub application: String,
    pub exported_at: String,
    pub includes_secrets: bool,
    pub includes_literal_env_values: bool,
    pub servers: Vec<McpServer>,
    #[serde(default)]
    pub warnings: Vec<String>,
}
