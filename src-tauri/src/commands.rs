//! Tauri command surface.
//!
//! Every command is `async` and pushes its (potentially slow) work onto a
//! worker thread through [`blocking`], so the webview never stalls on file I/O,
//! SQLite access or a connection probe.
//!
//! Rules enforced here:
//! * the frontend never supplies a raw filesystem write target: writes are
//!   derived from detected adapter locations, explicit allow-lists or paths the
//!   user picked in the OS file dialog;
//! * secret *values* are never returned to the UI, never logged and never
//!   written to the SQLite database — previews mask them with `••••••••`;
//! * a connection probe only runs on explicit user action and never performs an
//!   MCP handshake.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use serde_json::{Map, Value};
use tauri::State;

use crate::adapters::generic_json::{canonical_json, describe_entry, ResolvedVars};
use crate::adapters::{allowlist_for, ConfigDocument, HarnessAdapter};
use crate::database::now_rfc3339;
use crate::errors::{AppError, AppResult};
use crate::models::{
    AdapterStatus, AppSettings, BackupInfo, ConfigInspection, ConnectionTestResult, DashboardStats,
    DefinitionPreview, HarnessEntries, HarnessEntryInfo, HarnessInfo, HarnessState, ImportResult,
    InstalledSkill, McpServer, McpServerInput, PreviewReport, RecentError, RecentServer,
    RegistryExportRequest, RegistryImportRequest, RestoreResult, SkillCatalogPage,
    SkillInstallRequest, SkillInstallResult, SkillRefreshResult, StoreCatalogPage, StoreRefreshResult,
    StoreSettings, SyncHistoryEntry, SyncReport, SyncRequest, TransportType, ValidationReport,
    VarKind, REDACTED,
};
use crate::services::config_sync::{self, SyncContext};
use crate::services::config_validation::validate_input;
use crate::services::harness_detection::{self, find_adapter, hints_for};
use crate::services::{mcp_registry, mcp_store, skill_store};
use crate::services::safe_io::read_snapshot;
use crate::state::{AppState, StateRefs};

/// Shown with every connection probe result.
const PROBE_DISCLAIMER: &str = "The probe only checks that the command starts or that the endpoint answers. It does not complete an MCP initialize handshake and cannot prove that the server works.";
const STDIO_PROBE_GRACE: Duration = Duration::from_millis(4_000);
const HTTP_PROBE_TIMEOUT: Duration = Duration::from_secs(8);

// ------------------------------------------------------------------ plumbing

type AdaptersGuard<'a> = std::sync::RwLockReadGuard<'a, Vec<Box<dyn HarnessAdapter>>>;

fn adapters_read(refs: &StateRefs) -> AppResult<AdaptersGuard<'_>> {
    refs.adapters
        .read()
        .map_err(|_| AppError::io("the adapter registry lock is poisoned"))
}

/// Run blocking work on a worker thread and surface both join errors and
/// domain errors in the same structured shape.
async fn blocking<T, F>(task: F) -> Result<T, AppError>
where
    T: Send + 'static,
    F: FnOnce() -> AppResult<T> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(task)
        .await
        .map_err(|error| AppError::io(format!("background task failed: {error}")))?
}

fn require_absolute(path: &str, what: &str) -> AppResult<PathBuf> {
    let candidate = PathBuf::from(path.trim());
    if !candidate.is_absolute() {
        return Err(AppError::validation(format!("{what} must be an absolute path.")));
    }
    Ok(candidate)
}

/// Compare paths the way the OS would (case-insensitive on Windows) without
/// requiring the file to exist.
fn normalize_for_compare(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = resolved.to_string_lossy().to_string();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

fn path_is_allowed(candidate: &Path, allowed: &[PathBuf]) -> bool {
    let normalized = normalize_for_compare(candidate);
    allowed
        .iter()
        .any(|path| normalize_for_compare(path) == normalized)
}

/// Replace known secret values in diagnostics so nothing sensitive is shown.
fn scrub(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_string();
    for secret in secrets {
        if secret.len() >= 4 {
            out = out.replace(secret.as_str(), REDACTED);
        }
    }
    out
}

// ------------------------------------------------------------------- registry

#[tauri::command]
pub async fn list_mcp_servers(state: State<'_, AppState>) -> Result<Vec<McpServer>, AppError> {
    let refs = state.refs.clone();
    blocking(move || refs.db.list_servers()).await
}

#[tauri::command]
pub async fn get_mcp_server(state: State<'_, AppState>, id: String) -> Result<McpServer, AppError> {
    let refs = state.refs.clone();
    blocking(move || refs.db.get_server(&id)).await
}

#[tauri::command]
pub async fn create_mcp_server(
    state: State<'_, AppState>,
    input: McpServerInput,
) -> Result<McpServer, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        let adapters = adapters_read(&refs)?;
        mcp_registry::create_server(&refs.db, &refs.secrets, &adapters, &input)
    })
    .await
}

#[tauri::command]
pub async fn update_mcp_server(
    state: State<'_, AppState>,
    id: String,
    input: McpServerInput,
) -> Result<McpServer, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        let adapters = adapters_read(&refs)?;
        mcp_registry::update_server(&refs.db, &refs.secrets, &adapters, &id, &input)
    })
    .await
}

#[tauri::command]
pub async fn delete_mcp_server(state: State<'_, AppState>, id: String) -> Result<(), AppError> {
    let refs = state.refs.clone();
    blocking(move || mcp_registry::delete_server(&refs.db, &refs.secrets, &id)).await
}

