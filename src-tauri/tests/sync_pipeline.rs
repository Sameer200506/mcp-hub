//! End-to-end synchronization scenarios.
//!
//! These exercise the same path a user goes through: read the existing file,
//! build a preview, resolve conflicts, write atomically, verify, back up and
//! roll back. Each test owns a temporary workspace.

mod common;

use std::path::PathBuf;
use std::time::Duration;

use common::{
    entry, only_preview, only_result, remote_server, removal, rename_resolution, request_for,
    resolution, stdio_server, var_literal, Fixture,
};
use serde_json::json;
use universal_mcp_manager_lib::models::{DiffAction, TransportType};
use universal_mcp_manager_lib::services::safe_io::sha256_hex;

/// The canonical rendering used by the engine for a fresh document.
fn canonical(value: serde_json::Value) -> String {
    let mut rendered = serde_json::to_string_pretty(&value).unwrap();
    rendered.push('\n');
    rendered
}

#[test]
fn merges_into_an_existing_configuration_without_touching_other_entries() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    fixture.write_json(
        &path,
        &json!({
            "$schema": "https://opencode.ai/config.json",
            "provider": { "anthropic": { "options": { "apiKey": "{env:ANTHROPIC_API_KEY}" } } },
            "mcp": {
                "existing": {
                    "type": "local",
                    "command": ["npx", "-y", "existing-server"],
                    "environment": { "KEEP": "1" }
                }
            }
        }),
    );

    fixture.insert_server(&stdio_server("filesystem", &["opencode"]));

    let preview = fixture.preview(&request_for(&["opencode"]));
    assert_eq!(only_preview(&preview).status, "ready");
    assert_eq!(only_preview(&preview).entries[0].action, DiffAction::Add);

    let report = fixture.apply(&request_for(&["opencode"]));
    let result = only_result(&report);
    assert_eq!(report.overall, "success");
    assert_eq!(result.status, "success");
    assert!(result.verified, "the written file must be re-read and verified");
    assert!(result.backup_path.is_some());

    let document = fixture.read_json(&path);
    assert_eq!(document["$schema"], json!("https://opencode.ai/config.json"));
    assert_eq!(
        document["provider"]["anthropic"]["options"]["apiKey"],
        json!("{env:ANTHROPIC_API_KEY}")
    );
    assert_eq!(
        document["mcp"]["existing"]["command"],
        json!(["npx", "-y", "existing-server"])
    );
    assert_eq!(document["mcp"]["existing"]["environment"], json!({ "KEEP": "1" }));
    assert_eq!(document["mcp"]["filesystem"]["type"], json!("local"));
    assert_eq!(
        document["mcp"]["filesystem"]["command"],
        json!(["npx", "-y", "@example/filesystem"])
    );

    // The registry records the sync so the dashboard can show staleness.
    let stored = fixture.db.get_server(&"id-filesystem".to_string()).unwrap();
    assert!(stored.last_sync_at.is_some());
    let history = fixture.db.sync_history(10).unwrap();
    assert!(history.iter().any(|item| item.harness_id == "opencode" && item.status == "success"));
}

#[test]
fn creates_a_missing_configuration_including_parent_directories() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("cline", "cline/nested/settings/cline_mcp_settings.json");
    fixture.insert_server(&stdio_server("filesystem", &["cline"]));

    let report = fixture.apply(&request_for(&["cline"]));

    assert_eq!(only_result(&report).status, "success");
    assert_eq!(only_result(&report).backup_path, None, "nothing existed to back up");
    let document = fixture.read_json(&path);
    assert_eq!(document["mcpServers"]["filesystem"]["command"], json!("npx"));
}

