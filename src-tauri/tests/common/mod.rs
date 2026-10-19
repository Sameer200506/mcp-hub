//! Shared fixture for the Universal MCP Manager integration tests.
//!
//! These tests drive the same service layer the Tauri commands call: adapters,
//! the synchronization engine, the registry, the backup manager and the secret
//! store. Every configuration file lives inside a `tempfile::TempDir` and is
//! reached through the per-harness path override, so no real harness file and
//! no user profile data is touched.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use universal_mcp_manager_lib::adapters::{HarnessAdapter, HarnessHints};
use universal_mcp_manager_lib::database::Db;
use universal_mcp_manager_lib::models::{
    AppSettings, GenericProfile, McpServer, PreviewReport, RemovalRequest, ResolutionSelection,
    SyncReport, SyncRequest, TransportType, VarValue,
};
use universal_mcp_manager_lib::security::secrets::SecretStore;
use universal_mcp_manager_lib::services::config_backup::BackupManager;
use universal_mcp_manager_lib::services::config_sync::{self, SyncContext};
use universal_mcp_manager_lib::services::harness_detection::{
    adapters_from_settings, find_adapter, hints_for,
};

/// A temporary workspace with a registry database and an empty backup folder.
pub struct Fixture {
    pub root: PathBuf,
    pub db: Db,
    pub secrets: SecretStore,
    pub backups: BackupManager,
    pub settings: AppSettings,
    pub adapters: Vec<Box<dyn HarnessAdapter>>,
    temp: tempfile::TempDir,
}

impl Fixture {
    pub fn new() -> Self {
        let temp = tempfile::tempdir().expect("temporary workspace");
        let root = std::fs::canonicalize(temp.path()).unwrap_or_else(|_| temp.path().to_path_buf());
        let db = Db::open(&root.join("registry.sqlite3")).expect("registry database");

        let mut settings = AppSettings::defaults();
        settings.data_dir = root.to_string_lossy().into_owned();
        settings.backup_dir = root.join("backups").to_string_lossy().into_owned();
        settings.create_backups = true;
        settings.watch_config_files = false;
        settings.detect_project_configs = false;

        let backups = BackupManager::new(root.join("backups"));
        let adapters = adapters_from_settings(&settings);
        Self {
            root,
            db,
            secrets: SecretStore::new(),
            backups,
            settings,
            adapters,
            temp,
        }
    }

    /// Absolute path inside the temporary workspace.
    pub fn path(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Point a built-in harness at a file inside the workspace.
    pub fn point_at(&mut self, harness_id: &str, relative: &str) -> PathBuf {
        let path = self.path(relative);
        self.settings
            .config_overrides
            .insert(harness_id.to_string(), path.to_string_lossy().into_owned());
        self.adapters = adapters_from_settings(&self.settings);
        path
    }

    /// Register a generic JSON profile pointing at a file inside the workspace.
    pub fn add_generic_profile(&mut self, id: &str, relative: &str, verified: bool) -> PathBuf {
        let path = self.path(relative);
        self.settings
            .generic_profiles
            .push(universal_mcp_manager_lib::models::GenericProfile {
                id: id.to_string(),
                label: format!("{id} profile"),
                config_path: path.to_string_lossy().into_owned(),
                mcp_path: "mcpServers".into(),
                server_name_key: "name".into(),
                transport_mapping: Default::default(),
                custom_fields: vec![],
                verified,
            });
        self.adapters = adapters_from_settings(&self.settings);
        path
    }

    /// Change a registered generic profile and rebuild the adapters, so the
    /// next `adapter()` call sees the new mapping.
    pub fn mutate_profile(&mut self, id: &str, change: impl FnOnce(&mut GenericProfile)) {
        let profile = self
            .settings
            .generic_profiles
            .iter_mut()
            .find(|profile| profile.id == id)
            .unwrap_or_else(|| panic!("no generic profile registered as '{id}'"));
        change(profile);
        self.adapters = adapters_from_settings(&self.settings);
    }

    pub fn adapter(&self, harness_id: &str) -> &dyn HarnessAdapter {
        find_adapter(&self.adapters, harness_id).expect("adapter registered")
    }

    pub fn hints(&self) -> HarnessHints {
        hints_for(&self.settings)
    }

    pub fn ctx(&self) -> SyncContext<'_> {
        SyncContext {
            db: &self.db,
            secrets: &self.secrets,
            backups: &self.backups,
            adapters: &self.adapters,
            settings: &self.settings,
        }
    }

    // ------------------------------------------------------------- filesystem

