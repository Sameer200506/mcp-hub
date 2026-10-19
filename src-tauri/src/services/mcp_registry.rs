//! Central MCP registry: CRUD, secret handling, duplicate detection,
//! import/export.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Map;

use crate::adapters::generic_json::ResolvedVars;
use crate::adapters::HarnessAdapter;
use crate::database::{now_rfc3339, Db};
use crate::errors::{AppError, AppResult};
use crate::models::{
    ImportResult, McpServer, McpServerInput, RegistryExportRequest, RegistryFile, RegistryImportRequest,
    TransportType, VarKind, VarValue,
};
use crate::security::secrets::SecretStore;
use crate::services::config_validation::{looks_like_secret, validate_input};
use crate::services::safe_io::{reject_symlink, write_atomic};

fn input_to_server(id: &str, input: &McpServerInput, created_at: &str) -> McpServer {
    McpServer {
        id: id.to_string(),
        name: input.name.trim().to_string(),
        description: input.description.trim().to_string(),
        transport: input.transport,
        command: input
            .command
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        args: input.args.iter().map(|value| value.trim().to_string()).filter(|value| !value.is_empty()).collect(),
        env: input.env.clone(),
        headers: input.headers.clone(),
        url: input
            .url
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        cwd: input
            .cwd
            .as_ref()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
        enabled: input.enabled,
        targets: input.targets.clone(),
        tags: input.tags.clone(),
        notes: input.notes.clone(),
        created_at: created_at.to_string(),
        updated_at: created_at.to_string(),
        last_sync_at: None,
    }
}

fn ensure_valid(input: &McpServerInput, adapters: &[Box<dyn HarnessAdapter>]) -> AppResult<()> {
    let report = validate_input(input, adapters);
    if report.valid {
        return Ok(());
    }
    let details = serde_json::to_value(&report).unwrap_or(serde_json::Value::Null);
    Err(
        AppError::validation("The MCP definition failed validation.")
            .with_details(details)
            .with_remediation("Fix the reported errors and try again."),
    )
}

/// Move secret values into the OS credential store, returning sanitized maps
/// that are safe to persist in SQLite.
fn sanitize_vars(
    db: &Db,
    secrets: &SecretStore,
    mcp_id: &str,
    slot: &str,
    vars: &BTreeMap<String, VarValue>,
) -> AppResult<BTreeMap<String, VarValue>> {
    let mut sanitized = BTreeMap::new();
    for (key, value) in vars {
        match value.kind {
            VarKind::Secret => {
                let existing = secrets.get(mcp_id, slot, key)?;
                if !value.value.trim().is_empty() {
                    if !secrets.is_available() {
                        return Err(AppError::secret_store(
                            "The OS credential store is unavailable, so secret values cannot be stored.",
                        ));
                    }
                    secrets.set(mcp_id, slot, key, value.value.trim())?;
                    db.register_secret_ref(mcp_id, slot, key)?;
                } else if existing.is_none() {
                    return Err(AppError::validation(format!(
                        "Secret '{key}' has no stored value yet. Enter the value or switch the row to Literal."
                    )));
                } else {
                    db.register_secret_ref(mcp_id, slot, key)?;
                }
                sanitized.insert(key.clone(), VarValue { value: String::new(), kind: VarKind::Secret });
            }
            _ => {
                // Value is no longer a secret: drop any stored credential.
                if secrets.get(mcp_id, slot, key)?.is_some() {
                    secrets.delete(mcp_id, slot, key)?;
                    db.remove_secret_ref(mcp_id, slot, key)?;
                }
                sanitized.insert(key.clone(), value.clone());
            }
        }
    }
    // Remove stored secrets whose keys disappeared from the definition.
    for (existing_slot, existing_key) in db.list_secret_refs(mcp_id)? {
        if existing_slot == slot && !sanitized.contains_key(&existing_key) {
            secrets.delete(mcp_id, &existing_slot, &existing_key)?;
            db.remove_secret_ref(mcp_id, &existing_slot, &existing_key)?;
        }
    }
    Ok(sanitized)
}

pub fn create_server(
    db: &Db,
    secrets: &SecretStore,
    adapters: &[Box<dyn HarnessAdapter>],
    input: &McpServerInput,
) -> AppResult<McpServer> {
    ensure_valid(input, adapters)?;
    if db.find_server_by_name(&input.name)?.is_some() {
        return Err(AppError::conflict(format!(
            "A server named '{}' already exists in the registry.",
            input.name
        )));
    }
    let id = input
        .id
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let now = now_rfc3339();
    let mut server = input_to_server(&id, input, &now);
    server.env = sanitize_vars(db, secrets, &id, "env", &server.env)?;
    server.headers = sanitize_vars(db, secrets, &id, "headers", &server.headers)?;
    db.insert_server(&server)?;
    Ok(server)
}

