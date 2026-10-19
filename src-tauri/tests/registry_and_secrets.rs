//! Registry CRUD, import/export and credential handling.

mod common;

use std::collections::BTreeMap;

use common::{only_preview, only_result, request_for, stdio_server, var_literal, Fixture};
use serde_json::json;
use universal_mcp_manager_lib::models::{
    McpServerInput, RegistryExportRequest, RegistryImportRequest, TransportType, VarKind, VarValue,
    REDACTED,
};
use universal_mcp_manager_lib::services::config_validation::validate_input;
use universal_mcp_manager_lib::services::mcp_registry;

fn input(name: &str, targets: &[&str]) -> McpServerInput {
    McpServerInput {
        id: None,
        name: name.into(),
        description: String::new(),
        transport: TransportType::Stdio,
        command: Some("npx".into()),
        args: vec!["-y".into(), format!("@example/{name}")],
        env: BTreeMap::new(),
        headers: BTreeMap::new(),
        url: None,
        cwd: None,
        enabled: true,
        targets: targets.iter().map(|value| (*value).to_string()).collect(),
        tags: vec![],
        notes: String::new(),
    }
}

fn unique_id(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    format!("{prefix}-{}-{nanos:x}", std::process::id())
}

#[test]
fn registry_export_and_import_round_trip() {
    let source = Fixture::new();
    let mut local = stdio_server("filesystem", &["opencode"]);
    local.env.insert("ROOT".into(), var_literal("C:/data"));
    let remote = common::remote_server("search", "https://mcp.example.test/mcp", &["cline"]);
    let mut disabled = stdio_server("retired", &["opencode"]);
    disabled.enabled = false;
    for server in [&local, &remote, &disabled] {
        source.insert_server(server);
    }

    let export_path = source.path("export/registry.json");
    let summary = mcp_registry::export_registry(
        &source.db,
        &RegistryExportRequest {
            path: export_path.to_string_lossy().into_owned(),
            include_disabled: true,
            include_literal_env_values: true,
        },
    )
    .expect("export");
    assert!(summary.contains("3 server definition(s)"), "{summary}");
    assert!(export_path.exists());

    let target = Fixture::new();
    let result = mcp_registry::import_registry(
        &target.db,
        &target.secrets,
        &RegistryImportRequest {
            path: export_path.to_string_lossy().into_owned(),
            mode: "merge".into(),
        },
    )
    .expect("import");

    assert_eq!(result.imported, 3);
    assert_eq!(result.updated, 0);
    assert_eq!(result.skipped, 0);

    let imported = target.db.list_servers().unwrap();
    assert_eq!(imported.len(), 3);
    let filesystem = target.db.get_server("id-filesystem").unwrap();
    assert_eq!(filesystem.name, "filesystem");
    assert_eq!(filesystem.args, vec!["-y", "@example/filesystem"]);
    assert_eq!(filesystem.env["ROOT"].value, "C:/data");
    assert_eq!(filesystem.targets, vec!["opencode"]);
    assert!(!target.db.get_server("id-retired").unwrap().enabled);
    assert_eq!(
        target.db.get_server("id-search").unwrap().url.as_deref(),
        Some("https://mcp.example.test/mcp")
    );

    // Re-importing the same file updates instead of duplicating.
    let again = mcp_registry::import_registry(
        &target.db,
        &target.secrets,
        &RegistryImportRequest {
            path: export_path.to_string_lossy().into_owned(),
            mode: "merge".into(),
        },
    )
    .unwrap();
    assert_eq!(again.imported, 0);
    assert_eq!(again.updated, 3);
    assert_eq!(target.db.list_servers().unwrap().len(), 3);
}