#[tauri::command]
pub async fn duplicate_mcp_server(
    state: State<'_, AppState>,
    id: String,
    new_name: Option<String>,
) -> Result<McpServer, AppError> {
    let refs = state.refs.clone();
    blocking(move || mcp_registry::duplicate_server(&refs.db, &refs.secrets, &id, new_name)).await
}

#[tauri::command]
pub async fn set_mcp_enabled(
    state: State<'_, AppState>,
    id: String,
    enabled: bool,
) -> Result<McpServer, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        refs.db.set_server_enabled(&id, enabled, &now_rfc3339())?;
        refs.db.get_server(&id)
    })
    .await
}

#[tauri::command]
pub async fn validate_mcp_config(
    state: State<'_, AppState>,
    input: McpServerInput,
) -> Result<ValidationReport, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        let adapters = adapters_read(&refs)?;
        Ok(validate_input(&input, &adapters))
    })
    .await
}

#[tauri::command]
pub async fn preview_definition(
    state: State<'_, AppState>,
    input: McpServerInput,
    harness_id: String,
) -> Result<DefinitionPreview, AppError> {
    let refs = state.refs.clone();
    blocking(move || preview_definition_inner(&refs, &input, &harness_id)).await
}

#[tauri::command]
pub async fn test_mcp_connection(
    state: State<'_, AppState>,
    input: McpServerInput,
) -> Result<ConnectionTestResult, AppError> {
    let refs = state.refs.clone();
    blocking(move || probe_connection(&refs, &input)).await
}

// -------------------------------------------------------------------- secrets

#[tauri::command]
pub async fn list_secret_keys(
    state: State<'_, AppState>,
    mcp_id: String,
) -> Result<Vec<String>, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        Ok(refs
            .db
            .list_secret_refs(&mcp_id)?
            .into_iter()
            .map(|(slot, key)| format!("{slot}:{key}"))
            .collect())
    })
    .await
}

#[tauri::command]
pub async fn set_secret(
    state: State<'_, AppState>,
    mcp_id: String,
    key: String,
    value: String,
    slot: String,
) -> Result<(), AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        if slot != "env" && slot != "headers" {
            return Err(AppError::validation("The secret slot must be 'env' or 'headers'."));
        }
        if key.trim().is_empty() {
            return Err(AppError::validation("A secret name is required."));
        }
        if value.is_empty() {
            return Err(AppError::validation("The secret value must not be empty."));
        }
        refs.db.get_server(&mcp_id)?;
        if !refs.secrets.is_available() {
            return Err(AppError::secret_store(
                "The OS credential store is unavailable; store a literal value instead.",
            ));
        }
        refs.secrets.set(&mcp_id, &slot, &key, &value)?;
        refs.db.register_secret_ref(&mcp_id, &slot, &key)?;
        // Never log the value.
        tracing::info!(mcp_id = %mcp_id, slot = %slot, key = %key, "secret stored in the OS credential store");
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn delete_secret(
    state: State<'_, AppState>,
    mcp_id: String,
    key: String,
) -> Result<(), AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        for slot in ["env", "headers"] {
            refs.secrets.delete(&mcp_id, slot, &key)?;
            refs.db.remove_secret_ref(&mcp_id, slot, &key)?;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn has_secret(
    state: State<'_, AppState>,
    mcp_id: String,
    key: String,
) -> Result<bool, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        for slot in ["env", "headers"] {
            if refs.secrets.get(&mcp_id, slot, &key)?.is_some() {
                return Ok(true);
            }
        }
        Ok(false)
    })
    .await
}

// ------------------------------------------------------------------- harnesses

#[tauri::command]
pub async fn list_harnesses(state: State<'_, AppState>) -> Result<Vec<HarnessInfo>, AppError> {
    let refs = state.refs.clone();
    blocking(move || harness_infos(&refs)).await
}

#[tauri::command]
pub async fn detect_harnesses(state: State<'_, AppState>) -> Result<Vec<HarnessInfo>, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        let infos = harness_infos(&refs)?;
        if let Err(error) = refs.restart_watcher() {
            tracing::warn!(message = %error.message, "watch could not be restarted after detection");
        }
        Ok(infos)
    })
    .await
}

#[tauri::command]
pub async fn get_harness_config(
    state: State<'_, AppState>,
    harness_id: String,
) -> Result<ConfigInspection, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        let settings = refs.settings_snapshot();
        let adapters = adapters_read(&refs)?;
        let adapter = find_adapter(&adapters, &harness_id)?;
        let hints = hints_for(&settings);
        let path = adapter.locate_config(&hints);
        inspect_config(adapter, path)
    })
    .await
}

#[tauri::command]
pub async fn get_harness_config_at_path(
    state: State<'_, AppState>,
    harness_id: String,
    path: String,
) -> Result<ConfigInspection, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        let settings = refs.settings_snapshot();
        let adapters = adapters_read(&refs)?;
        let adapter = find_adapter(&adapters, &harness_id)?;
        let hints = hints_for(&settings);
        let candidate = require_absolute(&path, "The configuration path")?;
        let allowed = allowlist_for(adapter, &hints);
        if !path_is_allowed(&candidate, &allowed) {
            return Err(AppError::permission_denied(format!(
                "{} is not one of the configuration locations known for {}.",
                candidate.display(),
                adapter.display_name()
            ))
            .with_remediation(
                "Pick one of the listed configuration variants, or add a path override in Settings.",
            ));
        }
        inspect_config(adapter, Some(candidate))
    })
    .await
}

#[tauri::command]
pub async fn list_harness_entries(
    state: State<'_, AppState>,
    harness_id: String,
) -> Result<HarnessEntries, AppError> {
    let refs = state.refs.clone();
    blocking(move || list_entries(&refs, &harness_id)).await
}

