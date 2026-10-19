//! Optional configuration-file watcher.
//!
//! External changes are *never* overwritten automatically. The watcher only
//! marks the harness as externally modified (persisted in SQLite) and emits an
//! event so the UI can offer a diff. Writes performed by this application
//! update the recorded hash first, which prevents feedback loops.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult, Debouncer};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::database::Db;
use crate::errors::{AppError, AppResult};
use crate::models::HarnessState;
use crate::services::safe_io::read_snapshot;

pub struct WatcherHandle {
    _debouncer: Debouncer<notify::RecommendedWatcher>,
    watched: Vec<PathBuf>,
}

impl WatcherHandle {
    pub fn monitored(&self) -> &[PathBuf] {
        &self.watched
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigChangedPayload {
    pub harness_id: String,
    pub path: String,
}

pub fn start(
    app: AppHandle,
    db: Arc<Db>,
    targets: Vec<(String, PathBuf)>,
) -> AppResult<WatcherHandle> {
    let targets = targets
        .into_iter()
        .filter(|(_, path)| path.parent().is_some())
        .collect::<Vec<_>>();
    let targets_for_handler = targets.clone();

    let mut debouncer = new_debouncer(
        Duration::from_millis(900),
        move |result: DebounceEventResult| {
            let Ok(events) = result else {
                return;
            };
            for event in events {
                let changed = &event.path;
                for (harness_id, target) in &targets_for_handler {
                    let same_file = normalize(changed) == normalize(target);
                    if !same_file {
                        continue;
                    }
                    let current_hash = read_snapshot(target).ok().flatten().map(|snapshot| snapshot.sha256);
                    let state = db.get_harness_state(harness_id).ok().flatten();
                    let known = state.as_ref().and_then(|state| state.last_known_hash.clone());
                    let differs = match (&current_hash, &known) {
                        (Some(current), Some(known)) => current != known,
                        (Some(_), None) => true,
                        _ => false,
                    };
                    if !differs {
                        continue;
                    }
                    let updated = HarnessState {
                        harness_id: harness_id.clone(),
                        last_sync_at: state.as_ref().and_then(|state| state.last_sync_at.clone()),
                        last_error: state.as_ref().and_then(|state| state.last_error.clone()),
                        last_known_hash: known,
                        externally_modified: true,
                    };
                    if db.upsert_harness_state(&updated).is_ok() {
                        let _ = app.emit(
                            "harness-config-changed",
                            ConfigChangedPayload {
                                harness_id: harness_id.clone(),
                                path: target.to_string_lossy().to_string(),
                            },
                        );
                    }
                }
            }
        },
    )
    .map_err(|error| AppError::io(format!("could not start configuration watcher: {error}")))?;

    let mut watched = Vec::new();
    for (_, path) in &targets {
        if let Some(parent) = path.parent() {
            // Watch the directory (not the file) so atomic replace is reported.
            let parent = parent.to_path_buf();
            if debouncer
                .watcher()
                .watch(&parent, RecursiveMode::NonRecursive)
                .is_ok()
            {
                watched.push(parent);
            }
        }
    }

    Ok(WatcherHandle { _debouncer: debouncer, watched })
}

fn normalize(path: &std::path::Path) -> String {
    let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    canonical.to_string_lossy().to_lowercase()
}