    pub fn write(&self, path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("parent directory");
        }
        std::fs::write(path, content).expect("write fixture file");
    }

    pub fn write_json(&self, path: &Path, value: &Value) {
        let rendered = serde_json::to_string_pretty(value).expect("serialise fixture JSON");
        self.write(path, &rendered);
    }

    pub fn read(&self, path: &Path) -> String {
        std::fs::read_to_string(path).expect("read fixture file")
    }

    pub fn read_json(&self, path: &Path) -> Value {
        serde_json::from_str(&self.read(path)).expect("fixture file is valid JSON")
    }

    pub fn set_read_only(&self, path: &Path, read_only: bool) {
        let mut permissions = std::fs::metadata(path).expect("metadata").permissions();
        permissions.set_readonly(read_only);
        std::fs::set_permissions(path, permissions).expect("permissions");
    }

    // --------------------------------------------------------------- registry

    pub fn insert_server(&self, server: &McpServer) {
        self.db.insert_server(server).expect("insert server");
    }

    // ------------------------------------------------------------- sync engine

    pub fn preview(&self, request: &SyncRequest) -> PreviewReport {
        config_sync::build_preview(&self.ctx(), request).expect("preview")
    }

    pub fn apply(&self, request: &SyncRequest) -> SyncReport {
        config_sync::apply(&self.ctx(), request, &|_, _| {}).expect("apply")
    }

    pub fn harness_infos(&self) -> Vec<universal_mcp_manager_lib::models::HarnessInfo> {
        universal_mcp_manager_lib::services::harness_detection::build_harness_infos(
            &self.db,
            &self.adapters,
            &self.settings,
        )
        .expect("harness infos")
    }
}

// ------------------------------------------------------------------ factories

/// A stdio definition using `npx -y @example/<name>`.
pub fn stdio_server(name: &str, targets: &[&str]) -> McpServer {
    McpServer {
        id: format!("id-{name}"),
        name: name.to_string(),
        description: String::new(),
        transport: TransportType::Stdio,
        command: Some("npx".into()),
        args: vec!["-y".into(), format!("@example/{name}")],
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        url: None,
        cwd: None,
        enabled: true,
        targets: targets.iter().map(|target| (*target).to_string()).collect(),
        tags: vec![],
        notes: String::new(),
        created_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
        last_sync_at: None,
    }
}

/// A Streamable HTTP definition.
pub fn remote_server(name: &str, url: &str, targets: &[&str]) -> McpServer {
    let mut server = stdio_server(name, targets);
    server.transport = TransportType::StreamableHttp;
    server.command = None;
    server.args = vec![];
    server.url = Some(url.to_string());
    server
}

pub fn var_literal(value: &str) -> VarValue {
    VarValue::literal(value)
}

/// A request that targets the given harnesses and every registry entry.
pub fn request_for(harnesses: &[&str]) -> SyncRequest {
    SyncRequest {
        harness_ids: Some(harnesses.iter().map(|value| (*value).to_string()).collect()),
        ..SyncRequest::default()
    }
}

pub fn resolution(harness_id: &str, name: &str, action: &str) -> ResolutionSelection {
    ResolutionSelection {
        harness_id: harness_id.into(),
        name: name.into(),
        action: action.into(),
        new_name: None,
    }
}

pub fn rename_resolution(harness_id: &str, name: &str, new_name: &str) -> ResolutionSelection {
    ResolutionSelection {
        harness_id: harness_id.into(),
        name: name.into(),
        action: "rename".into(),
        new_name: Some(new_name.into()),
    }
}

pub fn removal(harness_id: &str, name: &str) -> RemovalRequest {
    RemovalRequest {
        harness_id: harness_id.into(),
        name: name.into(),
    }
}

/// Read one server entry out of a harness document.
pub fn entry<'a>(document: &'a Value, container: &str, name: &str) -> &'a Value {
    document
        .get(container)
        .and_then(|section| section.get(name))
        .unwrap_or_else(|| panic!("no '{name}' entry under '{container}' in {document}"))
}

/// Read the entry of the only preview in a report.
pub fn only_preview(report: &PreviewReport) -> &universal_mcp_manager_lib::models::SyncPreview {
    assert_eq!(report.previews.len(), 1, "expected exactly one preview");
    &report.previews[0]
}

pub fn only_result(report: &SyncReport) -> &universal_mcp_manager_lib::models::HarnessSyncResult {
    assert_eq!(report.results.len(), 1, "expected exactly one sync result");
    &report.results[0]
}