#[tauri::command]
pub async fn open_harness_config_dir(
    state: State<'_, AppState>,
    harness_id: String,
) -> Result<Option<String>, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        let settings = refs.settings_snapshot();
        let adapters = adapters_read(&refs)?;
        let adapter = find_adapter(&adapters, &harness_id)?;
        let hints = hints_for(&settings);
        // The UI opens the returned directory with the OS shell; this command
        // never launches a process by itself.
        Ok(adapter
            .locate_config(&hints)
            .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
            .map(|parent| parent.to_string_lossy().to_string()))
    })
    .await
}

// ------------------------------------------------------------------------ sync

#[tauri::command]
pub async fn preview_sync(
    state: State<'_, AppState>,
    request: SyncRequest,
) -> Result<PreviewReport, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        let settings = refs.settings_snapshot();
        let backups = refs.backup_manager();
        let adapters = adapters_read(&refs)?;
        let ctx = SyncContext {
            db: &refs.db,
            secrets: &refs.secrets,
            backups: &backups,
            adapters: &adapters,
            settings: &settings,
        };
        config_sync::build_preview(&ctx, &request)
    })
    .await
}

#[tauri::command]
pub async fn sync_all(
    state: State<'_, AppState>,
    request: SyncRequest,
) -> Result<SyncReport, AppError> {
    let refs = state.refs.clone();
    blocking(move || run_sync(&refs, &request, "sync_all")).await
}

#[tauri::command]
pub async fn sync_one(
    state: State<'_, AppState>,
    request: SyncRequest,
) -> Result<SyncReport, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        let harnesses = request.harness_ids.clone().unwrap_or_default();
        if harnesses.len() != 1 {
            return Err(AppError::validation(
                "A single-harness synchronization requires exactly one harness id.",
            ));
        }
        let adapters = adapters_read(&refs)?;
        find_adapter(&adapters, &harnesses[0])?;
        drop(adapters);
        run_sync(&refs, &request, "sync_one")
    })
    .await
}

#[tauri::command]
pub async fn get_sync_history(
    state: State<'_, AppState>,
    limit: u32,
) -> Result<Vec<SyncHistoryEntry>, AppError> {
    let refs = state.refs.clone();
    blocking(move || refs.db.sync_history(limit.clamp(1, 1_000))).await
}

// --------------------------------------------------------------------- backups

#[tauri::command]
pub async fn list_backups(
    state: State<'_, AppState>,
    harness_id: Option<String>,
) -> Result<Vec<BackupInfo>, AppError> {
    let refs = state.refs.clone();
    blocking(move || refs.db.list_backups(harness_id.as_deref())).await
}

#[tauri::command]
pub async fn restore_backup(
    state: State<'_, AppState>,
    backup_id: i64,
) -> Result<RestoreResult, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        let backup = refs.db.get_backup(backup_id)?;
        let manager = refs.backup_manager();
        let result = manager.restore(&refs.db, &backup)?;

        let target = PathBuf::from(&backup.original_path);
        let new_hash = read_snapshot(&target).ok().flatten().map(|snapshot| snapshot.sha256);
        let previous = refs.db.get_harness_state(&backup.harness_id).ok().flatten();
        let state = HarnessState {
            harness_id: backup.harness_id.clone(),
            last_sync_at: previous.as_ref().and_then(|item| item.last_sync_at.clone()),
            last_error: None,
            last_known_hash: new_hash,
            externally_modified: false,
        };
        let _ = refs.db.upsert_harness_state(&state);
        let _ = refs
            .db
            .record_sync(&backup.harness_id, None, None, "restore", "success", &result.message);
        refs.emit_config_changed(&backup.harness_id, Some(&backup.original_path));
        Ok(result)
    })
    .await
}

// -------------------------------------------------------------------- registry

#[tauri::command]
pub async fn export_registry(
    state: State<'_, AppState>,
    request: RegistryExportRequest,
) -> Result<String, AppError> {
    let refs = state.refs.clone();
    blocking(move || mcp_registry::export_registry(&refs.db, &request)).await
}

#[tauri::command]
pub async fn import_registry(
    state: State<'_, AppState>,
    request: RegistryImportRequest,
) -> Result<ImportResult, AppError> {
    let refs = state.refs.clone();
    blocking(move || mcp_registry::import_registry(&refs.db, &refs.secrets, &request)).await
}

// ------------------------------------------------------------------ MCP store

#[tauri::command]
pub async fn refresh_store_catalog(
    state: State<'_, AppState>,
    pages: Option<u32>,
    restart: Option<bool>,
) -> Result<StoreRefreshResult, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        mcp_store::refresh_catalog(&refs.db, pages.unwrap_or(1), restart.unwrap_or(false))
    })
    .await
}

#[tauri::command]
pub async fn list_store_catalog(
    state: State<'_, AppState>,
    query: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<StoreCatalogPage, AppError> {
    let refs = state.refs.clone();
    blocking(move || mcp_store::catalog_page(&refs.db, query, limit, offset)).await
}

#[tauri::command]
pub async fn store_prefill_server(
    state: State<'_, AppState>,
    entry_id: String,
) -> Result<McpServerInput, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        let entry = refs.db.get_store_entry(&entry_id)?;
        Ok(mcp_store::entry_prefill(&entry))
    })
    .await
}

// --------------------------------------------------------------- agent skills

#[tauri::command]
pub async fn get_store_settings(state: State<'_, AppState>) -> Result<StoreSettings, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        let settings = skill_store::load_settings(&refs.db)?;
        // Persist the defaults once so the settings dialog edits a stored value.
        if refs
            .db
            .get_setting_json::<StoreSettings>(skill_store::STORE_SETTINGS_KEY)?
            .is_none()
        {
            refs.db
                .set_setting_json(skill_store::STORE_SETTINGS_KEY, &settings)?;
        }
        Ok(settings)
    })
    .await
}

