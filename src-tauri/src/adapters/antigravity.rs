//! Google Antigravity adapter.
//!
//! Verified against the official Antigravity MCP documentation:
//! * global configuration: `~/.gemini/config/mcp_config.json`, shared by the
//!   Antigravity App, Antigravity CLI and Antigravity IDE;
//! * workspace configuration: `.agents/mcp_config.json` inside a project;
//! * entries live under `mcpServers`;
//! * stdio: `{ "command": "node", "args": [...], "env": {...}, "cwd": ...,
//!   "disabled": false }`;
//! * remote: `{ "serverUrl": "https://…", "headers": {...}, "oauth": {...},
//!   "authProviderType": "google_credentials", "disabledTools": [...] }`.
//!
//! Antigravity rejects an explicit `type` field, so this adapter never writes
//! one and removes it from entries it updates (legacy configs copied from
//! other clients often contain it).
//!
//! Only the global file is written automatically; the workspace file is listed
//! as a candidate and can be selected through the per-harness path override in
//! Settings, which is the explicit opt-in required before touching project
//! configuration.

use std::path::PathBuf;


use crate::adapters::generic_json::EntryEncoding;
use crate::adapters::{home_dir, DetectionResult, HarnessAdapter, HarnessHints};
use crate::models::{
    AdapterStatus, ConfigVariant, ExtraEntryField, HarnessCapabilities, ReloadRequirements,
};

pub struct AntigravityAdapter;

impl AntigravityAdapter {
    pub fn global_path() -> Option<PathBuf> {
        home_dir().map(|home| {
            home.join(".gemini")
                .join("config")
                .join("mcp_config.json")
        })
    }

    fn legacy_path() -> Option<PathBuf> {
        home_dir().map(|home| {
            home.join(".gemini")
                .join("antigravity")
                .join("mcp_config.json")
        })
    }

    fn install_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            dirs.push(PathBuf::from(local).join("Programs").join("Antigravity"));
        }
        if let Some(program_files) = std::env::var_os("ProgramFiles") {
            dirs.push(PathBuf::from(program_files).join("Antigravity"));
        }
        dirs
    }
}

impl HarnessAdapter for AntigravityAdapter {
    fn id(&self) -> String {
        "antigravity".into()
    }

    fn display_name(&self) -> String {
        "Google Antigravity".into()
    }

    fn adapter_status(&self) -> AdapterStatus {
        AdapterStatus::Verified
    }

    fn capabilities(&self) -> HarnessCapabilities {
        HarnessCapabilities {
            stdio: true,
            streamable_http: true,
            sse: true,
            headers: true,
            env: true,
            cwd: true,
            enabled_flag: true,
            auto_approve: false,
            extra_entry_fields: vec![
                ExtraEntryField {
                    key: "authProviderType".into(),
                    description: "`google_credentials` uses Application Default Credentials.".into(),
                },
                ExtraEntryField {
                    key: "oauth".into(),
                    description: "Pre-registered OAuth client id/secret.".into(),
                },
                ExtraEntryField {
                    key: "disabledTools".into(),
                    description: "Tool names withheld from the model.".into(),
                },
            ],
            notes: vec![
                "Antigravity rejects an explicit `type` field; transport is inferred from `command` vs `serverUrl`."
                    .into(),
                "Remote servers use `serverUrl` (Streamable HTTP or SSE).".into(),
                "`disabled: true` keeps the server configured but inactive.".into(),
            ],
        }
    }

    fn reload(&self) -> ReloadRequirements {
        ReloadRequirements {
            reload_required: true,
            description: "Antigravity reads mcp_config.json when the MCP servers are refreshed. Reopen the Agent panel or restart Antigravity after synchronizing; OAuth tokens are stored separately and are not touched."
                .into(),
        }
    }

    fn notes(&self) -> Vec<String> {
        vec![
            "Three install variants exist (App, CLI, IDE) and share this file. OAuth tokens live in ~/.gemini/antigravity/mcp_oauth_tokens.json and are never modified."
                .into(),
            "Workspace `.agents/mcp_config.json` files are listed for inspection but only written when the path override for this harness points at them."
                .into(),
        ]
    }

    fn config_format_label(&self) -> String {
        "Antigravity mcp_config.json (`mcpServers` object, no `type` field)".into()
    }

    fn mcp_path(&self) -> Vec<String> {
        vec!["mcpServers".into()]
    }

    fn encoding(&self) -> EntryEncoding {
        let mut encoding = EntryEncoding::claude_style();
        // Antigravity rejects an explicit `type` field. Declaring the field name
        // with no value for any transport makes `build_entry` remove it from
        // entries that legacy configs (copied from Claude Desktop, Cline, …)
        // shipped with.
        encoding.type_field = Some("type".into());
        encoding.stdio_type_value = None;
        encoding.remote_type_value = None;
        encoding.sse_type_value = None;
        encoding.url_field = "serverUrl".into();
        encoding.disabled_field = Some("disabled".into());
        encoding.enabled_field = None;
        encoding
    }

    fn detect_installation(&self) -> DetectionResult {
        let hints = HarnessHints::default();
        let config = self.locate_config(&hints);
        let config_exists = config.as_ref().map(|path| path.exists()).unwrap_or(false);
        let mut notes = Vec::new();
        for dir in Self::install_dirs() {
            if dir.exists() {
                notes.push(format!("Installation directory found: {}", dir.display()));
            }
        }
        if let Some(legacy) = Self::legacy_path() {
            if legacy.exists() && legacy != Self::global_path().unwrap_or_default() {
                notes.push(format!(
                    "Legacy Antigravity config also present at {} — verify which file your version reads.",
                    legacy.display()
                ));
            }
        }
        let installed_dir = Self::install_dirs().into_iter().any(|dir| dir.exists());
        DetectionResult {
            detected: config_exists || installed_dir,
            confidence: if config_exists {
                "verified".into()
            } else if installed_dir {
                "probable".into()
            } else {
                "notDetected".into()
            },
            version: None,
            notes,
        }
    }

    fn locate_config(&self, hints: &HarnessHints) -> Option<PathBuf> {
        if let Some(override_path) = hints.settings.config_overrides.get("antigravity") {
            if !override_path.trim().is_empty() {
                return Some(PathBuf::from(override_path));
            }
        }
        if let Some(global) = Self::global_path() {
            if global.exists() {
                return Some(global);
            }
        }
        if let Some(legacy) = Self::legacy_path() {
            if legacy.exists() {
                return Some(legacy);
            }
        }
        Self::global_path()
    }

    fn candidate_configs(&self, hints: &HarnessHints) -> Vec<ConfigVariant> {
        let mut variants = Vec::new();
        if let Some(global) = Self::global_path() {
            variants.push(ConfigVariant {
                path: global.to_string_lossy().to_string(),
                label: "Global config (App / CLI / IDE)".into(),
                exists: global.exists(),
                scope: "global".into(),
            });
        }
        if let Some(legacy) = Self::legacy_path() {
            variants.push(ConfigVariant {
                path: legacy.to_string_lossy().to_string(),
                label: "Legacy per-product config".into(),
                exists: legacy.exists(),
                scope: "global".into(),
            });
        }
        if let Some(project) = &hints.project_dir {
            let workspace = project.join(".agents").join("mcp_config.json");
            variants.push(ConfigVariant {
                path: workspace.to_string_lossy().to_string(),
                label: "Workspace config (written only when selected)".into(),
                exists: workspace.exists(),
                scope: "project".into(),
            });
        }
        variants
    }
}