#[test]
fn preserves_unknown_keys_when_an_entry_is_updated() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    fixture.write_json(
        &path,
        &json!({
            "unrelated": { "keep": [1, 2, 3] },
            "mcp": {
                "filesystem": {
                    "type": "local",
                    "command": ["npx", "-y", "@example/old"],
                    "timeout": 15000,
                    "oauth": false,
                    "customField": { "nested": true },
                    "enabled": true
                }
            }
        }),
    );
    let server = stdio_server("filesystem", &["opencode"]);
    fixture.insert_server(&server);

    // The differing definition is a conflict, never a silent overwrite.
    let preview = fixture.preview(&request_for(&["opencode"]));
    let single = only_preview(&preview);
    assert_eq!(single.status, "conflict");
    assert_eq!(single.conflicts[0].kind, "conflictingDefinition");
    assert_eq!(
        fixture.read_json(&path)["mcp"]["filesystem"]["command"],
        json!(["npx", "-y", "@example/old"])
    );

    // With an explicit "update" resolution the registry version wins and the
    // harness-specific extras survive.
    let mut request = request_for(&["opencode"]);
    request
        .resolutions
        .push(resolution("opencode", "filesystem", "update"));
    let report = fixture.apply(&request);
    assert_eq!(only_result(&report).status, "success");

    let document = fixture.read_json(&path);
    let updated = entry(&document, "mcp", "filesystem");
    assert_eq!(updated["command"], json!(["npx", "-y", "@example/filesystem"]));
    assert_eq!(updated["timeout"], json!(15000));
    assert_eq!(updated["oauth"], json!(false));
    assert_eq!(updated["customField"], json!({ "nested": true }));
    assert_eq!(updated["enabled"], json!(true));
    assert_eq!(document["unrelated"], json!({ "keep": [1, 2, 3] }));
}

#[test]
fn malformed_configuration_is_never_overwritten() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    let broken = "{ \"mcp\": { \"filesystem\": { \"command\": [\"npx\", ] }";
    fixture.write(&path, broken);
    fixture.insert_server(&stdio_server("filesystem", &["opencode"]));

    let preview = fixture.preview(&request_for(&["opencode"]));
    let single = only_preview(&preview);
    assert_eq!(single.status, "error");
    assert!(single.malformed);
    assert!(
        single
            .warnings
            .iter()
            .any(|warning| warning.contains("not valid JSON")),
        "the user must be told why nothing will be written: {:?}",
        single.warnings
    );
    assert!(single.current_hash.is_some(), "the hash is still reported for inspection");

    let report = fixture.apply(&request_for(&["opencode"]));
    let result = only_result(&report);
    assert_eq!(result.status, "failed");
    let error = result.error.clone().expect("structured error");
    assert_eq!(error.code, "CONFIG_UNWRITABLE");
    assert_eq!(fixture.read(&path), broken, "the broken file is untouched");
}

#[test]
fn read_only_configuration_is_never_overwritten() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    let original = canonical(json!({ "mcp": {} }));
    fixture.write(&path, &original);
    fixture.set_read_only(&path, true);
    fixture.insert_server(&stdio_server("filesystem", &["opencode"]));

    let preview = fixture.preview(&request_for(&["opencode"]));
    let single = only_preview(&preview);
    assert_eq!(single.status, "error");
    assert!(single.read_only);
    assert!(single
        .warnings
        .iter()
        .any(|warning| warning.contains("read-only")));

    let report = fixture.apply(&request_for(&["opencode"]));
    let result = only_result(&report);
    assert_eq!(result.status, "failed");
    assert_eq!(result.error.clone().unwrap().code, "CONFIG_UNWRITABLE");
    assert_eq!(fixture.read(&path), original);

    // Restore the attribute so the temporary directory can be removed.
    fixture.set_read_only(&path, false);
}

