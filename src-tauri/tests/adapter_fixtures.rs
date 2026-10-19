//! Adapter fixtures: the exact JSON each harness receives.
//!
//! Every expectation in this file is taken from the vendor documentation quoted
//! at the top of the corresponding adapter module. If a harness changes its
//! schema, these tests fail before a user's configuration is rewritten.

mod common;

use common::{remote_server, stdio_server, var_literal, Fixture};
use serde_json::{json, Value};
use universal_mcp_manager_lib::models::{AdapterStatus, McpServer, TransportType};
use universal_mcp_manager_lib::services::config_sync::encode_for_preview;

fn encode(fixture: &Fixture, harness_id: &str, server: &McpServer) -> Value {
    let ctx = fixture.ctx();
    encode_for_preview(&ctx, fixture.adapter(harness_id), None, server).expect("encode definition")
}

fn keys(value: &Value) -> Vec<String> {
    let mut names: Vec<String> = value
        .as_object()
        .expect("entry must be a JSON object")
        .keys()
        .cloned()
        .collect();
    names.sort();
    names
}

// --------------------------------------------------------------------- OpenCode

#[test]
fn opencode_local_entry_uses_the_documented_fields() {
    let fixture = Fixture::new();
    let mut server = stdio_server("filesystem", &["opencode"]);
    server
        .env
        .insert("ROOT".into(), var_literal("C:/data"));
    server.cwd = Some("C:/data".into());

    let entry = encode(&fixture, "opencode", &server);

    assert_eq!(entry["type"], json!("local"));
    assert_eq!(entry["command"], json!(["npx", "-y", "@example/filesystem"]));
    assert_eq!(entry["environment"], json!({ "ROOT": "C:/data" }));
    assert_eq!(entry["cwd"], json!("C:/data"));
    assert_eq!(entry["enabled"], json!(true));
    assert_eq!(keys(&entry), ["command", "cwd", "enabled", "environment", "type"]);
}

#[test]
fn opencode_remote_entry_uses_url_and_headers() {
    let fixture = Fixture::new();
    let mut server = remote_server("search", "https://mcp.example.test/mcp", &["opencode"]);
    server
        .headers
        .insert("Authorization".into(), var_literal("{env:SEARCH_TOKEN}"));

    let entry = encode(&fixture, "opencode", &server);

    assert_eq!(entry["type"], json!("remote"));
    assert_eq!(entry["url"], json!("https://mcp.example.test/mcp"));
    assert_eq!(entry["headers"], json!({ "Authorization": "{env:SEARCH_TOKEN}" }));
    assert_eq!(entry["enabled"], json!(true));
    assert!(entry.get("serverUrl").is_none(), "OpenCode uses `url`");
    assert!(entry.get("command").is_none());
}

#[test]
fn opencode_declares_sse_as_unsupported() {
    let fixture = Fixture::new();
    let opencode = fixture.adapter("opencode");
    assert!(opencode.supports_transport(TransportType::Stdio));
    assert!(opencode.supports_transport(TransportType::StreamableHttp));
    assert!(!opencode.supports_transport(TransportType::Sse));
}

// ------------------------------------------------------------------------ Cline

#[test]
fn cline_local_entry_matches_the_settings_file_schema() {
    let fixture = Fixture::new();
    let mut server = stdio_server("filesystem", &["cline"]);
    server
        .env
        .insert("ROOT".into(), var_literal("C:/data"));
    // Cline has no working-directory field: the value must be dropped, not written.
    server.cwd = Some("C:/data".into());

    let entry = encode(&fixture, "cline", &server);

    assert_eq!(entry["type"], json!("stdio"));
    assert_eq!(entry["command"], json!("npx"));
    assert_eq!(entry["args"], json!(["-y", "@example/filesystem"]));
    assert_eq!(entry["env"], json!({ "ROOT": "C:/data" }));
    assert_eq!(entry["disabled"], json!(false));
    assert_eq!(keys(&entry), ["args", "command", "disabled", "env", "type"]);
}

#[test]
fn cline_remote_entries_always_declare_the_transport() {
    let fixture = Fixture::new();
    let mut server = remote_server("search", "https://mcp.example.test/mcp", &["cline"]);

    let entry = encode(&fixture, "cline", &server);
    assert_eq!(entry["type"], json!("streamableHttp"));
    assert_eq!(entry["url"], json!("https://mcp.example.test/mcp"));
    assert_eq!(entry["disabled"], json!(false));

    server.transport = TransportType::Sse;
    let entry = encode(&fixture, "cline", &server);
    assert_eq!(entry["type"], json!("sse"));
    assert!(entry.get("command").is_none());
}

// ------------------------------------------------------------------ Antigravity

#[test]
fn antigravity_local_entry_never_contains_a_type_field() {
    let fixture = Fixture::new();
    let mut server = stdio_server("filesystem", &["antigravity"]);
    server
        .env
        .insert("ROOT".into(), var_literal("C:/data"));

    let entry = encode(&fixture, "antigravity", &server);

    assert!(entry.get("type").is_none(), "Antigravity rejects `type`: {entry}");
    assert_eq!(entry["command"], json!("npx"));
    assert_eq!(entry["args"], json!(["-y", "@example/filesystem"]));
    assert_eq!(entry["env"], json!({ "ROOT": "C:/data" }));
    assert_eq!(entry["disabled"], json!(false));
    assert_eq!(keys(&entry), ["args", "command", "disabled", "env"]);
}

