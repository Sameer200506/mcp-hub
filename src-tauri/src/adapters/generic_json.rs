//! Generic JSON / JSONC document handling and shared entry encoding.
//!
//! Every harness adapter builds on these helpers so that unknown JSON fields
//! are preserved on round-trip and the encoding rules stay in one place.

use serde_json::{json, Map, Value};

use crate::errors::{AppError, AppResult};
use crate::models::{McpServer, TransportType};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    Json,
    Jsonc,
}

impl ConfigFormat {
    pub fn label(&self) -> &'static str {
        match self {
            ConfigFormat::Json => "JSON",
            ConfigFormat::Jsonc => "JSONC (comments preserved on read, normalised on write)",
        }
    }
}

/// Strips `//` / `/* */` comments and trailing commas without touching strings.
pub fn strip_jsonc(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        match ch {
            '"' => {
                in_string = true;
                out.push(ch);
            }
            '/' => match chars.peek() {
                Some('/') => {
                    for next in chars.by_ref() {
                        if next == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                }
                Some('*') => {
                    chars.next();
                    let mut prev = '\0';
                    for next in chars.by_ref() {
                        if next == '\n' {
                            out.push('\n');
                        }
                        if prev == '*' && next == '/' {
                            break;
                        }
                        prev = next;
                    }
                }
                _ => out.push(ch),
            },
            ',' => out.push(','),
            _ => out.push(ch),
        }
    }

    // Remove trailing commas before } or ].
    let bytes: Vec<char> = out.chars().collect();
    let mut cleaned = String::with_capacity(out.len());
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in bytes.iter().enumerate() {
        if in_string {
            cleaned.push(*ch);
            if escaped {
                escaped = false;
            } else if *ch == '\\' {
                escaped = true;
            } else if *ch == '"' {
                in_string = false;
            }
            continue;
        }
        if *ch == '"' {
            in_string = true;
            cleaned.push(*ch);
            continue;
        }
        if *ch == ',' {
            let mut lookahead = index + 1;
            while lookahead < bytes.len() && bytes[lookahead].is_whitespace() {
                lookahead += 1;
            }
            if lookahead < bytes.len() && (bytes[lookahead] == '}' || bytes[lookahead] == ']') {
                continue;
            }
        }
        cleaned.push(*ch);
    }
    cleaned
}

/// Parse a configuration document, tolerating JSONC when necessary.
pub fn parse_document(raw: &str) -> AppResult<(Value, ConfigFormat)> {
    match serde_json::from_str::<Value>(raw) {
        Ok(value) => Ok((value, ConfigFormat::Json)),
        Err(strict_error) => {
            let stripped = strip_jsonc(raw);
            match serde_json::from_str::<Value>(&stripped) {
                Ok(value) => Ok((value, ConfigFormat::Jsonc)),
                Err(_) => Err(AppError::validation(format!(
                    "configuration is not valid JSON (line {}, column {}): {}",
                    strict_error.line(),
                    strict_error.column(),
                    strict_error
                ))
                .with_remediation("Fix the JSON manually or restore a backup before synchronizing.")),
            }
        }
    }
}

pub fn canonical_json(value: &Value) -> String {
    let mut rendered = serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into());
    rendered.push('\n');
    rendered
}

/// Navigate a dot path such as `mcp` or `mcpServers`.
pub fn path_segments(path: &str) -> Vec<String> {
    path.split('.')
        .map(|segment| segment.trim().to_string())
        .filter(|segment| !segment.is_empty())
        .collect()
}

pub fn lookup<'a>(root: &'a Value, path: &[String]) -> Option<&'a Value> {
    let mut current = root;
    for segment in path {
        current = current.get(segment)?;
    }
    Some(current)
}

pub fn lookup_object<'a>(root: &'a Value, path: &[String]) -> Option<&'a Map<String, Value>> {
    lookup(root, path).and_then(Value::as_object)
}