#[tauri::command]
pub async fn update_store_settings(
    state: State<'_, AppState>,
    settings: StoreSettings,
) -> Result<StoreSettings, AppError> {
    let refs = state.refs.clone();
    blocking(move || skill_store::save_settings(&refs.db, &settings)).await
}

#[tauri::command]
pub async fn refresh_skills(
    state: State<'_, AppState>,
    source_id: Option<String>,
) -> Result<SkillRefreshResult, AppError> {
    let refs = state.refs.clone();
    blocking(move || skill_store::refresh_skills(&refs.db, source_id)).await
}

#[tauri::command]
pub async fn list_skill_catalog(
    state: State<'_, AppState>,
    source_id: Option<String>,
    query: Option<String>,
) -> Result<SkillCatalogPage, AppError> {
    let refs = state.refs.clone();
    blocking(move || skill_store::skill_catalog(&refs.db, source_id, query)).await
}

#[tauri::command]
pub async fn install_skill(
    state: State<'_, AppState>,
    request: SkillInstallRequest,
) -> Result<SkillInstallResult, AppError> {
    let refs = state.refs.clone();
    let data_dir = PathBuf::from(refs.settings_snapshot().data_dir);
    blocking(move || skill_store::install_skill(&refs.db, &data_dir, &request)).await
}

#[tauri::command]
pub async fn list_installed_skills(
    state: State<'_, AppState>,
) -> Result<Vec<InstalledSkill>, AppError> {
    let refs = state.refs.clone();
    blocking(move || refs.db.list_installed_skills()).await
}

#[tauri::command]
pub async fn uninstall_skill(
    state: State<'_, AppState>,
    skill_id: String,
    remove_deployed: Option<bool>,
) -> Result<(), AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        skill_store::uninstall_skill(&refs.db, &skill_id, remove_deployed.unwrap_or(true))
    })
    .await
}

// --------------------------------------------------------------------- settings

#[tauri::command]
pub async fn get_app_settings(state: State<'_, AppState>) -> Result<AppSettings, AppError> {
    let refs = state.refs.clone();
    blocking(move || Ok(refs.settings_snapshot())).await
}

#[tauri::command]
pub async fn update_app_settings(
    state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<AppSettings, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        validate_settings(&settings)?;
        persist_settings(&refs, &settings)?;
        Ok(settings)
    })
    .await
}

#[tauri::command]
pub async fn reset_app_settings(state: State<'_, AppState>) -> Result<AppSettings, AppError> {
    let refs = state.refs.clone();
    blocking(move || {
        let defaults = AppSettings::defaults();
        persist_settings(&refs, &defaults)?;
        Ok(defaults)
    })
    .await
}

#[tauri::command]
pub async fn get_dashboard_stats(state: State<'_, AppState>) -> Result<DashboardStats, AppError> {
    let refs = state.refs.clone();
    blocking(move || dashboard_stats(&refs)).await
}

#[tauri::command]
pub async fn get_log_file_path(state: State<'_, AppState>) -> Result<String, AppError> {
    Ok(state.log_file.to_string_lossy().to_string())
}

// --------------------------------------------------------------- implementations

fn persist_settings(refs: &StateRefs, settings: &AppSettings) -> AppResult<()> {
    refs.db.set_setting_json("settings", settings)?;
    {
        let mut guard = refs
            .settings
            .write()
            .map_err(|_| AppError::io("the settings lock is poisoned"))?;
        *guard = settings.clone();
    }
    refs.replace_adapters(settings);
    {
        let mut dir = refs
            .backup_dir
            .write()
            .map_err(|_| AppError::io("the backup directory lock is poisoned"))?;
        *dir = PathBuf::from(settings.backup_dir.trim());
    }
    std::fs::create_dir_all(Path::new(&settings.backup_dir))?;
    if let Err(error) = std::fs::create_dir_all(Path::new(&settings.data_dir)) {
        tracing::warn!(
            path = %settings.data_dir,
            message = %error,
            "the configured data directory could not be created"
        );
    }
    refs.restart_watcher()?;
    tracing::info!("application settings updated");
    Ok(())
}

fn validate_settings(settings: &AppSettings) -> AppResult<()> {
    if !matches!(settings.sync_mode.as_str(), "manual" | "automatic") {
        return Err(AppError::validation("Unknown synchronization mode."));
    }
    if !matches!(settings.theme.as_str(), "dark" | "light" | "system") {
        return Err(AppError::validation("Unknown theme."));
    }
    if !matches!(
        settings.logging_level.as_str(),
        "error" | "warn" | "info" | "debug" | "trace"
    ) {
        return Err(AppError::validation("Unknown logging level."));
    }
    if settings.backup_dir.trim().is_empty() {
        return Err(AppError::validation("The backup directory must not be empty."));
    }
    if !Path::new(settings.backup_dir.trim()).is_absolute() {
        return Err(AppError::validation("The backup directory must be an absolute path."));
    }

    let mut seen = std::collections::HashSet::new();
    for profile in &settings.generic_profiles {
        let id = profile.id.trim();
        if id.is_empty() {
            return Err(AppError::validation("Every generic profile needs an id."));
        }
        if !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
        {
            return Err(AppError::validation(format!(
                "The generic profile id '{id}' may only contain letters, digits, dot, dash and underscore."
            )));
        }
        if matches!(id, "opencode" | "cline" | "antigravity") {
            return Err(AppError::validation(format!(
                "The generic profile id '{id}' is reserved for a built-in adapter."
            )));
        }
        if !seen.insert(id.to_string()) {
            return Err(AppError::validation(format!(
                "Two generic profiles share the id '{id}'."
            )));
        }
        if profile.config_path.trim().is_empty() {
            return Err(AppError::validation(format!(
                "The generic profile '{id}' needs a configuration file path."
            )));
        }
        if !Path::new(profile.config_path.trim()).is_absolute() {
            return Err(AppError::validation(format!(
                "The generic profile '{id}' needs an absolute configuration file path."
            )));
        }
        if profile.mcp_path.trim().is_empty() {
            return Err(AppError::validation(format!(
                "The generic profile '{id}' needs a JSON path to the server entries (e.g. `mcpServers`)."
            )));
        }
    }

    for (harness_id, override_path) in &settings.config_overrides {
        if override_path.trim().is_empty() {
            continue;
        }
        if !Path::new(override_path.trim()).is_absolute() {
            return Err(AppError::validation(format!(
                "The path override for '{harness_id}' must be an absolute path."
            )));
        }
    }
    Ok(())
}

