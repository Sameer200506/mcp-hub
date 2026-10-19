//! Harness detection: builds the `HarnessInfo` list the UI renders.
//!
//! Detection never executes the harness binary. It inspects documented
//! configuration locations, OS-specific install directories and PATH entries,
//! and reports a confidence level instead of guessing.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::adapters::antigravity::AntigravityAdapter;
use crate::adapters::cline::ClineAdapter;
use crate::adapters::generic_json::GenericAdapter;
use crate::adapters::opencode::OpenCodeAdapter;
use crate::adapters::{HarnessAdapter, HarnessHints};
use crate::database::{count_targets, Db};
use crate::errors::{AppError, AppResult};
use crate::models::{AppSettings, HarnessInfo};

pub fn hints_for(settings: &AppSettings) -> HarnessHints {
    HarnessHints {
        settings: settings.clone(),
        project_dir: if settings.detect_project_configs {
            std::env::current_dir().ok()
        } else {
            None
        },
    }
}

/// Build the adapter list. Built-in adapters always exist; generic profiles
/// come from settings and are appended as additional harnesses.
pub fn adapters_from_settings(settings: &AppSettings) -> Vec<Box<dyn HarnessAdapter>> {
    let mut adapters: Vec<Box<dyn HarnessAdapter>> = vec![
        Box::new(OpenCodeAdapter),
        Box::new(ClineAdapter),
        Box::new(AntigravityAdapter),
    ];
    for profile in &settings.generic_profiles {
        if profile.id.trim().is_empty() || profile.config_path.trim().is_empty() {
            continue;
        }
        adapters.push(Box::new(GenericAdapter::new(profile.clone())));
    }
    adapters
}

pub fn find_adapter<'a>(
    adapters: &'a [Box<dyn HarnessAdapter>],
    id: &str,
) -> AppResult<&'a dyn HarnessAdapter> {
    adapters
        .iter()
        .find(|adapter| adapter.id() == id)
        .map(|adapter| adapter.as_ref())
        .ok_or_else(|| AppError::not_found(format!("No adapter registered for harness '{id}'")))
}

pub fn build_harness_infos(
    db: &Db,
    adapters: &[Box<dyn HarnessAdapter>],
    settings: &AppSettings,
) -> AppResult<Vec<HarnessInfo>> {
    let servers = db.list_servers()?;
    let states = db.list_harness_states()?;
    let state_map: HashMap<String, crate::models::HarnessState> = states
        .into_iter()
        .map(|state| (state.harness_id.clone(), state))
        .collect();
    let hints = hints_for(settings);

    let mut infos = Vec::new();
    for adapter in adapters {
        let id = adapter.id();
        let detection = adapter.detect_installation();
        let config_path = adapter.locate_config(&hints);
        let variants = adapter.candidate_configs(&hints);
        let (config_exists, mcp_entries, mut notes) = match &config_path {
            Some(path) => match adapter.read_config(path) {
                Ok(document) => (
                    document.existed,
                    adapter.entry_count(&document),
                    Vec::new(),
                ),
                Err(error) => (false, 0, vec![format!("Could not read configuration: {}", error.message)]),
            },
            None => (false, 0, vec!["No configuration location could be resolved.".to_string()]),
        };
        notes.extend(detection.notes.clone());
        notes.extend(adapter.notes());

        let state = state_map.get(&id);
        let externally_modified = match (&config_path, state) {
            (Some(path), Some(state)) => match (crate::services::safe_io::read_snapshot(path), &state.last_known_hash) {
                (Ok(Some(snapshot)), Some(known)) => &snapshot.sha256 != known,
                _ => state.externally_modified,
            },
            _ => false,
        };
        let external_report = state.map(|item| item.externally_modified).unwrap_or(false);

        if mcp_entries > 0 {
            notes.push(format!("{mcp_entries} MCP entr{} currently present in this file.", if mcp_entries == 1 { "y" } else { "ies" }));
        }

        infos.push(HarnessInfo {
            id: id.clone(),
            name: adapter.display_name(),
            detected: detection.detected,
            detection_confidence: detection.confidence,
            config_path: config_path.map(|path| path.to_string_lossy().to_string()),
            config_format: adapter.config_format_label(),
            config_exists,
            active_configs: variants,
            capabilities: adapter.capabilities(),
            reload: adapter.reload(),
            adapter_status: adapter.adapter_status(),
            notes,
            last_sync_at: state.and_then(|item| item.last_sync_at.clone()),
            last_error: state.and_then(|item| item.last_error.clone()),
            assigned_mcp_count: count_targets(&servers, &id),
            externally_modified: externally_modified || external_report,
            installed_version: detection.version,
        });
    }
    Ok(infos)
}

/// Convenience helper for the settings screen: the default config paths.
pub fn default_config_paths(settings: &AppSettings) -> Vec<(String, Option<PathBuf>)> {
    let hints = hints_for(settings);
    adapters_from_settings(settings)
        .iter()
        .map(|adapter| (adapter.id(), adapter.locate_config(&hints)))
        .collect()
}