pub fn ensure_object<'a>(root: &'a mut Value, path: &[String]) -> AppResult<&'a mut Map<String, Value>> {
    if path.is_empty() {
        return root
            .as_object_mut()
            .ok_or_else(|| AppError::validation("configuration root must be a JSON object"));
    }
    let mut current = root;
    for segment in path {
        if !current.is_object() {
            *current = Value::Object(Map::new());
        }
        let map = current.as_object_mut().expect("object");
        if !map.contains_key(segment) || !map[segment].is_object() {
            map.insert(segment.clone(), Value::Object(Map::new()));
        }
        current = map.get_mut(segment).expect("inserted");
    }
    Ok(current.as_object_mut().expect("object"))
}

/// Remove the MCP container when it became empty.
pub fn prune_empty(root: &mut Value, path: &[String]) {
    if path.is_empty() {
        return;
    }
    let mut cursor = root;
    for segment in &path[..path.len() - 1] {
        match cursor.get_mut(segment) {
            Some(next) => cursor = next,
            None => return,
        }
    }
    if let Some(map) = cursor.as_object_mut() {
        let last = &path[path.len() - 1];
        if let Some(child) = map.get(last) {
            if child.as_object().map(|object| object.is_empty()).unwrap_or(false) {
                map.remove(last);
            }
        }
    }
}

pub fn get_entry(root: &Value, path: &[String], name: &str) -> Option<Value> {
    lookup_object(root, path).and_then(|object| object.get(name).cloned())
}

pub fn set_entry(root: &mut Value, path: &[String], name: &str, entry: Value) -> AppResult<()> {
    ensure_object(root, path)?.insert(name.to_string(), entry);
    Ok(())
}

pub fn remove_entry(root: &mut Value, path: &[String], name: &str) -> AppResult<bool> {
    let removed = ensure_object(root, path)?.remove(name).is_some();
    prune_empty(root, path);
    Ok(removed)
}

pub fn entry_names(root: &Value, path: &[String]) -> Vec<String> {
    lookup_object(root, path)
        .map(|object| object.keys().cloned().collect())
        .unwrap_or_default()
}

/// How a harness expresses transport selection inside an entry.
#[derive(Debug, Clone, Default)]
pub struct EntryEncoding {
    /// Field used for the transport discriminator (e.g. `type`), if any.
    pub type_field: Option<String>,
    pub stdio_type_value: Option<String>,
    pub remote_type_value: Option<String>,
    pub sse_type_value: Option<String>,
    pub url_field: String,
    pub command_field: String,
    /// `command: ["npx", "-y", ...]` (OpenCode) instead of command + args.
    pub command_as_array: bool,
    pub args_field: Option<String>,
    pub env_field: Option<String>,
    pub headers_field: Option<String>,
    pub cwd_field: Option<String>,
    pub disabled_field: Option<String>,
    /// Harnesses that express enablement as `enabled: true|false` (OpenCode).
    pub enabled_field: Option<String>,
    pub custom_fields: Vec<(String, Value)>,
}

impl EntryEncoding {
    pub fn claude_style() -> Self {
        Self {
            type_field: Some("type".into()),
            stdio_type_value: Some("stdio".into()),
            remote_type_value: Some("streamableHttp".into()),
            sse_type_value: Some("sse".into()),
            url_field: "url".into(),
            command_field: "command".into(),
            command_as_array: false,
            args_field: Some("args".into()),
            env_field: Some("env".into()),
            headers_field: Some("headers".into()),
            cwd_field: Some("cwd".into()),
            disabled_field: Some("disabled".into()),
            enabled_field: None,
            custom_fields: Vec::new(),
        }
    }
}

pub struct ResolvedVars {
    pub env: Map<String, Value>,
    pub headers: Map<String, Value>,
    pub warnings: Vec<String>,
    /// Values read from the OS credential store. Anything equal to one of these
    /// strings is masked before JSON is shown in the UI or written to a log.
    pub sensitive: Vec<String>,
}