fn harness_infos(refs: &StateRefs) -> AppResult<Vec<HarnessInfo>> {
    let settings = refs.settings_snapshot();
    let adapters = adapters_read(refs)?;
    harness_detection::build_harness_infos(&refs.db, &adapters, &settings)
}

fn inspect_config(
    adapter: &dyn HarnessAdapter,
    path: Option<PathBuf>,
) -> AppResult<ConfigInspection> {
    let Some(path) = path else {
        return Ok(ConfigInspection {
            harness_id: adapter.id(),
            path: None,
            exists: false,
            malformed: false,
            format: adapter.config_format_label(),
            raw: None,
            pretty: None,
            parse_error: None,
            mcp_entry_count: 0,
            sha256: None,
            size_bytes: 0,
            read_only: false,
        });
    };
    let document = adapter.read_config(&path)?;
    let pretty = if document.malformed.is_none() {
        Some(canonical_json(&document.value))
    } else {
        None
    };
    Ok(ConfigInspection {
        harness_id: adapter.id(),
        path: Some(path.to_string_lossy().to_string()),
        exists: document.existed,
        malformed: document.malformed.is_some(),
        format: document.format.clone(),
        raw: document.raw.clone(),
        pretty,
        parse_error: document.malformed.clone(),
        mcp_entry_count: adapter.entry_count(&document),
        sha256: document.hash.clone(),
        size_bytes: document.size,
        read_only: document.read_only,
    })
}

fn list_entries(refs: &StateRefs, harness_id: &str) -> AppResult<HarnessEntries> {
    let settings = refs.settings_snapshot();
    let adapters = adapters_read(refs)?;
    let adapter = find_adapter(&adapters, harness_id)?;
    let hints = hints_for(&settings);
    let Some(path) = adapter.locate_config(&hints) else {
        return Ok(HarnessEntries {
            harness_id: adapter.id(),
            path: None,
            malformed: false,
            entries: Vec::new(),
        });
    };
    let document = adapter.read_config(&path)?;
    let servers = refs.db.list_servers()?;
    let entries = adapter
        .existing_entries(&document)
        .into_iter()
        .map(|entry| {
            let (summary, enabled) = describe_entry(&entry.value);
            let managed = servers.iter().any(|server| {
                server.name.eq_ignore_ascii_case(&entry.name)
                    && server.targets.iter().any(|target| target == harness_id)
            });
            HarnessEntryInfo {
                name: entry.name,
                managed,
                summary,
                enabled,
            }
        })
        .collect();
    Ok(HarnessEntries {
        harness_id: adapter.id(),
        path: Some(path.to_string_lossy().to_string()),
        malformed: document.malformed.is_some(),
        entries,
    })
}

fn run_sync(refs: &StateRefs, request: &SyncRequest, initiator: &str) -> AppResult<SyncReport> {
    let settings = refs.settings_snapshot();
    let backups = refs.backup_manager();
    let adapters = adapters_read(refs)?;
    let ctx = SyncContext {
        db: &refs.db,
        secrets: &refs.secrets,
        backups: &backups,
        adapters: &adapters,
        settings: &settings,
    };
    let report = config_sync::apply(&ctx, request, &|harness_id, message| {
        refs.emit_progress(harness_id, message);
    })?;

    for result in &report.results {
        if matches!(
            result.status.as_str(),
            "success" | "partial" | "noChange"
        ) {
            refs.emit_config_changed(&result.harness_id, None);
        }
    }
    tracing::info!(
        initiator = %initiator,
        overall = %report.overall,
        harnesses = report.results.len(),
        "synchronization finished"
    );
    Ok(report)
}