#[test]
fn antigravity_remote_entry_uses_server_url_without_type() {
    let fixture = Fixture::new();
    let mut server = remote_server("search", "https://mcp.example.test/mcp", &["antigravity"]);
    server
        .headers
        .insert("Authorization".into(), var_literal("{env:SEARCH_TOKEN}"));

    let entry = encode(&fixture, "antigravity", &server);

    assert!(entry.get("type").is_none());
    assert!(entry.get("url").is_none(), "`url` is not an Antigravity field");
    assert_eq!(entry["serverUrl"], json!("https://mcp.example.test/mcp"));
    assert_eq!(entry["headers"], json!({ "Authorization": "{env:SEARCH_TOKEN}" }));
    assert_eq!(entry["disabled"], json!(false));
}

// ----------------------------------------------------------- enablement flags

#[test]
fn disabled_definitions_follow_each_harness_convention() {
    let fixture = Fixture::new();

    let mut opencode_server = stdio_server("filesystem", &["opencode"]);
    opencode_server.enabled = false;
    let entry = encode(&fixture, "opencode", &opencode_server);
    assert_eq!(entry["enabled"], json!(false));
    assert!(
        entry.get("disabled").is_none(),
        "OpenCode only understands `enabled`"
    );

    let mut cline_server = stdio_server("filesystem", &["cline"]);
    cline_server.enabled = false;
    let entry = encode(&fixture, "cline", &cline_server);
    assert_eq!(entry["disabled"], json!(true));
    assert!(entry.get("enabled").is_none(), "Cline only understands `disabled`");
}

// ------------------------------------------------------------- generic profile

#[test]
fn generic_profile_is_pending_verification_until_the_user_confirms_it() {
    let mut fixture = Fixture::new();
    fixture.add_generic_profile("custom", "custom/mcp.json", false);

    let adapter = fixture.adapter("custom");
    assert_eq!(adapter.adapter_status(), AdapterStatus::PendingVerification);
    let notes = adapter.notes();
    assert!(
        notes.iter().any(|note| note.contains("not marked as verified")),
        "an unverified profile must warn in the UI: {notes:?}"
    );
}

#[test]
fn generic_profile_uses_its_configured_mapping() {
    let mut fixture = Fixture::new();
    let path = fixture.add_generic_profile("custom", "custom/mcp.json", true);
    let server = stdio_server("filesystem", &["custom"]);

    let entry = encode(&fixture, "custom", &server);

    assert_eq!(entry["command"], json!("npx"));
    assert_eq!(entry["args"], json!(["-y", "@example/filesystem"]));
    assert_eq!(entry["disabled"], json!(false));
    assert!(path.exists() == false, "nothing is written before a synchronization");
}

#[test]
fn generic_profile_refuses_to_write_outside_its_configured_file() {
    let mut fixture = Fixture::new();
    fixture.add_generic_profile("custom", "custom/mcp.json", true);
    let escape = fixture.path("elsewhere/mcp.json");
    let adapter = fixture.adapter("custom");

    let error = adapter
        .write_config_safely(&escape, "{}", None)
        .expect_err("writes outside the configured file must be refused");

    assert_eq!(error.code, "PERMISSION_DENIED");
    assert!(!escape.exists());
}

#[test]
fn a_profile_with_blank_field_names_never_writes_empty_json_keys() {
    let mut fixture = Fixture::new();
    fixture.add_generic_profile("custom", "custom/mcp.json", true);
    // The settings form lets a user clear any optional field name; an empty
    // name means "this harness has no such field", not an empty JSON key.
    fixture.mutate_profile("custom", |profile| {
        profile.transport_mapping.args_field = String::new();
        profile.transport_mapping.env_field = String::new();
        profile.transport_mapping.headers_field = String::new();
        profile.transport_mapping.cwd_field = String::new();
        profile.transport_mapping.disabled_field = String::new();
    });

    let mut server = stdio_server("filesystem", &["custom"]);
    server.env.insert("ROOT".into(), var_literal("C:/data"));
    server.cwd = Some("C:/data".into());

    let entry = encode(&fixture, "custom", &server);
    assert_eq!(keys(&entry), ["command", "type"], "{entry}");
    assert_eq!(entry["command"], json!("npx"));
    assert_eq!(entry["type"], json!("stdio"));

    let remote = remote_server("search", "https://mcp.example.test/mcp", &["custom"]);
    let entry = encode(&fixture, "custom", &remote);
    assert_eq!(keys(&entry), ["type", "url"], "{entry}");
    assert_eq!(entry["url"], json!("https://mcp.example.test/mcp"));
}

#[test]
fn a_profile_without_a_command_field_is_rejected_instead_of_encoding_a_broken_entry() {
    let mut fixture = Fixture::new();
    fixture.add_generic_profile("custom", "custom/mcp.json", true);
    fixture.mutate_profile("custom", |profile| {
        profile.transport_mapping.command_field = String::new();
    });

    let ctx = fixture.ctx();
    let server = stdio_server("filesystem", &["custom"]);
    let error = encode_for_preview(&ctx, fixture.adapter("custom"), None, &server)
        .expect_err("a mapping without a command field must not encode an entry");

    assert_eq!(error.code, "VALIDATION_FAILED");
    assert!(error.message.contains("filesystem"), "{error:?}");
}
