//! Validation for registry definitions and per-harness compatibility.
//!
//! This module only reasons about the *configuration* — it never starts a
//! process or opens a socket. Connection probes live behind an explicit user
//! action in `commands::test_mcp_connection`.

use std::collections::HashSet;
use std::path::Path;

use serde_json::Map;

use crate::adapters::generic_json::ResolvedVars;
use crate::adapters::HarnessAdapter;
use crate::errors::AppResult;
use crate::models::{McpServer, McpServerInput, TransportType, ValidationIssue, ValidationReport, VarKind};

pub fn validate_input(input: &McpServerInput, adapters: &[Box<dyn HarnessAdapter>]) -> ValidationReport {
    let mut issues = Vec::new();
    let name = input.name.trim();

    if name.is_empty() {
        issues.push(ValidationIssue::error("name", "A server name is required."));
    } else {
        if name.len() > 100 {
            issues.push(ValidationIssue::error("name", "Server names must be 100 characters or fewer."));
        }
        if !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
        {
            issues.push(
                ValidationIssue::error(
                    "name",
                    "Server names may only contain letters, digits, dot, dash and underscore because every harness uses the name as a JSON key.",
                )
                .with_remediation("Example: filesystem, github-mcp, my_server"),
            );
        }
    }

    if input.description.len() > 500 {
        issues.push(ValidationIssue::warning("description", "Description is very long; harness configs never see it, but keep it concise."));
    }

    match input.transport {
        TransportType::Stdio => {
            match input.command.as_ref().map(|value| value.trim()).filter(|value| !value.is_empty()) {
                Some(command) => {
                    if command.contains('\n') {
                        issues.push(ValidationIssue::error("command", "The command must be a single executable path or name."));
                    }
                    if command.contains('"') || command.contains('\'') {
                        issues.push(ValidationIssue::warning(
                            "command",
                            "Quotes are written literally; MCP servers are launched without a shell, so quoting is unnecessary.",
                        ));
                    }
                }
                None => issues.push(ValidationIssue::error("command", "A command is required for stdio servers.")),
            }
            for argument in &input.args {
                if argument.contains('\n') {
                    issues.push(ValidationIssue::error("args", "Arguments must not contain line breaks."));
                    break;
                }
            }
            if let Some(cwd) = input.cwd.as_ref().filter(|value| !value.trim().is_empty()) {
                if argument_is_relative(cwd) {
                    issues.push(ValidationIssue::warning(
                        "cwd",
                        "A relative working directory resolves differently per harness; prefer an absolute path.",
                    ));
                }
            }
        }
        TransportType::StreamableHttp | TransportType::Sse => {
            match input.url.as_ref().map(|value| value.trim()).filter(|value| !value.is_empty()) {
                Some(url) => match url::Url::parse(url) {
                    Ok(parsed) => {
                        if !matches!(parsed.scheme(), "http" | "https") {
                            issues.push(ValidationIssue::error("url", "Only http:// and https:// endpoints are supported."));
                        }
                        if parsed.scheme() == "http" {
                            issues.push(ValidationIssue::warning(
                                "url",
                                "Unencrypted HTTP endpoints expose credentials on the network.",
                            ));
                        }
                        if parsed.host_str().is_none() {
                            issues.push(ValidationIssue::error("url", "The URL must include a host name."));
                        }
                    }
                    Err(error) => issues.push(
                        ValidationIssue::error("url", format!("Not a valid URL: {error}"))
                            .with_remediation("Example: https://mcp.example.com/mcp"),
                    ),
                },
                None => issues.push(ValidationIssue::error("url", "A URL is required for remote servers.")),
            }
        }
    }

    // Environment / header key hygiene.
    let mut env_keys: HashSet<String> = HashSet::new();
    for (key, value) in &input.env {
        validate_var_key("env", key, &mut issues);
        if !env_keys.insert(key.to_lowercase()) {
            issues.push(ValidationIssue::error("env", format!("Duplicate environment key '{key}'.")));
        }
        validate_var_value("env", key, value, &mut issues);
    }
    let mut header_keys: HashSet<String> = HashSet::new();
    for (key, value) in &input.headers {
        if key.trim().is_empty() {
            issues.push(ValidationIssue::error("headers", "Header names must not be empty."));
        }
        if !header_keys.insert(key.to_lowercase()) {
            issues.push(ValidationIssue::error("headers", format!("Duplicate header '{key}'.")));
        }
        validate_var_value("headers", key, value, &mut issues);
    }
    if input.transport == TransportType::Stdio && !input.headers.is_empty() {
        issues.push(ValidationIssue::info(
            "headers",
            "HTTP headers are ignored for stdio servers and will be dropped when writing.",
        ));
    }

    let mut per_harness = Vec::new();
    for adapter in adapters {
        if !input.targets.iter().any(|target| target == &adapter.id()) {
            continue;
        }
        let mut harness_issues = Vec::new();
        let capabilities = adapter.capabilities();
        let supported = match input.transport {
            TransportType::Stdio => capabilities.stdio,
            TransportType::StreamableHttp => capabilities.streamable_http,
            TransportType::Sse => capabilities.sse,
        };
        if !supported {
            harness_issues.push(ValidationIssue::error(
                "transport",
                format!(
                    "{} does not support the '{}' transport.",
                    adapter.display_name(),
                    input.transport.as_str()
                ),
            ));
        }
        if input.cwd.as_ref().filter(|value| !value.trim().is_empty()).is_some() && !capabilities.cwd {
            harness_issues.push(ValidationIssue::warning(
                "cwd",
                format!("{} has no working-directory field; the value will be dropped.", adapter.display_name()),
            ));
        }
        if !input.headers.is_empty() && input.transport != TransportType::Stdio && !capabilities.headers {
            harness_issues.push(ValidationIssue::warning(
                "headers",
                format!("{} cannot send custom HTTP headers.", adapter.display_name()),
            ));
        }
        if input.transport == TransportType::Sse && capabilities.streamable_http && !capabilities.sse {
            harness_issues.push(ValidationIssue::warning(
                "transport",
                format!("{} prefers Streamable HTTP over legacy SSE.", adapter.display_name()),
            ));
        }
        for (key, value) in input.env.iter().chain(input.headers.iter()) {
            if value.kind == VarKind::Literal && looks_like_secret(value.value.as_str()) {
                harness_issues.push(ValidationIssue::warning(
                    &format!("{}:{key}", adapter.id()),
                    "Value looks like a credential. Mark it as a secret (OS credential store) or use an {env:NAME} reference.",
                ));
            }
        }
        per_harness.push(crate::models::HarnessValidation {
            harness_id: adapter.id(),
            harness_name: adapter.display_name(),
            supported,
            issues: harness_issues,
        });
    }

    let has_errors = issues
        .iter()
        .chain(per_harness.iter().flat_map(|item| item.issues.iter()))
        .any(|issue| issue.severity == "error");

    ValidationReport { valid: !has_errors, issues, per_harness }
}