#[test]
fn a_backup_of_the_previous_content_is_recorded_before_writing() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    let original = canonical(json!({ "mcp": {} }));
    fixture.write(&path, &original);
    fixture.insert_server(&stdio_server("filesystem", &["opencode"]));

    let report = fixture.apply(&request_for(&["opencode"]));
    let backup_path = PathBuf::from(
        only_result(&report)
            .backup_path
            .clone()
            .expect("a backup path is reported"),
    );

    assert!(backup_path.exists());
    assert_eq!(fixture.read(&backup_path), original);
    assert!(backup_path.starts_with(fixture.backups.directory()));
    assert_ne!(fixture.read(&path), original, "the file was actually written");

    let stored = fixture.db.list_backups(Some("opencode")).unwrap();
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].reason, "pre-sync");
    assert_eq!(stored[0].sha256, sha256_hex(original.as_bytes()));
    assert_eq!(stored[0].original_path, path.to_string_lossy());
    assert_eq!(stored[0].backup_path, backup_path.to_string_lossy());
}

#[test]
fn a_write_is_refused_when_the_file_changed_after_it_was_read() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    fixture.write(&path, &canonical(json!({ "mcp": {} })));

    let adapter = fixture.adapter("opencode");
    let document = adapter.read_config(&path).unwrap();
    let stale_hash = document.hash.clone().unwrap();

    // Another program edits the file between the preview and the write.
    fixture.write(
        &path,
        &canonical(json!({ "mcp": { "external": { "type": "local", "command": ["other"] } } })),
    );

    let error = adapter
        .write_config_safely(&path, &canonical(json!({ "mcp": {} })), Some(&stale_hash))
        .expect_err("a stale hash must abort the write");

    assert_eq!(error.code, "FILE_CHANGED_ON_DISK");
    assert!(fixture.read_json(&path)["mcp"]["external"].is_object());
}

#[test]
fn external_edits_are_reported_and_never_silently_clobbered() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    fixture.write_json(&path, &json!({ "mcp": {} }));
    fixture.insert_server(&stdio_server("filesystem", &["opencode"]));

    assert_eq!(fixture.apply(&request_for(&["opencode"])).overall, "success");
    let infos = fixture.harness_infos();
    let opencode = infos.iter().find(|info| info.id == "opencode").unwrap();
    assert!(!opencode.externally_modified);

    // Someone edits the harness file outside the manager.
    let mut document = fixture.read_json(&path);
    document["mcp"]["filesystem"]["command"] = json!(["npx", "-y", "@example/edited-outside"]);
    fixture.write_json(&path, &document);

    let infos = fixture.harness_infos();
    let opencode = infos.iter().find(|info| info.id == "opencode").unwrap();
    assert!(opencode.externally_modified, "the drift must be surfaced");

    // The next synchronization refuses to overwrite it without a decision.
    let preview = fixture.preview(&request_for(&["opencode"]));
    assert_eq!(only_preview(&preview).status, "conflict");
    let report = fixture.apply(&request_for(&["opencode"]));
    assert_eq!(only_result(&report).status, "skipped");
    assert_eq!(
        fixture.read_json(&path)["mcp"]["filesystem"]["command"],
        json!(["npx", "-y", "@example/edited-outside"])
    );
}

#[test]
fn synchronizing_twice_writes_nothing_the_second_time() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    fixture.write_json(&path, &json!({ "mcp": {} }));
    fixture.insert_server(&stdio_server("filesystem", &["opencode"]));

    let first = fixture.apply(&request_for(&["opencode"]));
    assert_eq!(only_result(&first).status, "success");
    let written = fixture.read(&path);
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();

    std::thread::sleep(Duration::from_millis(30));

    let second = fixture.apply(&request_for(&["opencode"]));
    assert_eq!(second.overall, "noChange");
    assert_eq!(only_result(&second).status, "noChange");
    assert_eq!(fixture.read(&path), written);
    assert_eq!(
        std::fs::metadata(&path).unwrap().modified().unwrap(),
        modified,
        "an identical proposal must not touch the file"
    );

    let preview = fixture.preview(&request_for(&["opencode"]));
    assert_eq!(only_preview(&preview).status, "noChange");
    let backups = fixture.db.list_backups(Some("opencode")).unwrap();
    assert_eq!(backups.len(), 1, "the second run created no extra backup");
}

