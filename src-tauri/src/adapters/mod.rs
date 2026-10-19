//! Harness adapter interface.
//!
//! Every supported harness has its own adapter module that declares what it
//! supports. The shared pipeline (read → validate → merge → safe write →
//! verify) lives here and is exposed through default trait methods, so an
//! adapter only has to describe its schema.

pub mod antigravity;
pub mod cline;
pub mod generic_json;
pub mod opencode;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::errors::{AppResult};
use crate::models::{
    AdapterStatus, AppSettings, ConfigVariant, HarnessCapabilities, McpServer, ReloadRequirements,
    TransportType, ValidationIssue,
};

use self::generic_json::{
    build_entry, canonical_json, entry_names, get_entry, parse_document, remove_entry, set_entry,
    EntryEncoding, ResolvedVars,
};

/// A configuration file as read from disk. `value` is `Null` when the file
/// exists but could not be parsed (see `malformed`).
#[derive(Debug, Clone)]
pub struct ConfigDocument {
    pub harness_id: String,
    pub path: PathBuf,
    pub existed: bool,
    pub raw: Option<String>,
    pub value: Value,
    pub format: String,
    pub hash: Option<String>,
    pub read_only: bool,
    pub malformed: Option<String>,
    pub size: u64,
}

impl ConfigDocument {
    pub fn empty_for(harness_id: &str, path: &Path) -> Self {
        Self {
            harness_id: harness_id.to_string(),
            path: path.to_path_buf(),
            existed: false,
            raw: None,
            value: Value::Object(Default::default()),
            format: "JSON".into(),
            hash: None,
            read_only: false,
            malformed: None,
            size: 0,
        }
    }

    pub fn canonical(&self) -> String {
        canonical_json(&self.value)
    }
}

#[derive(Debug, Clone, Default)]
pub struct DetectionResult {
    pub detected: bool,
    pub confidence: String,
    pub version: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct HarnessHints {
    pub settings: AppSettings,
    pub project_dir: Option<PathBuf>,
}

pub trait HarnessAdapter: Send + Sync {
    // ------------------------------------------------------------- identity
    fn id(&self) -> String;
    fn display_name(&self) -> String;
    fn adapter_status(&self) -> AdapterStatus;
    fn capabilities(&self) -> HarnessCapabilities;
    fn reload(&self) -> ReloadRequirements;
    fn notes(&self) -> Vec<String>;
    fn config_format_label(&self) -> String;

    /// JSON object path that holds the server entries (e.g. `["mcp"]`).
    fn mcp_path(&self) -> Vec<String>;
    /// How entries are encoded for this harness.
    fn encoding(&self) -> EntryEncoding;

    // ------------------------------------------------------------ interface
    fn detect_installation(&self) -> DetectionResult;
    fn locate_config(&self, hints: &HarnessHints) -> Option<PathBuf>;
    fn candidate_configs(&self, hints: &HarnessHints) -> Vec<ConfigVariant>;

    fn supports_transport(&self, transport: TransportType) -> bool {
        let caps = self.capabilities();
        match transport {
            TransportType::Stdio => caps.stdio,
            TransportType::StreamableHttp => caps.streamable_http,
            TransportType::Sse => caps.sse,
        }
    }

    /// Read the configuration, tolerating JSONC. Never fails on malformed JSON:
    /// the document is marked malformed instead so the UI can refuse to write.
    fn read_config(&self, path: &Path) -> AppResult<ConfigDocument> {
        load_document(&self.id(), path)
    }

    fn validate_config(&self, document: &ConfigDocument) -> Vec<ValidationIssue> {
        let mut issues = Vec::new();
        if let Some(error) = &document.malformed {
            issues.push(ValidationIssue::error("file", error.clone()));
        }
        if document.read_only {
            issues.push(ValidationIssue::error(
                "file",
                "configuration file is read-only; writing requires changing file permissions",
            ));
        }
        let mcp_path = self.mcp_path();
        let path_label = mcp_path.join(".");
        if let Some(value) = generic_json::lookup(&document.value, &mcp_path) {
            if !value.is_object() {
                issues.push(ValidationIssue::error(
                    path_label.as_str(),
                    "expected a JSON object holding server entries",
                ));
            }
        }
        for entry in self.existing_entries(document) {
            if !entry.value.is_object() {
                issues.push(ValidationIssue::warning(
                    &entry.name,
                    "existing entry is not an object; synchronization will treat it as a conflict",
                ));
            }
        }
        issues
    }