/// Whether a path is relative to the current directory.
fn argument_is_relative(path: &str) -> bool {
    !Path::new(path).is_absolute()
}

fn validate_var_key(field: &str, key: &str, issues: &mut Vec<ValidationIssue>) {    if key.trim().is_empty() {
        issues.push(ValidationIssue::error(field, "Names must not be empty."));
        return;
    }
    let valid = key
        .chars()
        .next()
        .map(|first| first.is_ascii_alphabetic() || first == '_')
        .unwrap_or(false)
        && key.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    if !valid {
        issues.push(ValidationIssue::warning(
            field,
            format!("'{key}' is unusual for an environment variable name (expected [A-Za-z_][A-Za-z0-9_]*)."),
        ));
    }
}

fn validate_var_value(field: &str, key: &str, value: &crate::models::VarValue, issues: &mut Vec<ValidationIssue>) {
    match value.kind {
        VarKind::Secret => {
            if value.value.is_empty() {
                issues.push(ValidationIssue::info(
                    field,
                    format!("'{key}' is stored in the OS credential store (values are never shown again)."),
                ));
            }
        }
        VarKind::EnvRef => {
            let is_reference = (value.value.starts_with("{env:") || value.value.starts_with("{file:"))
                && value.value.ends_with('}');
            if !is_reference {
                issues.push(
                    ValidationIssue::error(
                        field,
                        format!("'{key}' is marked as a reference but does not look like {{env:NAME}} or {{file:path}}."),
                    )
                    .with_remediation("Either switch the row to Literal or use the reference syntax."),
                );
            }
        }
        VarKind::Literal => {}
    }
}

