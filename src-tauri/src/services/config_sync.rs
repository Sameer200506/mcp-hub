//! Synchronization engine: preview, apply, verification and reporting.
//!
//! Failures are isolated per harness: a broken Cline config never prevents the
//! OpenCode write. Nothing is written unless the preview is confirmed, and a
//! file that is identical after applying the proposal is never rewritten
//! (idempotency).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::adapters::generic_json::{canonical_json, ResolvedVars};
use crate::adapters::{ConfigDocument, HarnessAdapter};
use crate::database::{now_rfc3339, Db};
use crate::errors::{AppResult, StructuredError};
use crate::models::{
    AppliedChange, AppSettings, DiffAction, DiffEntry, HarnessSyncResult, McpServer, PreviewReport,
    RemovalRequest, SyncConflict, SyncPreview, SyncReport, TransportType,
};
use crate::security::secrets::SecretStore;
use crate::services::config_backup::BackupManager;
use crate::services::config_validation::{detect_registry_conflicts, validate_definition_against_harness};
use crate::services::diff::{compact_diff, diff_lines};
use crate::services::harness_detection::hints_for;
use crate::services::mcp_registry::resolve_vars;

/// Resolve the environment *and* header variables of one definition.
///
/// `resolve_vars` reads a single slot, so a definition using both
/// `environment` and `headers` needs the two results merged. The sensitive
/// values of both slots must be merged as well, otherwise a credential stored
/// for a header would not be masked in previews.
fn resolve_entry_vars(ctx: &SyncContext<'_>, server: &McpServer) -> AppResult<ResolvedVars> {
    let mut resolved = resolve_vars(ctx.db, ctx.secrets, server, "env")?;
    let headers = resolve_vars(ctx.db, ctx.secrets, server, "headers")?;
    resolved.headers = headers.headers;
    resolved.warnings.extend(headers.warnings);
    resolved.sensitive.extend(headers.sensitive);
    Ok(resolved)
}

pub struct SyncContext<'a> {
    pub db: &'a Db,
    pub secrets: &'a SecretStore,
    pub backups: &'a BackupManager,
    pub adapters: &'a [Box<dyn HarnessAdapter>],
    pub settings: &'a AppSettings,
}

#[derive(Debug)]
struct HarnessPlan {
    harness_id: String,
    harness_name: String,
    path: Option<PathBuf>,
    document: ConfigDocument,
    upserts: BTreeMap<String, Value>,
    removals: Vec<String>,
    entries: Vec<DiffEntry>,
    conflicts: Vec<SyncConflict>,
    warnings: Vec<String>,
    status: String,
    /// Current file content, masked for display.
    current_json: Option<String>,
    /// Proposal, masked for display (never written).
    proposed_json: Option<String>,
    /// The exact bytes that are written to disk, including credential values.
    content_json: Option<String>,
    diff: Vec<crate::models::DiffLine>,
}

impl HarnessPlan {
    fn empty(adapter: &dyn HarnessAdapter, status: &str) -> Self {
        let placeholder = PathBuf::from(format!("<{}>", adapter.id()));
        Self {
            harness_id: adapter.id(),
            harness_name: adapter.display_name(),
            path: None,
            document: ConfigDocument::empty_for(&adapter.id(), &placeholder),
            upserts: BTreeMap::new(),
            removals: Vec::new(),
            entries: Vec::new(),
            conflicts: Vec::new(),
            warnings: Vec::new(),
            status: status.into(),
            current_json: None,
            proposed_json: None,
            content_json: None,
            diff: Vec::new(),
        }
    }
}