/// Build the JSON entry for a harness. `existing` is merged into so that
/// harness-specific extras (autoApprove, disabledTools, oauth, …) survive.
///
/// A field name that is missing or blank means "this harness has no such
/// field": it is never turned into an empty JSON key. The two shape-defining
/// names (`command` / `url`) are required, so a half-configured generic
/// profile fails loudly instead of writing a broken entry.
pub fn build_entry(
    encoding: &EntryEncoding,
    definition: &McpServer,
    existing: Option<&Value>,
    resolved: &ResolvedVars,
) -> AppResult<Value> {
    let mut entry = existing
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();

    let is_stdio = definition.transport == TransportType::Stdio;

    fn optional(name: &Option<String>) -> Option<String> {
        name.as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    let type_field = optional(&encoding.type_field);
    let args_field = optional(&encoding.args_field);
    let env_field = optional(&encoding.env_field);
    let headers_field = optional(&encoding.headers_field);
    let cwd_field = optional(&encoding.cwd_field);
    let disabled_field = optional(&encoding.disabled_field);
    let enabled_field = optional(&encoding.enabled_field);

    // Discriminator field.
    if let Some(field) = &type_field {
        let value = match definition.transport {
            TransportType::Stdio => encoding.stdio_type_value.clone(),
            TransportType::StreamableHttp => encoding.remote_type_value.clone(),
            TransportType::Sse => encoding.sse_type_value.clone().or_else(|| encoding.remote_type_value.clone()),
        };
        match value {
            Some(value) => {
                entry.insert(field.clone(), Value::String(value));
            }
            None => {
                entry.remove(field);
            }
        }
    }

    if is_stdio {
        let command_field = encoding.command_field.trim();
        if command_field.is_empty() {
            return Err(AppError::validation(format!(
                "{} cannot be written: the harness mapping does not name a command field.",
                definition.name
            ))
            .with_remediation("Set the command field (for example `command`) in the integration mapping."));
        }
        let command = definition.command.clone().unwrap_or_default();
        if command.trim().is_empty() {
            return Err(AppError::validation(format!(
                "{} is a stdio server without a command",
                definition.name
            )));
        }
        if encoding.command_as_array {
            let mut parts = vec![Value::String(command)];
            parts.extend(definition.args.iter().cloned().map(Value::String));
            entry.insert(command_field.to_string(), Value::Array(parts));
            if let Some(args_field) = &args_field {
                entry.remove(args_field);
            }
        } else {
            entry.insert(command_field.to_string(), Value::String(command));
            if let Some(args_field) = &args_field {
                if definition.args.is_empty() {
                    entry.remove(args_field);
                } else {
                    entry.insert(
                        args_field.clone(),
                        Value::Array(definition.args.iter().cloned().map(Value::String).collect()),
                    );
                }
            }
        }
        if !encoding.url_field.trim().is_empty() {
            entry.remove(encoding.url_field.trim());
        }
        entry.remove("serverUrl");
        if let Some(headers_field) = &headers_field {
            if resolved.headers.is_empty() {
                entry.remove(headers_field);
            } else {
                entry.insert(headers_field.clone(), Value::Object(resolved.headers.clone()));
            }
        }
        if let Some(env_field) = &env_field {
            if resolved.env.is_empty() {
                entry.remove(env_field);
            } else {
                entry.insert(env_field.clone(), Value::Object(resolved.env.clone()));
            }
        }
        if let Some(cwd_field) = &cwd_field {
            match definition.cwd.as_ref().filter(|value| !value.trim().is_empty()) {
                Some(cwd) => {
                    entry.insert(cwd_field.clone(), Value::String(cwd.clone()));
                }
                None => {
                    entry.remove(cwd_field);
                }
            }
        }
    } else {
        let url_field = encoding.url_field.trim();
        if url_field.is_empty() {
            return Err(AppError::validation(format!(
                "{} cannot be written: the harness mapping does not name a URL field.",
                definition.name
            ))
            .with_remediation("Set the URL field (for example `url` or `serverUrl`) in the integration mapping."));
        }
        let url = definition.url.clone().unwrap_or_default();
        if url.trim().is_empty() {
            return Err(AppError::validation(format!(
                "{} is a remote server without a URL",
                definition.name
            )));
        }
        entry.insert(url_field.to_string(), Value::String(url));
        entry.remove(encoding.command_field.trim());
        if let Some(args_field) = &args_field {
            entry.remove(args_field);
        }
        if let Some(env_field) = &env_field {
            entry.remove(env_field);
        }
        if let Some(cwd_field) = &cwd_field {
            entry.remove(cwd_field);
        }
        if let Some(headers_field) = &headers_field {
            if resolved.headers.is_empty() {
                entry.remove(headers_field);
            } else {
                entry.insert(headers_field.clone(), Value::Object(resolved.headers.clone()));
            }
        }
        // OpenCode uses `url`; Antigravity uses `serverUrl`. Never keep both.
        if url_field == "url" {
            entry.remove("serverUrl");
        } else {
            entry.remove("url");
        }
    }

    if let Some(disabled_field) = &disabled_field {
        entry.insert(disabled_field.clone(), Value::Bool(!definition.enabled));
    }
    if let Some(enabled_field) = &enabled_field {
        entry.insert(enabled_field.clone(), Value::Bool(definition.enabled));
    }
    // Never keep a contradictory pair when an existing entry used the other
    // convention: OpenCode only understands `enabled`, everyone else `disabled`.
    if enabled_field.is_some() {
        entry.remove("disabled");
    } else if disabled_field.is_some() {
        entry.remove("enabled");
    }

    for (key, value) in &encoding.custom_fields {
        entry.insert(key.clone(), value.clone());
    }

    Ok(Value::Object(entry))
}

/// Merge the `enabled` flag for harnesses that use a different convention.
pub fn set_enabled_flag(entry: &mut Value, field: &str, enabled: bool) {
    if let Some(map) = entry.as_object_mut() {
        map.insert(field.to_string(), Value::Bool(!enabled));
    }
}

/// Summary used by the config inspector.
pub fn describe_entry(entry: &Value) -> (String, bool) {
    let object = entry.as_object().cloned().unwrap_or_default();
    let enabled = !object.get("disabled").and_then(Value::as_bool).unwrap_or(false)
        && object.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    let summary = if let Some(url) = object
        .get("url")
        .or_else(|| object.get("serverUrl"))
        .and_then(Value::as_str)
    {
        let transport = object
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("remote");
        format!("{transport} → {url}")
    } else if let Some(command) = object.get("command") {
        match command {
            Value::Array(parts) => {
                let rendered: Vec<String> = parts
                    .iter()
                    .filter_map(|part| part.as_str().map(|value| value.to_string()))
                    .collect();
                format!("stdio → {}", rendered.join(" "))
            }
            Value::String(text) => {
                let args = object
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str())
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                format!("stdio → {text} {args}").trim().to_string()
            }
            _ => "unknown entry".into(),
        }
    } else {
        "no command or URL".into()
    };
    (summary, enabled)
}