pub fn looks_like_secret(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.starts_with("{env:") || trimmed.starts_with("{file:") {
        return false;
    }
    let lowercase = trimmed.to_lowercase();
    lowercase.starts_with("bearer ")
        || trimmed.starts_with("sk-")
        || trimmed.starts_with("ghp_")
        || trimmed.starts_with("xoxb-")
        || (trimmed.len() >= 32 && trimmed.chars().all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
}

/// Build empty resolved maps (used when validating a definition that has no
/// stored secrets yet).
pub fn empty_resolved() -> ResolvedVars {
    ResolvedVars { env: Map::new(), headers: Map::new(), warnings: Vec::new(), sensitive: Vec::new() }
}

/// Validate a stored definition the same way as an input.
pub fn validate_server(server: &McpServer, adapters: &[Box<dyn HarnessAdapter>]) -> ValidationReport {
    validate_input(
        &McpServerInput {
            id: Some(server.id.clone()),
            name: server.name.clone(),
            description: server.description.clone(),
            transport: server.transport,
            command: server.command.clone(),
            args: server.args.clone(),
            env: server.env.clone(),
            headers: server.headers.clone(),
            url: server.url.clone(),
            cwd: server.cwd.clone(),
            enabled: server.enabled,
            targets: server.targets.clone(),
            tags: server.tags.clone(),
            notes: server.notes.clone(),
        },
        adapters,
    )
}

/// Duplicate detection across the registry.
#[derive(Debug, Clone)]
pub struct RegistryConflict {
    pub kind: String,
    pub message: String,
    pub servers: Vec<String>,
}

pub fn detect_registry_conflicts(servers: &[McpServer]) -> Vec<RegistryConflict> {
    let mut conflicts = Vec::new();
    let mut signatures: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    let mut names: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();

    for server in servers {
        signatures
            .entry(server.signature())
            .or_default()
            .push(server.name.clone());
        names
            .entry(server.name.to_lowercase())
            .or_default()
            .push(server.name.clone());
    }

    for (_, group) in names.into_iter().filter(|(_, group)| group.len() > 1) {
        conflicts.push(RegistryConflict {
            kind: "duplicateRegistryKey".into(),
            message: format!("Multiple registry entries would generate the same harness key: {}", group.join(", ")),
            servers: group,
        });
    }
    for (signature, group) in signatures.into_iter().filter(|(_, group)| group.len() > 1) {
        conflicts.push(RegistryConflict {
            kind: "duplicateEndpoint".into(),
            message: format!(
                "{} registry entries share the same endpoint or command ({}). Writing all of them is allowed but may be unintentional.",
                group.len(),
                signature
            ),
            servers: group,
        });
    }
    conflicts
}

pub fn validate_definition_against_harness(
    server: &McpServer,
    adapter: &dyn HarnessAdapter,
) -> AppResult<Vec<ValidationIssue>> {
    let mut issues = Vec::new();
    if !adapter.supports_transport(server.transport) {
        issues.push(ValidationIssue::error(
            "transport",
            format!("{} does not support {}.", adapter.display_name(), server.transport.as_str()),
        ));
    }
    let capabilities = adapter.capabilities();
    if server.cwd.is_some() && !capabilities.cwd {
        issues.push(ValidationIssue::warning("cwd", "Working directory will be dropped by this adapter."));
    }
    if !server.headers.is_empty() && server.transport != TransportType::Stdio && !capabilities.headers {
        issues.push(ValidationIssue::warning("headers", "Custom headers are not supported by this adapter."));
    }
    Ok(issues)
}