#[test]
fn one_broken_harness_does_not_block_the_others() {
    let mut fixture = Fixture::new();
    let opencode_path = fixture.point_at("opencode", "opencode/opencode.json");
    let cline_path = fixture.point_at("cline", "cline/cline_mcp_settings.json");
    fixture.write_json(&opencode_path, &json!({ "mcp": {} }));
    let broken = "{ this is not json";
    fixture.write(&cline_path, broken);
    fixture.insert_server(&stdio_server("filesystem", &["opencode", "cline"]));

    let report = fixture.apply(&request_for(&["opencode", "cline"]));

    assert_eq!(report.overall, "partial");
    let opencode = report.results.iter().find(|item| item.harness_id == "opencode").unwrap();
    let cline = report.results.iter().find(|item| item.harness_id == "cline").unwrap();
    assert_eq!(opencode.status, "success");
    assert_eq!(cline.status, "failed");
    assert_eq!(cline.error.clone().unwrap().code, "CONFIG_UNWRITABLE");
    assert_eq!(fixture.read(&cline_path), broken);
    assert_eq!(
        fixture.read_json(&opencode_path)["mcp"]["filesystem"]["command"],
        json!(["npx", "-y", "@example/filesystem"])
    );
}

#[test]
fn only_the_requested_harness_is_touched() {
    let mut fixture = Fixture::new();
    let opencode_path = fixture.point_at("opencode", "opencode/opencode.json");
    let cline_path = fixture.point_at("cline", "cline/cline_mcp_settings.json");
    fixture.write_json(&opencode_path, &json!({ "mcp": {} }));
    fixture.write_json(&cline_path, &json!({ "mcpServers": {} }));
    fixture.insert_server(&stdio_server("filesystem", &["opencode", "cline"]));

    let report = fixture.apply(&request_for(&["opencode"]));

    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].harness_id, "opencode");
    assert!(fixture.read_json(&opencode_path)["mcp"]["filesystem"].is_object());
    assert!(
        fixture.read_json(&cline_path)["mcpServers"]
            .as_object()
            .unwrap()
            .is_empty(),
        "the untouched harness keeps its configuration"
    );
}

#[test]
fn conflicting_definitions_can_be_updated_renamed_or_skipped() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    let existing = json!({
        "mcp": {
            "filesystem": { "type": "local", "command": ["npx", "-y", "@example/old"] },
            "other": { "type": "local", "command": ["npx", "-y", "other"] }
        }
    });
    fixture.insert_server(&stdio_server("filesystem", &["opencode"]));

    // 1. skip: the existing entry wins and nothing is written.
    fixture.write_json(&path, &existing);
    let mut request = request_for(&["opencode"]);
    request.resolutions.push(resolution("opencode", "filesystem", "skip"));
    let report = fixture.apply(&request);
    assert_eq!(only_result(&report).status, "noChange");
    assert_eq!(
        fixture.read_json(&path)["mcp"]["filesystem"]["command"],
        json!(["npx", "-y", "@example/old"])
    );

    // 2. rename: the registry version is written under a new key.
    fixture.write_json(&path, &existing);
    let mut request = request_for(&["opencode"]);
    request
        .resolutions
        .push(rename_resolution("opencode", "filesystem", "filesystem-v2"));
    let report = fixture.apply(&request);
    assert_eq!(only_result(&report).status, "success");
    let document = fixture.read_json(&path);
    assert_eq!(
        document["mcp"]["filesystem"]["command"],
        json!(["npx", "-y", "@example/old"]),
        "the original entry is preserved"
    );
    assert_eq!(
        document["mcp"]["filesystem-v2"]["command"],
        json!(["npx", "-y", "@example/filesystem"])
    );
    assert_eq!(document["mcp"]["other"]["command"], json!(["npx", "-y", "other"]),);
}