/// Convenience constructor used by tests and adapters.
pub fn object(entries: &[(&str, Value)]) -> Value {
    let mut map = Map::new();
    for (key, value) in entries {
        map.insert((*key).to_string(), value.clone());
    }
    Value::Object(map)
}

pub fn env_object(values: &[(String, String)]) -> Value {
    let mut map = Map::new();
    for (key, value) in values {
        map.insert(key.clone(), Value::String(value.clone()));
    }
    Value::Object(map)
}

pub fn sample_entry() -> Value {
    json!({
        "command": "npx",
        "args": ["-y", "@modelcontextprotocol/server-everything"]
    })
}

// ---------------------------------------------------------------------------
// Generic adapter
// ---------------------------------------------------------------------------

/// A user-configured JSON integration. Nothing is written until the profile's
/// schema is either marked verified or confirmed in the preview dialog.
pub struct GenericAdapter {
    profile: crate::models::GenericProfile,
    policy: crate::security::permissions::PathPolicy,
}

impl GenericAdapter {
    pub fn new(profile: crate::models::GenericProfile) -> Self {
        let mut policy = crate::security::permissions::PathPolicy::new();
        policy.allow_file(&profile.config_path);
        Self { profile, policy }
    }

    pub fn profile(&self) -> &crate::models::GenericProfile {
        &self.profile
    }