#[test]
fn export_never_writes_secret_material() {
    let fixture = Fixture::new();
    let credential = "sk-test-abcdefghijklmnop";
    let mut server = stdio_server("github", &["opencode"]);
    server
        .env
        .insert("GITHUB_TOKEN".into(), var_literal(credential));
    server
        .env
        .insert("SAFE".into(), var_literal("plain-value"));
    fixture.insert_server(&server);

    let export_path = fixture.path("export/registry.json");
    let summary = mcp_registry::export_registry(
        &fixture.db,
        &RegistryExportRequest {
            path: export_path.to_string_lossy().into_owned(),
            include_disabled: false,
            include_literal_env_values: false,
        },
    )
    .unwrap();

    let written = fixture.read(&export_path);
    assert!(
        !written.contains(credential),
        "a credential-looking literal must be redacted on export"
    );
    assert!(written.contains("plain-value"));
    assert!(summary.contains("redaction warning"));

    let document: serde_json::Value = serde_json::from_str(&written).unwrap();
    assert_eq!(document["includesSecrets"], json!(false));
    assert_eq!(document["servers"][0]["env"]["GITHUB_TOKEN"]["value"], json!(""));
    assert!(document["warnings"].as_array().unwrap().len() == 1);
}

#[test]
fn import_in_replace_mode_removes_entries_that_are_not_in_the_file() {
    let source = Fixture::new();
    source.insert_server(&stdio_server("filesystem", &["opencode"]));
    let export_path = source.path("registry.json");
    mcp_registry::export_registry(
        &source.db,
        &RegistryExportRequest {
            path: export_path.to_string_lossy().into_owned(),
            include_disabled: true,
            include_literal_env_values: true,
        },
    )
    .unwrap();

    let target = Fixture::new();
    target.insert_server(&stdio_server("obsolete", &["opencode"]));

    let result = mcp_registry::import_registry(
        &target.db,
        &target.secrets,
        &RegistryImportRequest {
            path: export_path.to_string_lossy().into_owned(),
            mode: "replace".into(),
        },
    )
    .unwrap();

    assert_eq!(result.imported, 1);
    let remaining: Vec<String> = target
        .db
        .list_servers()
        .unwrap()
        .into_iter()
        .map(|server| server.name)
        .collect();
    assert_eq!(remaining, vec!["filesystem"]);
    assert!(target.db.find_server_by_name("obsolete").unwrap().is_none());
}

#[test]
fn importing_something_that_is_not_a_registry_file_fails_cleanly() {
    let fixture = Fixture::new();
    let path = fixture.path("not-a-registry.json");
    fixture.write(&path, "{\"mcpServers\":{}}");

    let error = mcp_registry::import_registry(
        &fixture.db,
        &fixture.secrets,
        &RegistryImportRequest {
            path: path.to_string_lossy().into_owned(),
            mode: "merge".into(),
        },
    )
    .unwrap_err();

    assert_eq!(error.code, "VALIDATION_FAILED");
    assert!(fixture.db.list_servers().unwrap().is_empty());
}

#[test]
fn the_redacted_view_hides_every_credential_like_value() {
    let mut server = stdio_server("github", &["opencode"]);
    server.env.insert(
        "GITHUB_TOKEN".into(),
        VarValue {
            value: "ghp_shouldstayhidden".into(),
            kind: VarKind::Secret,
        },
    );
    server.env.insert(
        "OTHER".into(),
        var_literal("sk-literal-that-looks-secret"),
    );
    server.env.insert("SAFE".into(), var_literal("plain"));

    let masked = mcp_registry::redacted(&server);

    assert_eq!(masked.env["GITHUB_TOKEN"].value, "");
    assert_eq!(masked.env["OTHER"].value, REDACTED);
    assert_eq!(masked.env["SAFE"].value, "plain");
    let rendered = serde_json::to_string(&masked).unwrap();
    assert!(!rendered.contains("ghp_shouldstayhidden"));
    assert!(!rendered.contains("sk-literal-that-looks-secret"));
}

#[test]
fn a_secret_without_a_stored_value_is_rejected_instead_of_written_empty() {
    let fixture = Fixture::new();
    let mut definition = input("github", &["opencode"]);
    definition.env.insert(
        "GITHUB_TOKEN".into(),
        VarValue {
            value: String::new(),
            kind: VarKind::Secret,
        },
    );

    let error = mcp_registry::create_server(&fixture.db, &fixture.secrets, &fixture.adapters, &definition)
        .unwrap_err();

    assert_eq!(error.code, "VALIDATION_FAILED");
    assert!(error.message.contains("GITHUB_TOKEN"));
    assert!(fixture.db.list_servers().unwrap().is_empty());
}