#[test]
fn duplicate_endpoints_are_flagged_before_writing() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    fixture.write_json(
        &path,
        &json!({
            "mcp": {
                "search-legacy": { "type": "remote", "url": "https://mcp.example.test/mcp" }
            }
        }),
    );
    fixture.insert_server(&remote_server(
        "search",
        "https://mcp.example.test/mcp",
        &["opencode"],
    ));

    let preview = fixture.preview(&request_for(&["opencode"]));
    let single = only_preview(&preview);
    assert_eq!(single.status, "conflict");
    assert_eq!(single.conflicts[0].kind, "duplicateEndpoint");
    assert!(single.conflicts[0].message.contains("search-legacy"));
    assert!(single
        .conflicts[0]
        .resolutions
        .iter()
        .any(|item| item == "rename"));

    let report = fixture.apply(&request_for(&["opencode"]));
    assert_eq!(only_result(&report).status, "skipped");
    assert!(fixture.read_json(&path)["mcp"].get("search").is_none());
}

#[test]
fn removals_require_explicit_confirmation_and_are_never_dropped() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    // No registry entry targets OpenCode: this is the "remove unmanaged
    // entries" flow that must still be planned.
    fixture.write_json(
        &path,
        &json!({
            "mcp": { "filesystem": { "type": "local", "command": ["npx", "-y", "old"] } }
        }),
    );

    let mut request = request_for(&["opencode"]);
    request.removals.push(removal("opencode", "filesystem"));

    let preview = fixture.preview(&request);
    assert_eq!(preview.previews.len(), 1, "the removal must be planned");
    let single = only_preview(&preview);
    assert_eq!(single.status, "conflict");
    assert_eq!(single.conflicts[0].kind, "removalConfirmation");
    assert!(single.conflicts[0].requires_removal_confirmation);

    let report = fixture.apply(&request);
    assert_eq!(only_result(&report).status, "skipped");
    assert!(fixture.read_json(&path)["mcp"]["filesystem"].is_object());

    // Confirmed removals are applied and recorded.
    let mut confirmed = request_for(&["opencode"]);
    confirmed.removals.push(removal("opencode", "filesystem"));
    confirmed.confirm_removals = true;
    let report = fixture.apply(&confirmed);
    assert_eq!(only_result(&report).status, "success");
    assert!(fixture.read_json(&path)["mcp"].get("filesystem").is_none());
    let history = fixture.db.sync_history(10).unwrap();
    assert!(history
        .iter()
        .any(|item| item.action == "remove" && item.status == "success"));
}

#[test]
fn the_latest_backup_can_be_restored() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    let original = canonical(json!({ "mcp": { "existing": { "command": "keep-me" } } }));
    fixture.write(&path, &original);
    fixture.insert_server(&stdio_server("filesystem", &["opencode"]));

    let report = fixture.apply(&request_for(&["opencode"]));
    let backup = fixture.db.list_backups(Some("opencode")).unwrap().remove(0);
    assert_ne!(fixture.read(&path), original);
    assert_eq!(
        only_result(&report).backup_path.clone().unwrap(),
        backup.backup_path
    );

    let restored = fixture.backups.restore(&fixture.db, &backup).unwrap();

    assert!(restored.restored);
    assert_eq!(restored.target_path, path.to_string_lossy());
    assert_eq!(fixture.read(&path), original);
    let safety = PathBuf::from(restored.safety_backup_path.expect("safety copy"));
    assert!(safety.exists());
    assert_ne!(fixture.read(&safety), original, "the replaced state is preserved");
    let stored = fixture.db.list_backups(Some("opencode")).unwrap();
    assert!(stored.iter().any(|item| item.reason == "restore-safety"));
}