    fn mapping_encoding(&self) -> EntryEncoding {
        let mapping = &self.profile.transport_mapping;
        let type_field = match mapping.stdio_uses.as_str() {
            "none" | "" => None,
            other => Some(other.to_string()),
        };
        EntryEncoding {
            type_field,
            stdio_type_value: if mapping.stdio_value.is_empty() {
                None
            } else {
                Some(mapping.stdio_value.clone())
            },
            remote_type_value: if mapping.streamable_http_value.is_empty() {
                None
            } else {
                Some(mapping.streamable_http_value.clone())
            },
            sse_type_value: if mapping.sse_value.is_empty() {
                None
            } else {
                Some(mapping.sse_value.clone())
            },
            url_field: mapping.url_field.clone(),
            command_field: mapping.command_field.clone(),
            command_as_array: false,
            args_field: Some(mapping.args_field.clone()),
            env_field: Some(mapping.env_field.clone()),
            headers_field: Some(mapping.headers_field.clone()),
            cwd_field: Some(mapping.cwd_field.clone()),
            disabled_field: Some(mapping.disabled_field.clone()),
            enabled_field: None,
            custom_fields: self
                .profile
                .custom_fields
                .iter()
                .map(|field| (field.key.clone(), Value::String(field.value.clone())))
                .collect(),
        }
    }
}

impl super::HarnessAdapter for GenericAdapter {
    fn id(&self) -> String {
        self.profile.id.clone()
    }

    fn display_name(&self) -> String {
        format!("{} (generic JSON)", self.profile.label)
    }

    fn adapter_status(&self) -> crate::models::AdapterStatus {
        if self.profile.verified {
            crate::models::AdapterStatus::Verified
        } else {
            crate::models::AdapterStatus::PendingVerification
        }
    }

    fn capabilities(&self) -> crate::models::HarnessCapabilities {
        let mapping = &self.profile.transport_mapping;
        crate::models::HarnessCapabilities {
            stdio: true,
            streamable_http: true,
            sse: !mapping.sse_value.is_empty(),
            headers: !mapping.headers_field.is_empty(),
            env: !mapping.env_field.is_empty(),
            cwd: !mapping.cwd_field.is_empty(),
            enabled_flag: !mapping.disabled_field.is_empty(),
            auto_approve: false,
            extra_entry_fields: self
                .profile
                .custom_fields
                .iter()
                .map(|field| crate::models::ExtraEntryField {
                    key: field.key.clone(),
                    description: "Custom field configured by the user.".into(),
                })
                .collect(),
            notes: vec![
                "Schema mapping was configured manually; verify it against the target application.".into(),
            ],
        }
    }

    fn reload(&self) -> crate::models::ReloadRequirements {
        crate::models::ReloadRequirements {
            reload_required: true,
            description: "Restart or reload the target application so it re-reads its configuration file.".into(),
        }
    }

    fn notes(&self) -> Vec<String> {
        let mut notes = vec![format!(
            "Entry format: transport field `{}` = `{}` (stdio) / `{}` (remote), URL field `{}`.",
            self.profile.transport_mapping.stdio_uses,
            self.profile.transport_mapping.stdio_value,
            self.profile.transport_mapping.streamable_http_value,
            self.profile.transport_mapping.url_field
        )];
        if !self.profile.verified {
            notes.push(
                "This profile is not marked as verified: the preview dialog requires explicit confirmation before writing."
                    .into(),
            );
        }
        notes
    }

    fn config_format_label(&self) -> String {
        format!("Generic JSON (`{}` object)", self.profile.mcp_path)
    }

    fn mcp_path(&self) -> Vec<String> {
        path_segments(&self.profile.mcp_path)
    }

    fn encoding(&self) -> EntryEncoding {
        self.mapping_encoding()
    }

    fn detect_installation(&self) -> super::DetectionResult {
        let path = std::path::PathBuf::from(&self.profile.config_path);
        super::DetectionResult {
            detected: path.exists(),
            confidence: if path.exists() { "verified".into() } else { "notDetected".into() },
            version: None,
            notes: if path.exists() {
                vec![]
            } else {
                vec![format!("No file at {}", path.display())]
            },
        }
    }

    fn locate_config(&self, _hints: &super::HarnessHints) -> Option<std::path::PathBuf> {
        Some(std::path::PathBuf::from(&self.profile.config_path))
    }

    fn candidate_configs(&self, _hints: &super::HarnessHints) -> Vec<crate::models::ConfigVariant> {
        let path = std::path::PathBuf::from(&self.profile.config_path);
        vec![crate::models::ConfigVariant {
            path: path.to_string_lossy().to_string(),
            label: "Configured file".into(),
            exists: path.exists(),
            scope: "global".into(),
        }]
    }