#[test]
fn validation_flags_literal_values_that_look_like_credentials() {
    let fixture = Fixture::new();
    let mut definition = input("github", &["opencode"]);
    definition
        .env
        .insert("GITHUB_TOKEN".into(), var_literal("ghp_abcdefghijklmnop"));

    let report = validate_input(&definition, &fixture.adapters);
    assert!(
        report.valid,
        "a literal credential is a warning, not an error: {:?}",
        report.issues
    );
    let harness = report
        .per_harness
        .iter()
        .find(|item| item.harness_id == "opencode")
        .expect("opencode is targeted");
    assert!(harness
        .issues
        .iter()
        .any(|issue| issue.message.contains("looks like a credential")));

    // The registry accepts it, so nothing blocks the user — but the value is
    // flagged for the export redaction as well.
    assert!(universal_mcp_manager_lib::services::config_validation::looks_like_secret(
        "ghp_abcdefghijklmnop"
    ));
}

#[test]
fn duplicate_registry_names_are_rejected() {
    let fixture = Fixture::new();
    mcp_registry::create_server(&fixture.db, &fixture.secrets, &fixture.adapters, &input("filesystem", &["opencode"]))
        .unwrap();

    let error = mcp_registry::create_server(
        &fixture.db,
        &fixture.secrets,
        &fixture.adapters,
        &input("FileSystem", &["opencode"]),
    )
    .unwrap_err();

    assert_eq!(error.code, "CONFLICT");
    assert_eq!(fixture.db.list_servers().unwrap().len(), 1);
}

/// Exercises the real Windows Credential Manager. It is skipped (with a printed
/// note) when the store is unavailable, and always removes the credential it
/// created.
#[cfg(windows)]
#[test]
fn a_stored_credential_stays_out_of_the_registry_and_the_preview() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    fixture.write_json(&path, &json!({ "mcp": {} }));

    let id = unique_id("uum-it");
    let secret = format!("ghp_{}", unique_id("value"));
    let mut definition = input("github", &["opencode"]);
    definition.id = Some(id.clone());
    definition.env.insert(
        "GITHUB_TOKEN".into(),
        VarValue {
            value: secret.clone(),
            kind: VarKind::Secret,
        },
    );

    let created = match mcp_registry::create_server(
        &fixture.db,
        &fixture.secrets,
        &fixture.adapters,
        &definition,
    ) {
        Ok(created) => created,
        Err(error) => {
            assert_eq!(
                error.code, "SECRET_STORE_ERROR",
                "unexpected failure: {error:?}"
            );
            eprintln!(
                "skipping: the Windows Credential Manager is unavailable ({})",
                error.message
            );
            return;
        }
    };

    // The database keeps only metadata.
    let stored = fixture.db.get_server(&id).unwrap();
    assert_eq!(stored.env["GITHUB_TOKEN"].value, "");
    assert_eq!(stored.env["GITHUB_TOKEN"].kind, VarKind::Secret);
    assert!(!serde_json::to_string(&stored).unwrap().contains(&secret));
    assert_eq!(
        fixture.db.list_secret_refs(&id).unwrap(),
        vec![("env".to_string(), "GITHUB_TOKEN".to_string())]
    );
    assert_eq!(fixture.secrets.get(&id, "env", "GITHUB_TOKEN").unwrap().as_deref(), Some(secret.as_str()));

    // The preview masks it...
    let preview = fixture.preview(&request_for(&["opencode"]));
    let single = only_preview(&preview);
    let proposed = single.proposed_json.clone().unwrap();
    assert!(
        !proposed.contains(&secret),
        "the preview must not echo a credential: {proposed}"
    );
    assert!(proposed.contains(REDACTED));
    let diff: String = single.diff.iter().map(|line| line.text.clone()).collect::<Vec<_>>().join("\n");
    assert!(!diff.contains(&secret));
    assert!(single.warnings.iter().any(|warning| warning.contains(REDACTED)));

    // ...but the harness file receives the real value, verified after the write.
    let report = fixture.apply(&request_for(&["opencode"]));
    assert_eq!(only_result(&report).status, "success");
    let document = fixture.read_json(&path);
    assert_eq!(
        document["mcp"]["github"]["environment"]["GITHUB_TOKEN"],
        json!(secret)
    );

    // Clean up the credential created by this test.
    mcp_registry::delete_server(&fixture.db, &fixture.secrets, &created.id).unwrap();
    assert!(fixture.secrets.get(&id, "env", "GITHUB_TOKEN").unwrap().is_none());
    assert!(fixture.db.list_secret_refs(&id).unwrap().is_empty());
}