/// Build a preview of how one definition would be encoded for a harness.
///
/// Credential values are never read for this: secret slots render as the
/// `••••••••` mask so the preview can be shown and logged safely.
fn preview_definition_inner(
    refs: &StateRefs,
    input: &McpServerInput,
    harness_id: &str,
) -> AppResult<DefinitionPreview> {
    let adapters = adapters_read(refs)?;
    let adapter = find_adapter(&adapters, harness_id)?;

    let report = validate_input(input, &adapters);
    if !report.valid {
        let details = serde_json::to_value(&report).unwrap_or(Value::Null);
        return Err(AppError::validation("The MCP definition failed validation.")
            .with_details(details)
            .with_remediation("Fix the reported errors and try again."));
    }

    let settings = refs.settings_snapshot();
    let hints = hints_for(&settings);
    let path = adapter.locate_config(&hints);
    let document = match path.as_deref() {
        Some(path) if path.exists() => adapter.read_config(path)?,
        Some(path) => ConfigDocument::empty_for(&adapter.id(), path),
        None => ConfigDocument::empty_for(&adapter.id(), Path::new("<new-file>")),
    };

    let server = input_to_server(input);
    let resolved = masked_resolved(&server);
    let mut warnings = resolved.warnings.clone();

    if !adapter.supports_transport(server.transport) {
        warnings.push(format!(
            "{} does not support the {} transport; a synchronization would skip this server.",
            adapter.display_name(),
            server.transport.as_str()
        ));
    }
    if server.cwd.is_some() && !adapter.capabilities().cwd {
        warnings.push("The working directory will be dropped by this adapter.".into());
    }
    if adapter.adapter_status() == AdapterStatus::PendingVerification {
        warnings.push(
            "This adapter is not verified: confirm the schema in the preview before writing.".into(),
        );
    }
    if document.malformed.is_some() {
        warnings.push(
            "The existing configuration file is not valid JSON; a synchronization would refuse to write until it is fixed."
                .into(),
        );
    }

    let entry = adapter.merge_mcp_definition(&document, &server, &resolved)?;
    Ok(DefinitionPreview {
        harness_id: adapter.id(),
        harness_name: adapter.display_name(),
        entry_json: canonical_json(&entry),
        warnings,
    })
}

fn input_to_server(input: &McpServerInput) -> McpServer {
    let clean = |value: &Option<String>| {
        value
            .as_ref()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
    };
    McpServer {
        id: input.id.clone().unwrap_or_default(),
        name: input.name.trim().to_string(),
        description: input.description.trim().to_string(),
        transport: input.transport,
        command: clean(&input.command),
        args: input
            .args
            .iter()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .collect(),
        env: input.env.clone(),
        headers: input.headers.clone(),
        url: clean(&input.url),
        cwd: clean(&input.cwd),
        enabled: input.enabled,
        targets: input.targets.clone(),
        tags: input.tags.clone(),
        notes: input.notes.clone(),
        created_at: String::new(),
        updated_at: String::new(),
        last_sync_at: None,
    }
}

fn masked_resolved(server: &McpServer) -> ResolvedVars {
    let mut resolved = ResolvedVars {
        env: Map::new(),
        headers: Map::new(),
        warnings: Vec::new(),
        sensitive: Vec::new(),
    };
    for (slot, vars) in [("env", &server.env), ("headers", &server.headers)] {
        for (key, value) in vars {
            let rendered = match value.kind {
                VarKind::Secret => {
                    resolved.warnings.push(format!(
                        "Credential '{key}' is masked in this preview; its stored value is written to the harness file on synchronize."
                    ));
                    REDACTED.to_string()
                }
                _ => value.value.clone(),
            };
            if slot == "env" {
                resolved.env.insert(key.clone(), Value::String(rendered));
            } else {
                resolved.headers.insert(key.clone(), Value::String(rendered));
            }
        }
    }
    resolved
}

fn dashboard_stats(refs: &StateRefs) -> AppResult<DashboardStats> {
    let servers = refs.db.list_servers()?;
    let infos = harness_infos(refs)?;
    let counts = refs.db.sync_counts()?;
    let history = refs.db.sync_history(200)?;

    let mut recent_servers: Vec<RecentServer> = servers
        .iter()
        .map(|server| RecentServer {
            id: server.id.clone(),
            name: server.name.clone(),
            updated_at: server.updated_at.clone(),
            last_sync_at: server.last_sync_at.clone(),
        })
        .collect();
    recent_servers.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    recent_servers.truncate(6);

    let recent_errors: Vec<RecentError> = history
        .iter()
        .filter(|entry| entry.status == "failed")
        .take(5)
        .map(|entry| RecentError {
            harness_id: entry.harness_id.clone(),
            message: entry.message.clone(),
            at: entry.created_at.clone(),
        })
        .collect();

    let enabled = servers.iter().filter(|server| server.enabled).count();
    let never_synced = servers.iter().filter(|server| server.last_sync_at.is_none()).count();

    Ok(DashboardStats {
        total_servers: servers.len(),
        enabled_servers: enabled,
        disabled_servers: servers.len() - enabled,
        detected_harnesses: infos.iter().filter(|info| info.detected).count(),
        supported_harnesses: infos.len(),
        successful_syncs: counts.successful,
        failed_syncs: counts.failed,
        pending_syncs: never_synced,
        servers_never_synced: never_synced,
        externally_modified_harnesses: infos
            .iter()
            .filter(|info| info.externally_modified)
            .map(|info| info.id.clone())
            .collect(),
        recent_servers,
        recent_errors,
        last_sync_at: counts.last_sync_at,
    })
}

// ------------------------------------------------------------- connection probe

fn probe_connection(refs: &StateRefs, input: &McpServerInput) -> AppResult<ConnectionTestResult> {
    let started = Instant::now();

    let report = {
        let adapters = adapters_read(refs)?;
        validate_input(input, &adapters)
    };
    if !report.valid {
        let details: Vec<String> = report
            .issues
            .iter()
            .chain(report.per_harness.iter().flat_map(|item| item.issues.iter()))
            .filter(|issue| issue.severity == "error")
            .map(|issue| format!("{}: {}", issue.field, issue.message))
            .collect();
        return Ok(ConnectionTestResult {
            mode: "jsonValidation".into(),
            target: input.name.clone(),
            success: false,
            message: "The definition is invalid, so no connection probe was attempted.".into(),
            details,
            duration_ms: elapsed_ms(started),
            executed_process: false,
            disclaimer: PROBE_DISCLAIMER.into(),
        });
    }

    match input.transport {
        TransportType::Stdio => probe_stdio(refs, input, started),
        TransportType::StreamableHttp | TransportType::Sse => probe_http(refs, input, started),
    }
}