    fn write_config_safely(
        &self,
        path: &std::path::Path,
        content: &str,
        expected_hash: Option<&str>,
    ) -> crate::errors::AppResult<crate::services::safe_io::WriteOutcome> {
        self.policy.check_write_target(path)?;
        crate::services::safe_io::write_atomic(path, content, expected_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{McpServer, TransportType};
    use std::collections::BTreeMap;

    fn server(name: &str, transport: TransportType) -> McpServer {
        McpServer {
            id: format!("id-{name}"),
            name: name.into(),
            description: String::new(),
            transport,
            command: Some("npx".into()),
            args: vec!["-y".into(), "@modelcontextprotocol/server-everything".into()],
            env: BTreeMap::new(),
            headers: BTreeMap::new(),
            url: Some("https://example.test/mcp".into()),
            cwd: None,
            enabled: true,
            targets: vec![],
            tags: vec![],
            notes: String::new(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            last_sync_at: None,
        }
    }

    #[test]
    fn strips_line_and_block_comments() {
        let input = r#"{
            // comment
            "a": 1, /* block */
            "b": "// not a comment"
        }"#;
        let stripped = strip_jsonc(input);
        let value: Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(value["a"], 1);
        assert_eq!(value["b"], "// not a comment");
    }

    #[test]
    fn removes_trailing_commas() {
        let stripped = strip_jsonc("{\"a\": [1, 2,],}");
        let value: Value = serde_json::from_str(&stripped).unwrap();
        assert_eq!(value["a"], json!([1, 2]));
    }

    #[test]
    fn preserves_unknown_fields_when_updating_entry() {
        let encoding = EntryEncoding::claude_style();
        let existing = json!({ "autoApprove": ["read_file"], "custom": {"keep": true}, "command": "old" });
        let resolved = ResolvedVars { env: Map::new(), headers: Map::new(), warnings: vec![], sensitive: vec![] };
        let entry = build_entry(&encoding, &server("demo", TransportType::Stdio), Some(&existing), &resolved)
            .unwrap();
        assert_eq!(entry["autoApprove"], json!(["read_file"]));
        assert_eq!(entry["custom"]["keep"], true);
        assert_eq!(entry["command"], "npx");
    }

    #[test]
    fn remote_entries_drop_stdio_leftovers() {
        let encoding = EntryEncoding::claude_style();
        let existing = json!({ "command": "old", "args": ["x"], "env": {"A": "b"} });
        let resolved = ResolvedVars { env: Map::new(), headers: Map::new(), warnings: vec![], sensitive: vec![] };
        let entry =
            build_entry(&encoding, &server("remote", TransportType::StreamableHttp), Some(&existing), &resolved)
                .unwrap();
        assert_eq!(entry["type"], "streamableHttp");
        assert_eq!(entry["url"], "https://example.test/mcp");
        assert!(entry.get("command").is_none());
        assert!(entry.get("args").is_none());
        assert!(entry.get("env").is_none());
    }

    #[test]
    fn opencode_style_command_is_an_array() {
        let mut encoding = EntryEncoding::claude_style();
        encoding.type_field = Some("type".into());
        encoding.stdio_type_value = Some("local".into());
        encoding.remote_type_value = Some("remote".into());
        encoding.command_as_array = true;
        encoding.args_field = None;
        encoding.env_field = Some("environment".into());
        let resolved = ResolvedVars { env: Map::new(), headers: Map::new(), warnings: vec![], sensitive: vec![] };
        let entry = build_entry(&encoding, &server("demo", TransportType::Stdio), None, &resolved).unwrap();
        assert_eq!(entry["command"], json!(["npx", "-y", "@modelcontextprotocol/server-everything"]));
        assert_eq!(entry["type"], "local");
    }

    #[test]
    fn path_helpers_create_and_prune() {
        let mut root = json!({});
        set_entry(&mut root, &path_segments("mcp"), "demo", json!({"type": "local"})).unwrap();
        assert_eq!(entry_names(&root, &path_segments("mcp")), vec!["demo".to_string()]);
        assert!(remove_entry(&mut root, &path_segments("mcp"), "demo").unwrap());
        assert!(root.get("mcp").is_none(), "empty container should be pruned");
    }
}