/// Same contract for a credential that belongs to an HTTP header instead of an
/// environment variable: the header slot must resolve *and* mask exactly like
/// the environment slot.
#[cfg(windows)]
#[test]
fn a_header_credential_is_masked_in_the_preview_and_written_to_the_file() {
    let mut fixture = Fixture::new();
    let path = fixture.point_at("opencode", "opencode/opencode.json");
    fixture.write_json(&path, &json!({ "mcp": {} }));

    let id = unique_id("uum-it-hdr");
    let secret = format!("Bearer {}", unique_id("value"));
    let mut definition = input("search", &["opencode"]);
    definition.id = Some(id.clone());
    definition.transport = TransportType::StreamableHttp;
    definition.command = None;
    definition.args = vec![];
    definition.url = Some("https://mcp.example.test/mcp".into());
    definition.headers.insert(
        "Authorization".into(),
        VarValue {
            value: secret.clone(),
            kind: VarKind::Secret,
        },
    );

    let created = match mcp_registry::create_server(
        &fixture.db,
        &fixture.secrets,
        &fixture.adapters,
        &definition,
    ) {
        Ok(created) => created,
        Err(error) => {
            assert_eq!(
                error.code, "SECRET_STORE_ERROR",
                "unexpected failure: {error:?}"
            );
            eprintln!(
                "skipping: the Windows Credential Manager is unavailable ({})",
                error.message
            );
            return;
        }
    };

    assert_eq!(
        fixture.db.list_secret_refs(&id).unwrap(),
        vec![("headers".to_string(), "Authorization".to_string())]
    );

    // The preview shows the header but never the credential value.
    let preview = fixture.preview(&request_for(&["opencode"]));
    let single = only_preview(&preview);
    let proposed = single.proposed_json.clone().unwrap();
    assert!(proposed.contains("Authorization"), "{proposed}");
    assert!(
        !proposed.contains(&secret),
        "the preview must not echo a header credential: {proposed}"
    );
    assert!(proposed.contains(REDACTED));

    // The harness file still receives the real value.
    let report = fixture.apply(&request_for(&["opencode"]));
    assert_eq!(only_result(&report).status, "success");
    let document = fixture.read_json(&path);
    assert_eq!(
        document["mcp"]["search"]["headers"]["Authorization"],
        json!(secret)
    );

    mcp_registry::delete_server(&fixture.db, &fixture.secrets, &created.id).unwrap();
}

/// The sample document shipped in `samples/` must keep working: if the registry
/// file format changes, this test fails before users hit an import error.
#[test]
fn the_sample_registry_file_imports_cleanly() {
    let fixture = Fixture::new();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repository root")
        .join("samples")
        .join("registry.sample.json");
    assert!(path.is_file(), "missing sample: {}", path.display());

    let import = |mode: &str| {
        mcp_registry::import_registry(
            &fixture.db,
            &fixture.secrets,
            &RegistryImportRequest {
                path: path.to_string_lossy().into_owned(),
                mode: mode.into(),
            },
        )
        .expect("the shipped sample must import")
    };

    let first = import("merge");
    assert_eq!(first.imported, 4, "warnings: {:?}", first.warnings);
    assert!(
        !first.warnings.iter().any(|warning| warning.contains("secret")),
        "the sample must not require stored credentials: {:?}",
        first.warnings
    );

    // Re-importing the same document updates in place instead of duplicating.
    let second = import("merge");
    assert_eq!(second.updated, 4);
    assert_eq!(fixture.db.list_servers().unwrap().len(), 4);

    let remote = fixture.db.find_server_by_name("github").unwrap().unwrap();
    assert_eq!(remote.headers["Authorization"].kind, VarKind::EnvRef);
    assert_eq!(remote.headers["Authorization"].value, "{env:GITHUB_TOKEN}");
}