pub fn update_server(
    db: &Db,
    secrets: &SecretStore,
    adapters: &[Box<dyn HarnessAdapter>],
    id: &str,
    input: &McpServerInput,
) -> AppResult<McpServer> {
    ensure_valid(input, adapters)?;
    let mut server = db.get_server(id)?;
    if let Some(other) = db.find_server_by_name(&input.name)? {
        if other.id != id {
            return Err(AppError::conflict(format!(
                "Another registry entry already uses the name '{}'.",
                input.name
            )));
        }
    }
    let created_at = server.created_at.clone();
    let mut updated = input_to_server(id, input, &created_at);
    updated.last_sync_at = server.last_sync_at.clone();
    updated.env = sanitize_vars(db, secrets, id, "env", &updated.env)?;
    updated.headers = sanitize_vars(db, secrets, id, "headers", &updated.headers)?;
    updated.updated_at = now_rfc3339();
    db.update_server(&updated)?;
    server = updated;
    Ok(server)
}

pub fn duplicate_server(
    db: &Db,
    secrets: &SecretStore,
    id: &str,
    new_name: Option<String>,
) -> AppResult<McpServer> {
    let original = db.get_server(id)?;
    let name = match new_name.filter(|value| !value.trim().is_empty()) {
        Some(name) => name,
        None => {
            let mut candidate = format!("{}-copy", original.name);
            let mut counter = 2;
            while db.find_server_by_name(&candidate)?.is_some() {
                candidate = format!("{}-copy-{counter}", original.name);
                counter += 1;
            }
            candidate
        }
    };
    let input = McpServerInput {
        id: None,
        name,
        description: original.description.clone(),
        transport: original.transport,
        command: original.command.clone(),
        args: original.args.clone(),
        env: original.env.clone(),
        headers: original.headers.clone(),
        url: original.url.clone(),
        cwd: original.cwd.clone(),
        enabled: original.enabled,
        targets: original.targets.clone(),
        tags: original.tags.clone(),
        notes: original.notes.clone(),
    };
    ensure_valid(&input, &[])?;
    if db.find_server_by_name(&input.name)?.is_some() {
        return Err(AppError::conflict(format!(
            "A server named '{}' already exists.",
            input.name
        )));
    }
    let now = now_rfc3339();
    let new_id = uuid::Uuid::new_v4().to_string();
    let mut server = input_to_server(&new_id, &input, &now);
    // Secret slots are copied by value reference: each duplicate owns its own
    // credential entry, so clone the stored values.
    for (slot, vars) in [("env", &original.env), ("headers", &original.headers)] {
        let mut cloned = BTreeMap::new();
        for (key, value) in vars.iter() {
            if value.kind == VarKind::Secret {
                if let Some(secret) = secrets.get(&original.id, slot, key)? {
                    secrets.set(&new_id, slot, key, &secret)?;
                    db.register_secret_ref(&new_id, slot, key)?;
                }
                cloned.insert(key.clone(), VarValue { value: String::new(), kind: VarKind::Secret });
            } else {
                cloned.insert(key.clone(), value.clone());
            }
        }
        if slot == "env" {
            server.env = cloned;
        } else {
            server.headers = cloned;
        }
    }
    db.insert_server(&server)?;
    Ok(server)
}

pub fn delete_server(db: &Db, secrets: &SecretStore, id: &str) -> AppResult<()> {
    for (slot, key) in db.list_secret_refs(id)? {
        let _ = secrets.delete(id, &slot, &key);
    }
    db.delete_server(id)
}

/// Resolve variables for writing into a harness configuration.
pub fn resolve_vars(
    db: &Db,
    secrets: &SecretStore,
    server: &McpServer,
    slot: &str,
) -> AppResult<ResolvedVars> {
    let source = if slot == "env" { &server.env } else { &server.headers };
    let mut resolved = ResolvedVars {
        env: Map::new(),
        headers: Map::new(),
        warnings: Vec::new(),
        sensitive: Vec::new(),
    };
    for (key, value) in source {
        let rendered = match value.kind {
            VarKind::Secret => match secrets.get(&server.id, slot, key)? {
                Some(secret) => {
                    resolved
                        .warnings
                        .push(format!("Credential '{key}' is written into the harness configuration file."));
                    if !secret.is_empty() {
                        resolved.sensitive.push(secret.clone());
                    }
                    secret
                }
                None => {
                    return Err(AppError::validation(format!(
                        "Stored credential '{key}' is missing from the OS credential store."
                    ))
                    .with_remediation("Re-enter the value in the server editor."));
                }
            },
            _ => value.value.clone(),
        };
        if slot == "env" {
            resolved.env.insert(key.clone(), serde_json::Value::String(rendered));
        } else {
            resolved.headers.insert(key.clone(), serde_json::Value::String(rendered));
        }
    }
    // A credential referenced through {env:NAME} is fine; a literal secret is
    // surfaced as a warning so previews stay explicit about what is on disk.
    for (key, value) in source {
        if value.kind == VarKind::Literal && looks_like_secret(&value.value) {
            resolved.warnings.push(format!(
                "'{key}' looks like a credential and will be written literally to the harness configuration."
            ));
        }
    }
    let _ = db;
    Ok(resolved)
}

