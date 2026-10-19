//! OpenCode adapter.
//!
//! Verified against the official OpenCode documentation:
//! * global config: `~/.config/opencode/opencode.json` (JSON or JSONC);
//! * `OPENCODE_CONFIG` overrides the config path;
//! * servers live under the `mcp` object;
//! * local servers: `{ "type": "local", "command": ["npx", "-y", "pkg"],
//!   "environment": {…}, "cwd": …, "enabled": true, "timeout": … }`;
//! * remote servers: `{ "type": "remote", "url": …, "headers": {…},
//!   "oauth": {…}, "enabled": true }`.
//!
//! Every other top-level key (provider, permission, plugin, instructions,
//! keybinds, formatter, …) is preserved because only the `mcp` object is
//! modified. Project-level `opencode.json` files are listed but only written
//! when the user explicitly points the adapter at that file.

use std::path::PathBuf;


use crate::adapters::generic_json::EntryEncoding;
use crate::adapters::{binary_on_path, home_dir, DetectionResult, HarnessAdapter, HarnessHints};
use crate::models::{
    AdapterStatus, ConfigVariant, ExtraEntryField, HarnessCapabilities, ReloadRequirements,
};

pub struct OpenCodeAdapter;

impl OpenCodeAdapter {
    pub fn global_candidates() -> Vec<PathBuf> {
        let mut out = Vec::new();
        if let Some(home) = home_dir() {
            out.push(home.join(".config").join("opencode").join("opencode.json"));
            out.push(home.join(".config").join("opencode").join("opencode.jsonc"));
        }
        out
    }
}

impl HarnessAdapter for OpenCodeAdapter {
    fn id(&self) -> String {
        "opencode".into()
    }

    fn display_name(&self) -> String {
        "OpenCode".into()
    }

    fn adapter_status(&self) -> AdapterStatus {
        AdapterStatus::Verified
    }

    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            stdio: true,
            streamable_http: true,
            sse: false,
            headers: true,
            env: true,
            cwd: true,
            enabled_flag: true,
            auto_approve: false,
            extra_entry_fields: vec![
                ExtraEntryField {
                    key: "timeout".into(),
                    description: "Milliseconds allowed when fetching tools from the server.".into(),
                },
                ExtraEntryField {
                    key: "oauth".into(),
                    description: "OAuth client credentials or `false` to disable auto-detection.".into(),
                },
            ],
            notes: vec![
                "Local servers use a single `command` array containing the executable and its arguments.".into(),
                "Remote servers are Streamable HTTP; SSE is not part of the documented schema.".into(),
                "`enabled: false` keeps the server in the file but stops OpenCode from starting it.".into(),
            ],
        }
    }

    fn reload(&self) -> ReloadRequirements {
        ReloadRequirements {
            reload_required: true,
            description: "OpenCode reads its configuration at startup. Restart the TUI/CLI (or start a new session) to pick up changes."
                .into(),
        }
    }

    fn notes(&self) -> Vec<String> {
        vec![
            "Global config supports JSONC; comments are normalised into plain JSON when the file is rewritten by a synchronization."
                .into(),
            "OpenCode merges global and project configs. This adapter targets the global file unless you override the path in Settings."
                .into(),
        ]
    }

    fn config_format_label(&self) -> String {
        "OpenCode JSON/JSONC (`mcp` object)".into()
    }

    fn mcp_path(&self) -> Vec<String> {
        vec!["mcp".into()]
    }

    fn encoding(&self) -> EntryEncoding {
        let mut encoding = EntryEncoding::claude_style();
        encoding.type_field = Some("type".into());
        encoding.stdio_type_value = Some("local".into());
        encoding.remote_type_value = Some("remote".into());
        encoding.sse_type_value = None;
        encoding.command_as_array = true;
        encoding.args_field = None;
        encoding.env_field = Some("environment".into());
        encoding.disabled_field = None;
        encoding.enabled_field = Some("enabled".into());
        encoding
    }

    fn detect_installation(&self) -> DetectionResult {
        let hints = HarnessHints::default();
        let config = self.locate_config(&hints);
        let config_exists = config.as_ref().map(|path| path.exists()).unwrap_or(false);
        let binary = binary_on_path(&["opencode"]);
        let mut notes = Vec::new();
        if let Some(path) = &binary {
            notes.push(format!("`opencode` found on PATH: {}", path.display()));
        }
        if !config_exists {
            notes.push(
                "No OpenCode config found yet; synchronizing will create ~/.config/opencode/opencode.json."
                    .into(),
            );
        }
        DetectionResult {
            detected: config_exists || binary.is_some(),
            confidence: if config_exists {
                "envOverrideOrConfigPresent".into()
            } else if binary.is_some() {
                "probable".into()
            } else {
                "notDetected".into()
            },
            version: None,
            notes,
        }
    }

    fn locate_config(&self, hints: &HarnessHints) -> Option<PathBuf> {
        if let Some(override_path) = hints.settings.config_overrides.get("opencode") {
            if !override_path.trim().is_empty() {
                return Some(PathBuf::from(override_path));
            }
        }
        if let Some(env_path) = std::env::var_os("OPENCODE_CONFIG") {
            let path = PathBuf::from(env_path);
            if !path.as_os_str().is_empty() {
                return Some(path);
            }
        }
        let candidates = Self::global_candidates();
        for candidate in &candidates {
            if candidate.exists() {
                return Some(candidate.clone());
            }
        }
        candidates.into_iter().next()
    }

    fn candidate_configs(&self, hints: &HarnessHints) -> Vec<ConfigVariant> {
        let mut variants = Vec::new();
        if let Some(env_path) = std::env::var_os("OPENCODE_CONFIG") {
            variants.push(ConfigVariant {
                path: PathBuf::from(env_path).to_string_lossy().to_string(),
                label: "OPENCODE_CONFIG environment override".into(),
                exists: true,
                scope: "global".into(),
            });
        }
        for (index, candidate) in Self::global_candidates().into_iter().enumerate() {
            variants.push(ConfigVariant {
                path: candidate.to_string_lossy().to_string(),
                label: if index == 0 { "Global config".into() } else { "Global config (JSONC)".into() },
                exists: candidate.exists(),
                scope: "global".into(),
            });
        }
        if hints.settings.detect_project_configs {
            if let Some(project) = &hints.project_dir {
                let project_path = project.join("opencode.json");
                variants.push(ConfigVariant {
                    path: project_path.to_string_lossy().to_string(),
                    label: "Project config (not modified unless selected)".into(),
                    exists: project_path.exists(),
                    scope: "project".into(),
                });
            }
        }
        variants
    }
}