fn resolve_probe_vars(
    refs: &StateRefs,
    input: &McpServerInput,
    slot: &str,
) -> AppResult<(Vec<(String, String)>, Vec<String>)> {
    let source = if slot == "env" { &input.env } else { &input.headers };
    let mut pairs = Vec::new();
    let mut secret_values = Vec::new();
    for (key, value) in source {
        match value.kind {
            VarKind::Secret => {
                // Prefer the value the user just typed; fall back to the store
                // for definitions that are already saved.
                let typed = value.value.trim();
                let resolved = if !typed.is_empty() {
                    Some(typed.to_string())
                } else if let Some(id) = input.id.as_ref().filter(|id| !id.trim().is_empty()) {
                    refs.secrets.get(id, slot, key)?
                } else {
                    None
                };
                match resolved {
                    Some(secret) => {
                        secret_values.push(secret.clone());
                        pairs.push((key.clone(), secret));
                    }
                    None => {
                        return Err(AppError::validation(format!(
                            "The credential '{key}' has no stored value yet."
                        ))
                        .with_remediation("Enter the value, save the definition and try again."));
                    }
                }
            }
            _ => pairs.push((key.clone(), value.value.clone())),
        }
    }
    Ok((pairs, secret_values))
}

fn probe_stdio(
    refs: &StateRefs,
    input: &McpServerInput,
    started: Instant,
) -> AppResult<ConnectionTestResult> {
    let command = input.command.clone().unwrap_or_default();
    let (env, secret_values) = resolve_probe_vars(refs, input, "env")?;
    let target = if input.args.is_empty() {
        command.clone()
    } else {
        format!("{} {}", command, input.args.join(" "))
    };

    let mut process = std::process::Command::new(&command);
    process
        .args(&input.args)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = input.cwd.as_ref().filter(|value| !value.trim().is_empty()) {
        process.current_dir(cwd.trim());
    }

    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = scrub(
                &format!("The command could not be started: {error}"),
                &secret_values,
            );
            return Ok(ConnectionTestResult {
                mode: "stdioLaunch".into(),
                target,
                success: false,
                message,
                details: vec![format!("Working directory: {}", input.cwd.clone().unwrap_or_else(|| "inherited".into()))],
                duration_ms: elapsed_ms(started),
                executed_process: true,
                disclaimer: PROBE_DISCLAIMER.into(),
            });
        }
    };

    let deadline = Instant::now() + STDIO_PROBE_GRACE;
    let mut exit_status = None;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                exit_status = Some(status);
                break;
            }
            Ok(None) => {}
            Err(error) => {
                let message = scrub(&format!("The process could not be polled: {error}"), &secret_values);
                let _ = child.kill();
                let _ = child.wait();
                return Ok(ConnectionTestResult {
                    mode: "stdioLaunch".into(),
                    target,
                    success: false,
                    message,
                    details: Vec::new(),
                    duration_ms: elapsed_ms(started),
                    executed_process: true,
                    disclaimer: PROBE_DISCLAIMER.into(),
                });
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(75));
    }

    let (success, message, details) = match exit_status {
        Some(status) => {
            let output = child.wait_with_output().ok();
            let stderr = output
                .as_ref()
                .map(|output| summarize_output(&output.stderr))
                .unwrap_or_default();
            let stderr = scrub(&stderr, &secret_values);
            let mut details = Vec::new();
            if !stderr.is_empty() {
                details.push(format!("stderr: {stderr}"));
            }
            if status.success() {
                (
                    true,
                    "The command started and exited with code 0.".to_string(),
                    details,
                )
            } else {
                let code = status
                    .code()
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "terminated by a signal".into());
                (
                    false,
                    format!("The command started but exited immediately (exit code {code})."),
                    details,
                )
            }
        }
        None => {
            let _ = child.kill();
            let _ = child.wait();
            (
                true,
                format!(
                    "The command started and was still running after {} s.",
                    STDIO_PROBE_GRACE.as_secs()
                ),
                Vec::new(),
            )
        }
    };

    Ok(ConnectionTestResult {
        mode: "stdioLaunch".into(),
        target,
        success,
        message,
        details,
        duration_ms: elapsed_ms(started),
        executed_process: true,
        disclaimer: PROBE_DISCLAIMER.into(),
    })
}

fn probe_http(
    refs: &StateRefs,
    input: &McpServerInput,
    started: Instant,
) -> AppResult<ConnectionTestResult> {
    let url = input.url.clone().unwrap_or_default();
    let (headers, secret_values) = resolve_probe_vars(refs, input, "headers")?;

    let client = reqwest::blocking::Client::builder()
        .timeout(HTTP_PROBE_TIMEOUT)
        .build()
        .map_err(|error| {
            AppError::new(
                "NETWORK_ERROR",
                scrub(&format!("The HTTP client could not be created: {error}"), &secret_values),
            )
        })?;

    let mut request = client.get(&url);
    for (key, value) in headers {
        request = request.header(key, value);
    }

    match request.send() {
        Ok(response) => {
            let status = response.status();
            let mut details = vec![format!("HTTP {}", status.as_u16())];
            if !response.headers().is_empty() {
                details.push(format!(
                    "{} response header(s) received",
                    response.headers().len()
                ));
            }
            let (success, message) = if status.is_success() || status.is_redirection() {
                (
                    true,
                    format!("The endpoint answered with HTTP {}.", status.as_u16()),
                )
            } else if status.as_u16() == 401 || status.as_u16() == 403 {
                (
                    false,
                    format!(
                        "The endpoint answered but rejected the credentials (HTTP {}).",
                        status.as_u16()
                    ),
                )
            } else if status.as_u16() == 404 {
                (
                    false,
                    "The endpoint answered with HTTP 404: the URL path looks wrong.".to_string(),
                )
            } else if status.is_server_error() {
                (
                    false,
                    format!(
                        "The endpoint answered with a server error (HTTP {}).",
                        status.as_u16()
                    ),
                )
            } else {
                (
                    true,
                    format!(
                        "The endpoint is reachable (HTTP {}). A GET request may not be the method this server expects.",
                        status.as_u16()
                    ),
                )
            };
            Ok(ConnectionTestResult {
                mode: "httpReachability".into(),
                target: url,
                success,
                message,
                details,
                duration_ms: elapsed_ms(started),
                executed_process: false,
                disclaimer: PROBE_DISCLAIMER.into(),
            })
        }
        Err(error) => {
            let message = if error.is_timeout() {
                format!(
                    "The endpoint did not answer within {} s.",
                    HTTP_PROBE_TIMEOUT.as_secs()
                )
            } else {
                format!("The endpoint could not be reached: {error}")
            };
            Ok(ConnectionTestResult {
                mode: "httpReachability".into(),
                target: url,
                success: false,
                message: scrub(&message, &secret_values),
                details: Vec::new(),
                duration_ms: elapsed_ms(started),
                executed_process: false,
                disclaimer: PROBE_DISCLAIMER.into(),
            })
        }
    }
}