/// A view of a definition without any secret material.
pub fn redacted(server: &McpServer) -> McpServer {
    let mut clone = server.clone();
    for vars in [&mut clone.env, &mut clone.headers] {
        for value in vars.values_mut() {
            if value.kind == VarKind::Secret {
                value.value = String::new();
            } else if looks_like_secret(&value.value) {
                value.value = crate::models::REDACTED.to_string();
                value.kind = VarKind::Literal;
            }
        }
    }
    clone
}

pub fn export_registry(db: &Db, request: &RegistryExportRequest) -> AppResult<String> {
    let path = Path::new(&request.path);
    if !path.is_absolute() {
        return Err(AppError::validation("The export path must be absolute."));
    }
    reject_symlink(path)?;
    let servers = db.list_servers()?;
    let mut warnings = Vec::new();
    let mut exported = Vec::new();
    for server in servers {
        if !request.include_disabled && !server.enabled {
            continue;
        }
        let mut clone = server.clone();
        for (slot, vars) in [("env", &mut clone.env), ("headers", &mut clone.headers)] {
            for (key, value) in vars.iter_mut() {
                match value.kind {
                    VarKind::Secret => {
                        // Secret values never leave the machine.
                        value.value = String::new();
                    }
                    _ => {
                        if !request.include_literal_env_values && looks_like_secret(&value.value) {
                            warnings.push(format!(
                                "Literal value for '{}' in '{}' was redacted; mark it as a secret or reference an environment variable.",
                                key, server.name
                            ));
                            value.value = String::new();
                        }
                    }
                }
                let _ = slot;
            }
        }
        exported.push(clone);
    }

    let document = RegistryFile {
        schema_version: 1,
        application: format!("Universal MCP Manager {}", env!("CARGO_PKG_VERSION")),
        exported_at: now_rfc3339(),
        includes_secrets: false,
        includes_literal_env_values: request.include_literal_env_values,
        servers: exported,
        warnings: warnings.clone(),
    };
    let value = serde_json::to_value(&document)?;
    let content = crate::adapters::generic_json::canonical_json(&value);
    write_atomic(path, &content, None)?;
    let mut summary = format!("Wrote {} server definition(s) to {}", document.servers.len(), path.display());
    if !warnings.is_empty() {
        summary.push_str(&format!(" ({} redaction warning(s))", warnings.len()));
    }
    Ok(summary)
}

pub fn import_registry(db: &Db, secrets: &SecretStore, request: &RegistryImportRequest) -> AppResult<ImportResult> {
    let path = Path::new(&request.path);
    if !path.is_absolute() {
        return Err(AppError::validation("The import path must be absolute."));
    }
    let raw = std::fs::read_to_string(path)?;
    let document: RegistryFile = serde_json::from_str(&raw).map_err(|error| {
        AppError::validation(format!("Not a Universal MCP Manager registry file: {error}"))
    })?;

    let mut result = ImportResult { imported: 0, updated: 0, skipped: 0, warnings: Vec::new() };
    if !document.servers.is_empty() && document.warnings.is_empty() {
        // nothing to do, kept for symmetry with the export structure
    }
    result.warnings.extend(document.warnings.clone());
    if document.includes_secrets {
        result
            .warnings
            .push("The imported file claims to include secrets; values were ignored and must be re-entered.".into());
    }

    if request.mode == "replace" {
        for existing in db.list_servers()? {
            delete_server(db, secrets, &existing.id)?;
            result.warnings.push(format!("Replaced existing entry '{}'.", existing.name));
        }
    }

    for mut server in document.servers {
        if server.name.trim().is_empty() {
            result.skipped += 1;
            continue;
        }
        let sanitized = sanitize_vars(db, secrets, &server.id, "env", &server.env)
            .and_then(|env| sanitize_vars(db, secrets, &server.id, "headers", &server.headers).map(|headers| (env, headers)));
        match sanitized {
            Ok((env, headers)) => {
                server.env = env;
                server.headers = headers;
            }
            Err(error) => {
                result.warnings.push(format!(
                    "'{}' imported without secret values: {}",
                    server.name, error.message
                ));
                server.env = server
                    .env
                    .into_iter()
                    .map(|(key, mut value)| {
                        if value.kind == VarKind::Secret {
                            value.value = String::new();
                        }
                        (key, value)
                    })
                    .collect();
            }
        }
        match db.find_server_by_name(&server.name)? {
            Some(existing) => {
                server.id = existing.id.clone();
                server.created_at = existing.created_at.clone();
                server.updated_at = now_rfc3339();
                server.last_sync_at = existing.last_sync_at.clone();
                db.update_server(&server)?;
                result.updated += 1;
            }
            None => {
                if server.id.trim().is_empty() || db.get_server(&server.id).is_ok() {
                    server.id = uuid::Uuid::new_v4().to_string();
                }
                server.created_at = now_rfc3339();
                server.updated_at = server.created_at.clone();
                server.transport = match server.transport {
                    TransportType::Sse => TransportType::Sse,
                    TransportType::StreamableHttp => TransportType::StreamableHttp,
                    TransportType::Stdio => TransportType::Stdio,
                };
                db.insert_server(&server)?;
                result.imported += 1;
            }
        }
    }
    Ok(result)
}