    /// Encode a registry definition into this harness' schema, preserving
    /// unknown fields of an existing entry.
    fn merge_mcp_definition(
        &self,
        document: &ConfigDocument,
        definition: &McpServer,
        resolved: &ResolvedVars,
    ) -> AppResult<Value> {
        let existing = self.existing_entry(document, &definition.name);
        build_entry(&self.encoding(), definition, existing.as_ref(), resolved)
    }

    fn remove_mcp_definition(&self, document: &ConfigDocument, name: &str) -> AppResult<bool> {
        let mut value = document.value.clone();
        remove_entry(&mut value, &self.mcp_path(), name)
    }

    fn existing_entry(&self, document: &ConfigDocument, name: &str) -> Option<Value> {
        get_entry(&document.value, &self.mcp_path(), name)
    }

    fn existing_entries(&self, document: &ConfigDocument) -> Vec<NamedEntry> {
        let names = entry_names(&document.value, &self.mcp_path());
        names
            .into_iter()
            .filter_map(|name| {
                get_entry(&document.value, &self.mcp_path(), &name)
                    .map(|value| NamedEntry { name, value })
            })
            .collect()
    }

    fn entry_count(&self, document: &ConfigDocument) -> usize {
        self.existing_entries(document).len()
    }

    /// Apply a set of entry mutations to the document, returning a new document
    /// whose `value` is the proposed configuration.
    fn propose(
        &self,
        document: &ConfigDocument,
        upserts: &BTreeMap<String, Value>,
        removals: &[String],
    ) -> AppResult<ConfigDocument> {
        let mut proposed = document.clone();
        for (name, entry) in upserts {
            set_entry(&mut proposed.value, &self.mcp_path(), name, entry.clone())?;
        }
        for name in removals {
            remove_entry(&mut proposed.value, &self.mcp_path(), name)?;
        }
        Ok(proposed)
    }

    /// Safe write wrapper: verifies against the expected hash, writes to a
    /// temporary file and replaces the original atomically.
    fn write_config_safely(
        &self,
        path: &Path,
        content: &str,
        expected_hash: Option<&str>,
    ) -> AppResult<crate::services::safe_io::WriteOutcome> {
        crate::services::safe_io::write_atomic(path, content, expected_hash)
    }

    fn get_supported_capabilities(&self) -> HarnessCapabilities {
        self.capabilities()
    }

    fn get_reload_requirements(&self) -> ReloadRequirements {
        self.reload()
    }
}

#[derive(Debug, Clone)]
pub struct NamedEntry {
    pub name: String,
    pub value: Value,
}

pub fn load_document(harness_id: &str, path: &Path) -> AppResult<ConfigDocument> {
    let snapshot = crate::services::safe_io::read_snapshot(path)?;
    match snapshot {
        Some(snapshot) => {
            let (value, format) = match parse_document(&snapshot.content) {
                Ok(parsed) => parsed,
                Err(error) => {
                    return Ok(ConfigDocument {
                        harness_id: harness_id.to_string(),
                        path: path.to_path_buf(),
                        existed: true,
                        raw: Some(snapshot.content),
                        value: Value::Null,
                        format: "malformed".into(),
                        hash: Some(snapshot.sha256),
                        read_only: snapshot.read_only,
                        malformed: Some(error.message),
                        size: snapshot.size,
                    });
                }
            };
            Ok(ConfigDocument {
                harness_id: harness_id.to_string(),
                path: path.to_path_buf(),
                existed: true,
                raw: Some(snapshot.content),
                value,
                format: format.label().to_string(),
                hash: Some(snapshot.sha256),
                read_only: snapshot.read_only,
                malformed: None,
                size: snapshot.size,
            })
        }
        None => Ok(ConfigDocument::empty_for(harness_id, path)),
    }
}

/// Locate a binary on PATH without executing it.
pub fn binary_on_path(names: &[&str]) -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let extensions = ["", ".exe", ".cmd", ".bat", ".ps1"];
    for dir in std::env::split_paths(&path_var) {
        for name in names {
            let base = dir.join(name);
            for extension in extensions {
                let candidate = if extension.is_empty() {
                    base.clone()
                } else {
                    PathBuf::from(format!("{}{}", base.to_string_lossy(), extension))
                };
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }
    }
    None
}

pub fn home_dir() -> Option<PathBuf> {
    dirs::home_dir()
}

pub fn allowlist_for(adapter: &dyn HarnessAdapter, hints: &HarnessHints) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = adapter.locate_config(hints) {
        paths.push(path);
    }
    for variant in adapter.candidate_configs(hints) {
        paths.push(PathBuf::from(variant.path));
    }
    paths
}
