//! Shared application state handed to every Tauri command.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use tauri::AppHandle;

use crate::adapters::HarnessAdapter;
use crate::database::Db;
use crate::models::AppSettings;
use crate::security::secrets::SecretStore;
use crate::services::config_backup::BackupManager;
use crate::services::file_watcher::WatcherHandle;
use crate::services::harness_detection::adapters_from_settings;

/// Cloneable handle set so blocking work can move into a worker thread.
#[derive(Clone)]
pub struct StateRefs {
    pub db: Arc<Db>,
    pub secrets: Arc<SecretStore>,
    pub settings: Arc<RwLock<AppSettings>>,
    pub adapters: Arc<RwLock<Vec<Box<dyn HarnessAdapter>>>>,
    pub backup_dir: Arc<RwLock<PathBuf>>,
    pub watcher: Arc<Mutex<Option<WatcherHandle>>>,
    pub app: Arc<Mutex<Option<AppHandle>>>,
}

impl StateRefs {
    pub fn settings_snapshot(&self) -> AppSettings {
        self.settings
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    pub fn backup_manager(&self) -> BackupManager {
        let dir = self
            .backup_dir
            .read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| crate::database::default_data_dir().join("backups"));
        BackupManager::new(dir)
    }

    pub fn replace_adapters(&self, settings: &AppSettings) {
        if let Ok(mut guard) = self.adapters.write() {
            *guard = adapters_from_settings(settings);
        }
    }

    pub fn emit_progress(&self, harness_id: &str, message: &str) {
        if let Ok(guard) = self.app.lock() {
            if let Some(handle) = guard.as_ref() {
                use tauri::Emitter;
                let payload = serde_json::json!({ "harnessId": harness_id, "message": message });
                let _ = handle.emit("sync-progress", payload);
            }
        }
    }

    /// Notify the UI that a harness configuration changed (our own write or a restore).
    pub fn emit_config_changed(&self, harness_id: &str, path: Option<&str>) {
        if let Ok(guard) = self.app.lock() {
            if let Some(handle) = guard.as_ref() {
                use tauri::Emitter;
                let payload = serde_json::json!({
                    "harnessId": harness_id,
                    "path": path.unwrap_or_default(),
                });
                let _ = handle.emit("harness-config-changed", payload);
            }
        }
    }
}

pub struct AppState {
    pub refs: StateRefs,
    pub log_file: PathBuf,
    /// Kept alive for the lifetime of the app so the non-blocking log writer flushes.
    pub log_guard: Option<tracing_appender::non_blocking::WorkerGuard>,
}

impl AppState {
    pub fn new(db: Db, settings: AppSettings, log_file: PathBuf) -> Self {
        let refs = StateRefs {
            db: Arc::new(db),
            secrets: Arc::new(SecretStore::new()),
            settings: Arc::new(RwLock::new(settings.clone())),
            adapters: Arc::new(RwLock::new(adapters_from_settings(&settings))),
            backup_dir: Arc::new(RwLock::new(PathBuf::from(&settings.backup_dir))),
            watcher: Arc::new(Mutex::new(None)),
            app: Arc::new(Mutex::new(None)),
        };
        Self { refs, log_file, log_guard: None }
    }
}

impl StateRefs {
    /// Rebuild the file watcher for the current adapters/settings.
    pub fn restart_watcher(&self) -> crate::errors::AppResult<()> {
        let settings = self.settings_snapshot();
        let hints = crate::services::harness_detection::hints_for(&settings);
        let mut targets = Vec::new();
        if settings.watch_config_files {
            if let Ok(adapters) = self.adapters.read() {
                for adapter in adapters.iter() {
                    if let Some(path) = adapter.locate_config(&hints) {
                        targets.push((adapter.id(), path));
                    }
                }
            }
        }

        let app = self
            .app
            .lock()
            .ok()
            .and_then(|guard| guard.clone());

        let mut guard = self
            .watcher
            .lock()
            .map_err(|_| crate::errors::AppError::io("watcher lock poisoned"))?;
        *guard = None;
        if let Some(app) = app {
            if !targets.is_empty() {
                match crate::services::file_watcher::start(app, self.db.clone(), targets) {
                    Ok(handle) => *guard = Some(handle),
                    Err(error) => {
                        tracing::warn!(error = %error.message, "file watcher could not start");
                    }
                }
            }
        }
        Ok(())
    }
}
