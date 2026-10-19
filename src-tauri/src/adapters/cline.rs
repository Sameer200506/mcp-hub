//! Cline adapter.
//!
//! Verified against the official Cline documentation:
//! * IDE extensions store MCP settings in the VS Code global storage folder:
//!   `%APPDATA%\Code\User\globalStorage\saoudrizwan.claude-dev\settings\cline_mcp_settings.json`
//!   (equivalent paths exist for VS Code Insiders, VSCodium, Cursor, Windsurf);
//! * Cline CLI stores the same document under `~/.cline/data/settings/cline_mcp_settings.json`
//!   and honours the `CLINE_MCP_SETTINGS_PATH` environment variable;
//! * entries live under `mcpServers`;
//! * stdio: `{ "command": "node", "args": [...], "env": {...}, "disabled": false, "autoApprove": [] }`;
//! * remote: `{ "type": "streamableHttp" | "sse", "url": ..., "headers": {...} }`.
//!
//! Because the settings file is owned by the extension, changes are picked up
//! when Cline reloads its MCP hub; the UI reports that a reload may be needed.

use std::path::PathBuf;


use crate::adapters::generic_json::EntryEncoding;
use crate::adapters::{binary_on_path, home_dir, DetectionResult, HarnessAdapter, HarnessHints};
use crate::models::{
    AdapterStatus, ConfigVariant, ExtraEntryField, HarnessCapabilities, ReloadRequirements,
};

const EXTENSION_DIR: &str = "saoudrizwan.claude-dev";

pub struct ClineAdapter;

impl ClineAdapter {
    fn vscode_like_candidates() -> Vec<(String, PathBuf)> {
        let mut candidates = Vec::new();
        let Some(appdata) = std::env::var_os("APPDATA") else {
            return candidates;
        };
        let base = PathBuf::from(appdata);
        let products = [
            ("VS Code", "Code"),
            ("VS Code Insiders", "Code - Insiders"),
            ("VSCodium", "VSCodium"),
            ("Cursor", "Cursor"),
            ("Windsurf", "Windsurf"),
        ];
        for (label, folder) in products {
            let path = base
                .join(folder)
                .join("User")
                .join("globalStorage")
                .join(EXTENSION_DIR)
                .join("settings")
                .join("cline_mcp_settings.json");
            candidates.push((label.to_string(), path));
        }
        candidates
    }

    fn cli_path() -> Option<PathBuf> {
        home_dir().map(|home| {
            home.join(".cline")
                .join("data")
                .join("settings")
                .join("cline_mcp_settings.json")
        })
    }
}

impl HarnessAdapter for ClineAdapter {
    fn id(&self) -> String {
        "cline".into()
    }

    fn display_name(&self) -> String {
        "Cline".into()
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
            cwd: false,
            enabled_flag: true,
            auto_approve: true,
            extra_entry_fields: vec![ExtraEntryField {
                key: "autoApprove".into(),
                description: "Tool names Cline may run without asking.".into(),
            }],
            notes: vec![
                "`disabled: true` keeps the entry in the file but stops Cline from starting it.".into(),
                "Omitting `type` means legacy SSE; this adapter always writes an explicit transport.".into(),
                "A working directory (`cwd`) is not part of the documented Cline schema and is skipped with a warning."
                    .into(),
            ],
        }
    }

    fn reload(&self) -> ReloadRequirements {
        ReloadRequirements {
            reload_required: true,
            description: "Cline reads the MCP settings file when it loads its MCP hub. Reopen the Cline panel (or reload the IDE window) after synchronizing."
                .into(),
        }
    }

    fn notes(&self) -> Vec<String> {
        vec![
            "The IDE extension and the Cline CLI use different files; the detected path is shown on the card and can be overridden in Settings."
                .into(),
            "This adapter edits the documented settings file only. Internal extension state can differ on older versions — the preview shows the exact JSON before writing."
                .into(),
        ]
    }

    fn config_format_label(&self) -> String {
        "Cline MCP settings JSON (`mcpServers` object)".into()
    }

    fn mcp_path(&self) -> Vec<String> {
        vec!["mcpServers".into()]
    }

    fn encoding(&self) -> EntryEncoding {
        let mut encoding = EntryEncoding::claude_style();
        // Cline has no working-directory field (`capabilities().cwd == false`):
        // writing one would produce a key the extension ignores, so drop it and
        // only report the warning produced by `validate_definition_against_harness`.
        encoding.cwd_field = None;
        encoding
    }

    fn detect_installation(&self) -> DetectionResult {
        let hints = HarnessHints::default();
        let config = self.locate_config(&hints);
        let config_exists = config.as_ref().map(|path| path.exists()).unwrap_or(false);
        let mut notes = Vec::new();
        if std::env::var_os("CLINE_MCP_SETTINGS_PATH").is_some() {
            notes.push("CLINE_MCP_SETTINGS_PATH is set and takes precedence.".into());
        }
        for (label, path) in Self::vscode_like_candidates() {
            if path.exists() {
                notes.push(format!("Existing {label} settings found: {}", path.display()));
            }
        }
        if let Some(cli) = Self::cli_path() {
            if cli.exists() {
                notes.push(format!("Cline CLI settings found: {}", cli.display()));
            }
        }
        if let Some(path) = binary_on_path(&["cline"]) {
            notes.push(format!("Cline CLI on PATH: {}", path.display()));
        }
        DetectionResult {
            detected: config_exists,
            confidence: if config_exists {
                "verified".into()
            } else if binary_on_path(&["cline"]).is_some() {
                "probable".into()
            } else {
                "notDetected".into()
            },
            version: None,
            notes,
        }
    }

    fn locate_config(&self, hints: &HarnessHints) -> Option<PathBuf> {
        if let Some(override_path) = hints.settings.config_overrides.get("cline") {
            if !override_path.trim().is_empty() {
                return Some(PathBuf::from(override_path));
            }
        }
        if let Some(env_path) = std::env::var_os("CLINE_MCP_SETTINGS_PATH") {
            let path = PathBuf::from(env_path);
            if !path.as_os_str().is_empty() {
                return Some(path);
            }
        }
        for (_, path) in Self::vscode_like_candidates() {
            if path.exists() {
                return Some(path);
            }
        }
        if let Some(cli) = Self::cli_path() {
            if cli.exists() {
                return Some(cli);
            }
        }
        // Nothing exists yet: default to the primary VS Code location.
        Self::vscode_like_candidates()
            .into_iter()
            .next()
            .map(|(_, path)| path)
    }

    fn candidate_configs(&self, hints: &HarnessHints) -> Vec<ConfigVariant> {
        let mut variants = Vec::new();
        if let Some(env_path) = std::env::var_os("CLINE_MCP_SETTINGS_PATH") {
            variants.push(ConfigVariant {
                path: PathBuf::from(env_path).to_string_lossy().to_string(),
                label: "CLINE_MCP_SETTINGS_PATH override".into(),
                exists: true,
                scope: "global".into(),
            });
        }
        if let Some(cli) = Self::cli_path() {
            variants.push(ConfigVariant {
                path: cli.to_string_lossy().to_string(),
                label: "Cline CLI settings".into(),
                exists: cli.exists(),
                scope: "global".into(),
            });
        }
        for (label, path) in Self::vscode_like_candidates() {
            variants.push(ConfigVariant {
                path: path.to_string_lossy().to_string(),
                label: format!("{label} global storage"),
                exists: path.exists(),
                scope: "global".into(),
            });
        }
        let _ = hints;
        variants
    }
}