/// Replace exact credential values with the mask.
///
/// Matching is structural and exact, so ordinary strings that merely contain a
/// credential are never mangled and the result stays valid JSON.
fn mask_sensitive(value: &Value, sensitive: &[String]) -> Value {
    if sensitive.is_empty() {
        return value.clone();
    }
    match value {
        Value::String(text) => {
            if sensitive.iter().any(|secret| secret == text) {
                Value::String(crate::models::REDACTED.to_string())
            } else {
                Value::String(text.clone())
            }
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|item| mask_sensitive(item, sensitive)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), mask_sensitive(item, sensitive)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Masked rendering of a document for previews, diffs and logs.
fn masked_canonical(value: &Value, sensitive: &[String]) -> String {
    canonical_json(&mask_sensitive(value, sensitive))
}

/// Signature of an existing harness entry, used for duplicate-endpoint checks.
fn entry_signature(entry: &Value) -> Option<String> {
    let object = entry.as_object()?;
    if let Some(url) = object
        .get("url")
        .or_else(|| object.get("serverUrl"))
        .and_then(Value::as_str)
    {
        return Some(format!("remote|{url}"));
    }
    match object.get("command") {
        Some(Value::String(command)) => {
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
            Some(format!("stdio|{command} {args}"))
        }
        Some(Value::Array(parts)) => {
            let joined = parts
                .iter()
                .filter_map(|part| part.as_str())
                .collect::<Vec<_>>()
                .join("\u{1f}");
            Some(format!("stdio|{joined}"))
        }
        _ => None,
    }
}

fn server_signature(server: &McpServer) -> Option<String> {
    match server.transport {
        TransportType::Stdio => {
            let command = server.command.clone().unwrap_or_default();
            if command.is_empty() {
                None
            } else {
                Some(format!("stdio|{}", {
                    let mut parts = vec![command];
                    parts.extend(server.args.iter().cloned());
                    parts.join(" ")
                }))
            }
        }
        _ => server.url.clone().map(|url| format!("remote|{url}")),
    }
}

fn sort_servers_for_harness(servers: &[McpServer], harness_id: &str, request_ids: Option<&Vec<String>>) -> Vec<McpServer> {
    servers
        .iter()
        .filter(|server| server.targets.iter().any(|target| target == harness_id))
        .filter(|server| match request_ids {
            Some(ids) => ids.iter().any(|id| id == &server.id),
            None => true,
        })
        .cloned()
        .collect()
}

fn plan_harness(
    ctx: &SyncContext<'_>,
    adapter: &dyn HarnessAdapter,
    harness_id: &str,
    servers: &[McpServer],
    request: &crate::models::SyncRequest,
) -> AppResult<HarnessPlan> {
    let hints = hints_for(ctx.settings);
    let Some(path) = adapter.locate_config(&hints) else {
        let mut plan = HarnessPlan::empty(adapter, "unsupported");
        plan.warnings.push(format!(
            "{} has no configuration location. Configure a path override or a generic profile.",
            adapter.display_name()
        ));
        return Ok(plan);
    };

    let document = adapter.read_config(&path)?;
    let mut plan = HarnessPlan {
        harness_id: harness_id.to_string(),
        harness_name: adapter.display_name(),
        path: Some(path.clone()),
        document: document.clone(),
        upserts: BTreeMap::new(),
        removals: Vec::new(),
        entries: Vec::new(),
        conflicts: Vec::new(),
        warnings: Vec::new(),
        status: "ready".into(),
        current_json: document.raw.clone(),
        proposed_json: None,
        content_json: None,
        diff: Vec::new(),
    };
    // Credential values resolved while planning are masked again before any
    // JSON leaves this function: previews and diffs never carry secrets.
    let mut sensitive: Vec<String> = Vec::new();

    for issue in adapter.validate_config(&document) {
        if issue.severity == "error" {
            plan.warnings.push(format!("{}: {}", issue.field, issue.message));
        }
    }

    if document.malformed.is_some() {
        plan.status = "error".into();
        plan.warnings.push(
            "The existing file is not valid JSON. It will not be modified; fix it manually or restore a backup first."
                .into(),
        );
        return Ok(plan);
    }
    if document.read_only {
        plan.status = "error".into();
        plan.warnings
            .push("The configuration file is read-only; writing requires adjusting file permissions.".into());
        return Ok(plan);
    }

    for server in servers {
        if !adapter.supports_transport(server.transport) {
            plan.entries.push(DiffEntry {
                name: server.name.clone(),
                action: DiffAction::Skip,
                details: format!(
                    "{} does not support the {} transport",
                    adapter.display_name(),
                    server.transport.as_str()
                ),
                before: adapter.existing_entry(&document, &server.name),
                after: None,
            });
            plan.warnings.push(format!(
                "{}: transport {} is not supported by {}",
                server.name,
                server.transport.as_str(),
                adapter.display_name()
            ));
            continue;
        }

        for issue in validate_definition_against_harness(server, adapter)? {
            plan.warnings.push(format!("{}: {}", server.name, issue.message));
        }

        let resolved = match resolve_entry_vars(ctx, server) {
            Ok(resolved) => resolved,
            Err(error) => {
                plan.entries.push(DiffEntry {
                    name: server.name.clone(),
                    action: DiffAction::Skip,
                    details: error.message.clone(),
                    before: adapter.existing_entry(&document, &server.name),
                    after: None,
                });
                plan.warnings.push(format!("{}: {}", server.name, error.message));
                continue;
            }
        };
        sensitive.extend(resolved.sensitive.iter().cloned());
        plan.warnings.extend(resolved.warnings.iter().map(|warning| format!("{}: {warning}", server.name)));

        let resolution = request.resolution(harness_id, &server.name);
        let action = resolution.map(|item| item.action.as_str()).unwrap_or("ask");

        if action == "skip" {
            plan.entries.push(DiffEntry {
                name: server.name.clone(),
                action: DiffAction::Skip,
                details: "kept the existing definition (user choice)".into(),
                before: adapter.existing_entry(&document, &server.name),
                after: None,
            });
            continue;
        }

        let target_name = if action == "rename" {
            resolution
                .and_then(|item| item.new_name.clone())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| server.name.clone())
        } else {
            server.name.clone()
        };

        let encoded = match adapter.merge_mcp_definition(&document, server, &resolved) {
            Ok(encoded) => encoded,
            Err(error) => {
                plan.entries.push(DiffEntry {
                    name: server.name.clone(),
                    action: DiffAction::Skip,
                    details: error.message.clone(),
                    before: None,
                    after: None,
                });
                plan.warnings.push(format!("{}: {}", server.name, error.message));
                continue;
            }
        };

        let existing = adapter.existing_entry(&document, &target_name);
        let mut handled = false;

        if action == "update" {
            if let Some(before) = existing.clone() {
                plan.upserts.insert(target_name.clone(), encoded.clone());
                plan.entries.push(DiffEntry {
                    name: target_name.clone(),
                    action: if before == encoded { DiffAction::Unchanged } else { DiffAction::Update },
                    details: if before == encoded {
                        "already up to date".into()
                    } else {
                        "replaced the existing definition with the registry version".into()
                    },
                    before: Some(before),
                    after: Some(encoded.clone()),
                });
                handled = true;
            }
        }

        if !handled {
            match &existing {
                None => {
                    // Duplicate endpoint under a different name?
                    if let Some(signature) = server_signature(server) {
                        let duplicate = adapter
                            .existing_entries(&document)
                            .into_iter()
                            .find(|entry| entry.name != target_name && entry_signature(&entry.value).as_ref() == Some(&signature));
                        if let Some(entry) = duplicate {
                            plan.conflicts.push(SyncConflict {
                                name: server.name.clone(),
                                kind: "duplicateEndpoint".into(),
                                message: format!(
                                    "'{}' points at the same endpoint or command as the existing entry '{}'. Choose how to continue.",
                                    server.name, entry.name
                                ),
                                existing: Some(entry.value.clone()),
                                incoming: Some(encoded.clone()),
                                resolutions: vec!["skip".into(), "rename".into(), "update".into(), "cancel".into()],
                                requires_removal_confirmation: false,
                            });
                            plan.entries.push(DiffEntry {
                                name: server.name.clone(),
                                action: DiffAction::Conflict,
                                details: format!("same endpoint as existing entry '{}'", entry.name),
                                before: Some(entry.value),
                                after: Some(encoded),
                            });
                        } else {
                            plan.upserts.insert(target_name.clone(), encoded.clone());
                            plan.entries.push(DiffEntry {
                                name: target_name.clone(),
                                action: DiffAction::Add,
                                details: if target_name != server.name {
                                    format!("added as '{}' (renamed from '{}')", target_name, server.name)
                                } else {
                                    "new entry".into()
                                },
                                before: None,
                                after: Some(encoded),
                            });
                        }
                    }
                }
                Some(before) => {
                    if before == &encoded {
                        plan.entries.push(DiffEntry {
                            name: target_name.clone(),
                            action: DiffAction::Unchanged,
                            details: "identical to the registry definition".into(),
                            before: Some(before.clone()),
                            after: Some(encoded.clone()),
                        });
                        if target_name != server.name {
                            plan.warnings.push(format!(
                                "'{}' is already present as '{}' with identical content.",
                                server.name, target_name
                            ));
                        }
                    } else {
                        plan.conflicts.push(SyncConflict {
                            name: server.name.clone(),
                            kind: "conflictingDefinition".into(),
                            message: format!(
                                "'{}' already exists in {} with a different definition (edited outside this app or by another client).",
                                target_name,
                                adapter.display_name()
                            ),
                            existing: Some(before.clone()),
                            incoming: Some(encoded.clone()),
                            resolutions: vec!["update".into(), "rename".into(), "skip".into(), "cancel".into()],
                            requires_removal_confirmation: false,
                        });
                        plan.entries.push(DiffEntry {
                            name: target_name.clone(),
                            action: DiffAction::Conflict,
                            details: "existing definition differs from the registry".into(),
                            before: Some(before.clone()),
                            after: Some(encoded),
                        });
                    }
                }
            }
        }
    }

    // Duplicate keys across registry entries targeting this harness.
    for conflict in detect_registry_conflicts(servers) {
        plan.warnings.push(conflict.message.clone());
        if conflict.kind == "duplicateRegistryKey" {
            plan.conflicts.push(SyncConflict {
                name: conflict.servers.join(", "),
                kind: conflict.kind.clone(),
                message: conflict.message.clone(),
                existing: None,
                incoming: None,
                resolutions: vec!["skip".into(), "cancel".into()],
                requires_removal_confirmation: false,
            });
        }
    }

    // Explicitly requested removals.
    for removal in request
        .removals
        .iter()
        .filter(|item: &&RemovalRequest| item.harness_id == harness_id)
    {
        let Some(existing) = adapter.existing_entry(&document, &removal.name) else {
            plan.warnings.push(format!(
                "Removal requested for '{}' but no such entry exists.",
                removal.name
            ));
            continue;
        };
        if request.confirm_removals {
            plan.removals.push(removal.name.clone());
            plan.entries.push(DiffEntry {
                name: removal.name.clone(),
                action: DiffAction::Remove,
                details: "removal confirmed by the user".into(),
                before: Some(existing),
                after: None,
            });
        } else {
            plan.conflicts.push(SyncConflict {
                name: removal.name.clone(),
                kind: "removalConfirmation".into(),
                message: format!("Removing '{}' requires explicit confirmation.", removal.name),
                existing: Some(existing),
                incoming: None,
                resolutions: vec!["remove".into(), "skip".into(), "cancel".into()],
                requires_removal_confirmation: true,
            });
            plan.entries.push(DiffEntry {
                name: removal.name.clone(),
                action: DiffAction::Remove,
                details: "awaiting confirmation".into(),
                before: adapter.existing_entry(&document, &removal.name),
                after: None,
            });
        }
    }

    let unresolved_conflicts = plan
        .conflicts
        .iter()
        .any(|conflict| !(conflict.requires_removal_confirmation && request.confirm_removals));

    if unresolved_conflicts {
        plan.status = "conflict".into();
    } else if plan.upserts.is_empty() && plan.removals.is_empty() {
        plan.status = "noChange".into();
        plan.proposed_json = Some(masked_canonical(&document.value, &sensitive));
    } else {
        match adapter.propose(&document, &plan.upserts, &plan.removals) {
            Ok(proposed) => {
                // `content_json` is what gets written (credentials included);
                // `proposed_json` is the masked copy the UI renders.
                let content_json = canonical_json(&proposed.value);
                let current_json = if document.existed {
                    Some(canonical_json(&document.value))
                } else {
                    Some(String::new())
                };
                plan.status = if current_json.as_deref() == Some(content_json.as_str()) {
                    "noChange".into()
                } else {
                    "ready".into()
                };
                let display_before = if document.existed {
                    masked_canonical(&document.value, &sensitive)
                } else {
                    String::new()
                };
                let display_proposed = masked_canonical(&proposed.value, &sensitive);
                plan.diff = compact_diff(diff_lines(&display_before, &display_proposed), 3);
                plan.content_json = Some(content_json);
                plan.proposed_json = Some(display_proposed);
            }
            Err(error) => {
                plan.status = "error".into();
                plan.warnings.push(error.message);
            }
        }
    }

    if !sensitive.is_empty() {
        // The "current file" view is masked as well so a credential that was
        // written earlier is not echoed back into the UI.
        plan.current_json = Some(masked_canonical(&document.value, &sensitive));
        plan.warnings.push(
            "One or more credential values are shown as •••••••• in this preview; the harness file receives the stored value."
                .into(),
        );
    }

    Ok(plan)
}

pub fn build_preview(ctx: &SyncContext<'_>, request: &crate::models::SyncRequest) -> AppResult<PreviewReport> {
    let started = now_rfc3339();
    let servers = ctx.db.list_servers()?;
    let mut previews = Vec::new();
    let mut resolutions_required = 0;

    for adapter in ctx.adapters {
        let harness_id = adapter.id();
        if let Some(ids) = &request.harness_ids {
            if !ids.iter().any(|id| id == &harness_id) {
                continue;
            }
        }
        let subset = sort_servers_for_harness(&servers, &harness_id, request.mcp_ids.as_ref());
        let explicit = request.mcp_ids.is_some() && !subset.is_empty();
        // A harness with no registry entries still has to be visited when the
        // user asked for a removal: otherwise "remove unmanaged entries" would
        // silently do nothing.
        let requested_removal = request
            .removals
            .iter()
            .any(|item| item.harness_id == harness_id);
        if subset.is_empty() && !explicit && !requested_removal {
            continue;
        }
        let plan = plan_harness(ctx, adapter.as_ref(), &harness_id, &subset, request)?;
        resolutions_required += plan.conflicts.len();
        previews.push(SyncPreview {
            harness_id: plan.harness_id.clone(),
            harness_name: plan.harness_name.clone(),
            config_path: plan.path.as_ref().map(|path| path.to_string_lossy().to_string()),
            config_format: adapter.config_format_label(),
            status: plan.status.clone(),
            entries: plan.entries.clone(),
            conflicts: plan.conflicts.clone(),
            warnings: plan.warnings.clone(),
            current_json: plan.current_json.clone(),
            proposed_json: plan.proposed_json.clone(),
            diff: plan.diff.clone(),
            malformed: plan.document.malformed.is_some(),
            current_hash: plan.document.hash.clone(),
            read_only: plan.document.read_only,
        });
    }

    let overall = if previews.iter().any(|preview| preview.status == "conflict") {
        "blocked"
    } else if previews.iter().all(|preview| preview.status == "noChange" || preview.status == "unsupported")
        && !previews.is_empty()
    {
        "noChange"
    } else if previews.is_empty() {
        "noChange"
    } else {
        "ready"
    };

    Ok(PreviewReport {
        started_at: started,
        finished_at: now_rfc3339(),
        overall: overall.into(),
        previews,
        resolutions_required,
    })
}

fn describe_conflicts(conflicts: &[SyncConflict]) -> String {
    conflicts
        .iter()
        .map(|conflict| format!("{} ({})", conflict.name, conflict.kind))
        .collect::<Vec<_>>()
        .join(", ")
}

fn apply_harness(
    ctx: &SyncContext<'_>,
    adapter: &dyn HarnessAdapter,
    plan: &HarnessPlan,
    now: &str,
) -> HarnessSyncResult {
    let mut result = HarnessSyncResult {
        harness_id: plan.harness_id.clone(),
        harness_name: plan.harness_name.clone(),
        status: "failed".into(),
        message: String::new(),
        applied: Vec::new(),
        backup_path: None,
        verified: false,
        error: None,
    };

    if plan.status == "unsupported" || plan.path.is_none() {
        result.status = "skipped".into();
        result.message = plan
            .warnings
            .first()
            .cloned()
            .unwrap_or_else(|| "no configuration location".into());
        return result;
    }
    if plan.status == "error" {
        result.status = "failed".into();
        result.message = plan.warnings.first().cloned().unwrap_or_else(|| "configuration error".into());
        result.error = Some(
            StructuredError {
                code: "CONFIG_UNWRITABLE".into(),
                message: result.message.clone(),
                remediation: Some("Fix or restore the file, then retry the synchronization.".into()),
                details: None,
            },
        );
        let _ = ctx.db.record_sync(
            &plan.harness_id,
            None,
            None,
            "sync",
            "failed",
            &format!("{}: aborted, file not modified", result.message),
        );
        persist_harness_error(ctx, &plan.harness_id, &result.message);
        return result;
    }
    if plan.status == "conflict" {
        result.status = "skipped".into();
        result.message = format!(
            "Unresolved conflicts prevented writing: {}",
            describe_conflicts(
                &plan
                    .conflicts
                    .iter()
                    .filter(|conflict| !conflict.requires_removal_confirmation)
                    .cloned()
                    .collect::<Vec<_>>()
            )
        );
        result.applied = plan
            .entries
            .iter()
            .map(|entry| AppliedChange {
                name: entry.name.clone(),
                action: entry.action.as_str().into(),
                message: entry.details.clone(),
            })
            .collect();
        let _ = ctx.db.record_sync(&plan.harness_id, None, None, "sync", "skipped", &result.message);
        return result;
    }

    let Some(path) = plan.path.clone() else {
        result.status = "skipped".into();
        result.message = "no configuration path".into();
        return result;
    };

    if plan.status == "noChange" {
        result.status = "noChange".into();
        result.applied = plan
            .entries
            .iter()
            .map(|entry| AppliedChange {
                name: entry.name.clone(),
                action: entry.action.as_str().into(),
                message: entry.details.clone(),
            })
            .collect();
        result.message = "Already up to date; nothing was written.".into();
        result.verified = true;
        let _ = ctx.db.record_sync(&plan.harness_id, None, None, "sync", "noChange", &result.message);
        let state = crate::models::HarnessState {
            harness_id: plan.harness_id.clone(),
            last_sync_at: Some(now.to_string()),
            last_error: None,
            last_known_hash: plan.document.hash.clone(),
            externally_modified: false,
        };
        let _ = ctx.db.upsert_harness_state(&state);
        return result;
    }

    let Some(content) = plan.content_json.clone() else {
        result.status = "failed".into();
        result.message = "no proposal generated".into();
        return result;
    };

    let backup = if ctx.settings.create_backups {
        match ctx.backups.backup_file(&plan.harness_id, &path, "pre-sync") {
            Ok(Some(info)) => match ctx.backups.store_backup(ctx.db, info) {
                Ok(stored) => Some(stored.backup_path),
                Err(error) => {
                    let _ = ctx.db.record_sync(
                        &plan.harness_id,
                        None,
                        None,
                        "backup",
                        "failed",
                        &error.message,
                    );
                    None
                }
            },
            Ok(None) => None,
            Err(error) => {
                result.status = "failed".into();
                result.message = format!("Backup failed, refusing to write: {}", error.message);
                result.error = Some(error.to_payload());
                let _ = ctx.db.record_sync(&plan.harness_id, None, None, "backup", "failed", &result.message);
                persist_harness_error(ctx, &plan.harness_id, &result.message);
                return result;
            }
        }
    } else {
        None
    };

    match adapter.write_config_safely(&path, &content, plan.document.hash.as_deref()) {
        Ok(outcome) => {
            let verification = adapter.read_config(&path);
            let verified = match verification {
                Ok(document) => {
                    document.malformed.is_none() && canonical_json(&document.value) == content
                }
                Err(_) => false,
            };
            result.backup_path = backup;
            result.verified = verified;
            result.status = if verified { "success".into() } else { "partial".into() };
            result.message = if outcome.wrote {
                if verified {
                    format!("Wrote {} entry mutation(s) and verified the file.", plan.upserts.len())
                } else {
                    "File was written but the verification re-read did not match the proposal.".into()
                }
            } else {
                "File already matched the proposal; no write was necessary.".into()
            };
            if !verified {
                result.error = Some(StructuredError {
                    code: "VERIFICATION_FAILED".into(),
                    message: "The written configuration did not match the proposal when re-read.".into(),
                    remediation: Some("Restore the pre-sync backup from the Sync History page.".into()),
                    details: None,
                });
            }

            result.applied = plan
                .entries
                .iter()
                .map(|entry| AppliedChange {
                    name: entry.name.clone(),
                    action: entry.action.as_str().into(),
                    message: entry.details.clone(),
                })
                .collect();

            for entry in &plan.entries {
                if matches!(entry.action, DiffAction::Add | DiffAction::Update | DiffAction::Remove) {
                    let _ = ctx.db.record_sync(
                        &plan.harness_id,
                        None,
                        Some(&entry.name),
                        entry.action.as_str(),
                        "success",
                        &entry.details,
                    );
                }
            }
            let _ = ctx.db.record_sync(&plan.harness_id, None, None, "sync", "success", &result.message);

            // Track the new on-disk hash so the file watcher does not report our
            // own write as an external change.
            let new_hash = crate::services::safe_io::read_snapshot(&path)
                .ok()
                .flatten()
                .map(|snapshot| snapshot.sha256);
            let state = crate::models::HarnessState {
                harness_id: plan.harness_id.clone(),
                last_sync_at: Some(now.to_string()),
                last_error: None,
                last_known_hash: new_hash,
                externally_modified: false,
            };
            let _ = ctx.db.upsert_harness_state(&state);

            for entry in &plan.entries {
                if matches!(entry.action, DiffAction::Add | DiffAction::Update) {
                    if let Ok(Some(server)) = ctx.db.find_server_by_name(&entry.name) {
                        let _ = ctx.db.mark_synced(&server.id, now);
                    }
                }
            }
        }
        Err(error) => {
            result.status = "failed".into();
            result.message = error.message.clone();
            result.backup_path = backup;
            result.error = Some(error.to_payload());
            let _ = ctx.db.record_sync(&plan.harness_id, None, None, "sync", "failed", &format!("{}: {}", plan.harness_id, error.message));
            persist_harness_error(ctx, &plan.harness_id, &error.message);
        }
    }

    result
}

fn persist_harness_error(ctx: &SyncContext<'_>, harness_id: &str, message: &str) {
    let previous = ctx
        .db
        .get_harness_state(harness_id)
        .ok()
        .flatten();
    let state = crate::models::HarnessState {
        harness_id: harness_id.to_string(),
        last_sync_at: previous.as_ref().and_then(|state| state.last_sync_at.clone()),
        last_error: Some(message.to_string()),
        last_known_hash: previous.as_ref().and_then(|state| state.last_known_hash.clone()),
        externally_modified: previous.map(|state| state.externally_modified).unwrap_or(false),
    };
    let _ = ctx.db.upsert_harness_state(&state);
}

pub fn apply(
    ctx: &SyncContext<'_>,
    request: &crate::models::SyncRequest,
    progress: &dyn Fn(&str, &str),
) -> AppResult<SyncReport> {
    let started = now_rfc3339();
    let now = started.clone();
    let servers = ctx.db.list_servers()?;
    let mut results = Vec::new();

    for adapter in ctx.adapters {
        let harness_id = adapter.id();
        if let Some(ids) = &request.harness_ids {
            if !ids.iter().any(|id| id == &harness_id) {
                continue;
            }
        }
        let subset = sort_servers_for_harness(&servers, &harness_id, request.mcp_ids.as_ref());
        let explicit = request.mcp_ids.is_some() && !subset.is_empty();
        let requested_removal = request
            .removals
            .iter()
            .any(|item| item.harness_id == harness_id);
        if subset.is_empty() && !explicit && !requested_removal {
            continue;
        }
        progress(&harness_id, "planning");
        let plan = match plan_harness(ctx, adapter.as_ref(), &harness_id, &subset, request) {
            Ok(plan) => plan,
            Err(error) => {
                let result = HarnessSyncResult {
                    harness_id: harness_id.clone(),
                    harness_name: adapter.display_name(),
                    status: "failed".into(),
                    message: error.message.clone(),
                    applied: Vec::new(),
                    backup_path: None,
                    verified: false,
                    error: Some(error.to_payload()),
                };
                let _ = ctx
                    .db
                    .record_sync(&harness_id, None, None, "sync", "failed", &error.message);
                results.push(result);
                continue;
            }
        };
        progress(&harness_id, if plan.status == "ready" { "writing" } else { "skipped" });
        let result = apply_harness(ctx, adapter.as_ref(), &plan, &now);
        results.push(result);
    }

    let failed = results.iter().filter(|item| item.status == "failed").count();
    let partial = results.iter().filter(|item| item.status == "partial").count();
    let skipped = results.iter().filter(|item| item.status == "skipped").count();
    let changed = results.iter().filter(|item| item.status == "success").count();
    let problems = failed + partial;
    let overall = if results.is_empty() {
        "noChange"
    } else if problems == results.len() {
        "failed"
    } else if problems > 0 || skipped > 0 {
        "partial"
    } else if changed == 0 {
        "noChange"
    } else {
        "success"
    };

    Ok(SyncReport {
        started_at: started,
        finished_at: now_rfc3339(),
        overall: overall.into(),
        results,
        dry_run: false,
    })
}

/// Encode a single definition for one harness (used by the JSON preview tab
/// before anything is saved).
pub fn preview_definition(
    ctx: &SyncContext<'_>,
    adapter: &dyn HarnessAdapter,
    server: &McpServer,
) -> AppResult<(String, Vec<String>)> {
    let path = adapter
        .locate_config(&hints_for(ctx.settings))
        .unwrap_or_else(|| PathBuf::from("<new-file>"));
    let document = if path.exists() {
        adapter.read_config(&path)?
    } else {
        ConfigDocument::empty_for(&adapter.id(), &path)
    };
    let resolved = resolve_entry_vars(ctx, server)?;
    let mut warnings = resolved.warnings.clone();
    if !adapter.supports_transport(server.transport) {
        warnings.push(format!(
            "{} does not support the {} transport; synchronization would skip this server.",
            adapter.display_name(),
            server.transport.as_str()
        ));
    }
    if server.cwd.is_some() && !adapter.capabilities().cwd {
        warnings.push("The working directory will be dropped by this adapter.".into());
    }
    if adapter.adapter_status() == crate::models::AdapterStatus::PendingVerification {
        warnings.push("This adapter is not verified: confirm the schema in the preview before writing.".into());
    }
    let entry = adapter.merge_mcp_definition(&document, server, &resolved)?;
    Ok((masked_canonical(&entry, &resolved.sensitive), warnings))
}

/// Helper used by tests and the JSON preview: encode one definition only.
pub fn encode_for_preview(
    ctx: &SyncContext<'_>,
    adapter: &dyn HarnessAdapter,
    existing: Option<&Value>,
    server: &McpServer,
) -> AppResult<Value> {
    let resolved = resolve_entry_vars(ctx, server)?;
    adapter.merge_mcp_definition(
        &ConfigDocument {
            harness_id: adapter.id(),
            path: PathBuf::from("<preview>"),
            existed: false,
            raw: None,
            value: serde_json::Value::Object(Default::default()),
            format: "JSON".into(),
            hash: None,
            read_only: false,
            malformed: None,
            size: 0,
        },
        server,
        &resolved,
    )
    .map(|entry| {
        let merged = match existing {
            Some(current) => {
                // Merge against an existing entry to show preserved extras.
                let mut merged = entry;
                if let (Some(target), Some(source)) = (merged.as_object_mut(), current.as_object()) {
                    for (key, value) in source {
                        target.entry(key.clone()).or_insert_with(|| value.clone());
                    }
                }
                merged
            }
            None => entry,
        };
        mask_sensitive(&merged, &resolved.sensitive)
    })
}