#[test]
fn restoring_a_tampered_backup_is_refused() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    let original = canonical(json!({ "mcp": {} }));
    fixture.write(&path, &original);
    fixture.insert_server(&stdio_server("filesystem", &["opencode"]));
    fixture.apply(&request_for(&["opencode"]));

    let mut backup = fixture.db.list_backups(Some("opencode")).unwrap().remove(0);
    fixture.write(&PathBuf::from(&backup.backup_path), "{\"mcp\":{\"tampered\":true}}");
    backup.sha256 = sha256_hex(b"{\"mcp\":{}}");

    let error = fixture.backups.restore(&fixture.db, &backup).unwrap_err();
    assert_eq!(error.code, "VALIDATION_FAILED");
    assert!(!fixture.read(&path).contains("tampered"));
}

#[test]
fn updates_remove_the_legacy_type_field_from_antigravity_entries() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("antigravity", "antigravity/mcp_config.json");
    fixture.write_json(
        &path,
        &json!({
            "mcpServers": {
                "filesystem": {
                    "type": "stdio",
                    "command": "npx",
                    "args": ["-y", "@example/old"],
                    "disabledTools": ["delete_file"]
                }
            }
        }),
    );
    fixture.insert_server(&stdio_server("filesystem", &["antigravity"]));

    let mut request = request_for(&["antigravity"]);
    request
        .resolutions
        .push(resolution("antigravity", "filesystem", "update"));
    let report = fixture.apply(&request);
    assert_eq!(only_result(&report).status, "success");

    let document = fixture.read_json(&path);
    let updated = entry(&document, "mcpServers", "filesystem");
    assert!(
        updated.get("type").is_none(),
        "Antigravity rejects an explicit `type`: {updated}"
    );
    assert_eq!(updated["args"], json!(["-y", "@example/filesystem"]));
    assert_eq!(updated["disabledTools"], json!(["delete_file"]), "extras survive");
}

#[test]
fn unsupported_transports_are_skipped_with_a_warning() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    fixture.write_json(&path, &json!({ "mcp": {} }));
    let mut server = remote_server("legacy-sse", "https://mcp.example.test/sse", &["opencode"]);
    server.transport = TransportType::Sse;
    fixture.insert_server(&server);

    let preview = fixture.preview(&request_for(&["opencode"]));
    let single = only_preview(&preview);
    assert_eq!(single.status, "noChange");
    assert_eq!(single.entries[0].action, DiffAction::Skip);
    assert!(single
        .entries[0]
        .details
        .contains("does not support the sse transport"));

    let report = fixture.apply(&request_for(&["opencode"]));
    assert_eq!(only_result(&report).status, "noChange");
    assert!(fixture.read_json(&path)["mcp"].get("legacy-sse").is_none());
}

#[test]
fn a_working_directory_is_reported_as_dropped_for_cline() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("cline", "cline/cline_mcp_settings.json");
    fixture.write_json(&path, &json!({ "mcpServers": {} }));
    let mut server = stdio_server("filesystem", &["cline"]);
    server.cwd = Some("C:/data".into());
    fixture.insert_server(&server);

    let preview = fixture.preview(&request_for(&["cline"]));
    assert!(only_preview(&preview)
        .warnings
        .iter()
        .any(|warning| warning.contains("dropped")));

    let report = fixture.apply(&request_for(&["cline"]));
    assert_eq!(only_result(&report).status, "success");
    let document = fixture.read_json(&path);
    assert!(entry(&document, "mcpServers", "filesystem").get("cwd").is_none());
}

#[test]
fn environment_references_are_written_verbatim() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    fixture.write_json(&path, &json!({ "mcp": {} }));
    let mut server = stdio_server("github", &["opencode"]);
    server
        .env
        .insert("GITHUB_TOKEN".into(), var_literal("{env:GITHUB_TOKEN}"));
    fixture.insert_server(&server);

    let report = fixture.apply(&request_for(&["opencode"]));

    assert_eq!(only_result(&report).status, "success");
    let document = fixture.read_json(&path);
    assert_eq!(
        document["mcp"]["github"]["environment"]["GITHUB_TOKEN"],
        json!("{env:GITHUB_TOKEN}")
    );
}