fn summarize_output(raw: &[u8]) -> String {
    let text = String::from_utf8_lossy(raw);
    let lines: Vec<&str> = text
        .split('\n')
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .collect();
    let tail = lines
        .iter()
        .rev()
        .take(4)
        .rev()
        .cloned()
        .collect::<Vec<&str>>()
        .join(" | ");
    if tail.chars().count() > 400 {
        let truncated: String = tail.chars().take(400).collect();
        format!("{truncated}…")
    } else {
        tail
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::VarValue;

    fn input(name: &str, transport: TransportType) -> McpServerInput {
        McpServerInput {
            id: None,
            name: name.into(),
            description: String::new(),
            transport,
            command: Some("npx".into()),
            args: vec!["-y".into(), "demo".into()],
            env: Default::default(),
            headers: Default::default(),
            url: Some("https://example.test/mcp".into()),
            cwd: None,
            enabled: true,
            targets: vec!["opencode".into()],
            tags: vec![],
            notes: String::new(),
        }
    }

    fn settings() -> AppSettings {
        AppSettings::defaults()
    }

    #[test]
    fn settings_validation_rejects_reserved_profile_ids() {
        let mut value = settings();
        value.generic_profiles.push(crate::models::GenericProfile {
            id: "opencode".into(),
            label: "Trying to shadow the built-in adapter".into(),
            config_path: "C:/tmp/x.json".into(),
            mcp_path: "mcpServers".into(),
            server_name_key: "name".into(),
            transport_mapping: Default::default(),
            custom_fields: vec![],
            verified: false,
        });
        assert!(validate_settings(&value).is_err());
    }

    #[test]
    fn settings_validation_requires_absolute_paths() {
        let mut value = settings();
        value.backup_dir = "relative/backups".into();
        assert!(validate_settings(&value).is_err());
    }

    #[test]
    fn masked_resolved_hides_secret_values() {
        let mut server = input_to_server(&input("demo", TransportType::Stdio));
        server.env.insert(
            "GITHUB_TOKEN".into(),
            VarValue { value: "ghp_supersecretvalue".into(), kind: VarKind::Secret },
        );
        let resolved = masked_resolved(&server);
        assert_eq!(
            resolved.env.get("GITHUB_TOKEN").and_then(Value::as_str),
            Some(REDACTED)
        );
        assert!(resolved.warnings.iter().any(|warning| warning.contains("GITHUB_TOKEN")));
    }

    #[test]
    fn scrub_removes_secret_values_from_diagnostics() {
        let scrubbed = scrub(
            "failed with token ghp_abcdef123456 in the request",
            &["ghp_abcdef123456".to_string()],
        );
        assert!(!scrubbed.contains("ghp_abcdef123456"));
        assert!(scrubbed.contains(REDACTED));
    }

    #[test]
    fn path_allow_list_matches_normalized_paths() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("opencode.json");
        std::fs::write(&file, "{}").unwrap();
        let allowed = vec![dir.path().join("./opencode.json")];
        assert!(path_is_allowed(&file, &allowed));
        assert!(!path_is_allowed(&dir.path().join("other.json"), &allowed));
    }

    #[test]
    fn summarize_output_keeps_the_tail() {
        let summary = summarize_output(b"line one\nline two\nline three\n");
        assert_eq!(summary, "line one | line two | line three");
    }

    #[test]
    fn probe_reports_validation_failure_without_executing() {
        let refs = StateRefs {
            db: std::sync::Arc::new(crate::database::Db::open_in_memory().unwrap()),
            secrets: std::sync::Arc::new(crate::security::secrets::SecretStore::new()),
            settings: std::sync::Arc::new(std::sync::RwLock::new(settings())),
            adapters: std::sync::Arc::new(std::sync::RwLock::new(vec![])),
            backup_dir: std::sync::Arc::new(std::sync::RwLock::new(std::path::PathBuf::from("."))),
            watcher: std::sync::Arc::new(std::sync::Mutex::new(None)),
            app: std::sync::Arc::new(std::sync::Mutex::new(None)),
        };
        let mut broken = input("broken", TransportType::Stdio);
        broken.command = Some(String::new());
        let result = probe_connection(&refs, &broken).unwrap();
        assert_eq!(result.mode, "jsonValidation");
        assert!(!result.success);
        assert!(!result.executed_process);
    }
}
