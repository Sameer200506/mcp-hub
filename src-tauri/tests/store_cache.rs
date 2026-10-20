//! Local store cache: registry entries, skill catalog, installed skills and the
//! store-specific settings blob. No network access is used here.

use universal_mcp_manager_lib::database::Db;
use universal_mcp_manager_lib::models::{
    InstalledSkill, SkillEntry, SkillFile, StoreEntry, StoreEnvVar, StorePackage, StoreRefreshResult,
};
use universal_mcp_manager_lib::services::{mcp_store, skill_store};

fn entry(id: &str, name: &str, description: &str, version: &str) -> StoreEntry {
    StoreEntry {
        id: id.into(),
        source: mcp_store::OFFICIAL_SOURCE.into(),
        name: name.into(),
        title: Some(name.to_string()),
        description: description.into(),
        version: Some(version.into()),
        homepage: Some("https://example.test".into()),
        repository: Some("https://github.com/acme/example".into()),
        packages: vec![StorePackage {
            registry_type: "npm".into(),
            identifier: "@acme/example".into(),
            version: Some(version.into()),
            runtime_hint: None,
            transport_type: "stdio".into(),
            environment_variables: vec![StoreEnvVar {
                name: "ACME_KEY".into(),
                description: "API key".into(),
                is_required: true,
                is_secret: true,
            }],
        }],
        remotes: vec![],
        fetched_at: "2026-01-01T00:00:00Z".into(),
    }
}

fn skill(id: &str, name: &str) -> SkillEntry {
    let source_id = id.split('/').next().unwrap_or("openclaw").to_string();
    SkillEntry {
        id: id.into(),
        source_id,
        source_label: "OpenClaw skills".into(),
        name: name.into(),
        description: "A cached test skill.".into(),
        path: format!("skills/{name}"),
        files: vec![SkillFile {
            path: "SKILL.md".into(),
            url: "https://raw.githubusercontent.test/SKILL.md".into(),
        }],
        fetched_at: "2026-01-01T00:00:00Z".into(),
    }
}

#[test]
fn store_catalog_round_trips_and_searches() {
    let db = Db::open_in_memory().expect("in-memory database");
    db.upsert_store_entry(&entry(
        "official:acme/search",
        "acme/search",
        "A GitHub search server.",
        "1.0.0",
    ))
    .unwrap();
    db.upsert_store_entry(&entry(
        "official:acme/files",
        "acme/files",
        "A filesystem server.",
        "2.0.0",
    ))
    .unwrap();

    assert_eq!(db.count_store_entries(None).unwrap(), 2);
    assert_eq!(db.count_store_entries(Some("github")).unwrap(), 1);

    let page = mcp_store::catalog_page(&db, Some("search".into()), None, None).unwrap();
    assert_eq!(page.total, 1);
    assert_eq!(page.entries[0].name, "acme/search");
    assert_eq!(page.entries[0].packages.len(), 1);
    assert_eq!(page.entries[0].packages[0].environment_variables[0].name, "ACME_KEY");
    assert!(page.last_refreshed_at.is_some());

    // Upsert replaces the row rather than duplicating it.
    let mut updated = entry("official:acme/search", "acme/search", "Updated description.", "1.1.0");
    updated.packages.clear();
    db.upsert_store_entry(&updated).unwrap();
    assert_eq!(db.count_store_entries(None).unwrap(), 2);
    let cached = db.get_store_entry("official:acme/search").unwrap();
    assert_eq!(cached.version.as_deref(), Some("1.1.0"));
    assert!(cached.packages.is_empty());

    // Cursor persistence powers "Load more".
    db.set_store_meta("store.cursor.official", Some("cursor-1")).unwrap();
    assert_eq!(
        db.get_store_meta("store.cursor.official").unwrap().as_deref(),
        Some("cursor-1")
    );
    db.set_store_meta("store.cursor.official", None).unwrap();
    assert!(db.get_store_meta("store.cursor.official").unwrap().is_none());

    db.clear_store_source(mcp_store::OFFICIAL_SOURCE).unwrap();
    assert_eq!(db.count_store_entries(None).unwrap(), 0);
}

#[test]
fn skill_catalog_replaces_stale_entries_and_finds_installed_copies() {
    let db = Db::open_in_memory().expect("in-memory database");
    db.replace_source_skills("openclaw", &[skill("openclaw/onepassword", "onepassword")])
        .unwrap();
    db.replace_source_skills("anthropic", &[skill("anthropic/pdf", "pdf")])
        .unwrap();

    assert_eq!(db.list_skill_entries(None, None).unwrap().len(), 2);
    assert_eq!(db.list_skill_entries(Some("openclaw"), None).unwrap().len(), 1);
    assert_eq!(db.list_skill_entries(None, Some("pdf")).unwrap().len(), 1);
    assert!(db.get_skill_entry("openclaw/onepassword").is_ok());

    // Refreshing a source removes entries that disappeared upstream.
    db.replace_source_skills("anthropic", &[]).unwrap();
    assert_eq!(db.list_skill_entries(None, None).unwrap().len(), 1);

    let installed = InstalledSkill {
        id: "openclaw/onepassword".into(),
        name: "onepassword".into(),
        description: "A cached test skill.".into(),
        source_id: "openclaw".into(),
        source_path: "skills/onepassword".into(),
        library_path: "C:/data/skills/onepassword".into(),
        deployed_paths: vec!["C:/config/opencode/skills/onepassword".into()],
        installed_at: "2026-01-01T00:00:00Z".into(),
        updated_at: "2026-01-01T00:00:00Z".into(),
    };
    db.upsert_installed_skill(&installed).unwrap();
    let listed = db.list_installed_skills().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].deployed_paths.len(), 1);

    db.delete_installed_skill(&installed.id).unwrap();
    assert!(db.list_installed_skills().unwrap().is_empty());
}

#[test]
fn store_settings_defaults_validate_and_round_trip() {
    let db = Db::open_in_memory().expect("in-memory database");
    let defaults = skill_store::load_settings(&db).unwrap();
    assert!(defaults
        .skill_sources
        .iter()
        .any(|source| source.repo == "openclaw/openclaw"));
    assert!(!defaults.skill_targets.is_empty());
    assert!(!defaults.default_target_ids.is_empty());

    let mut changed = defaults.clone();
    changed.default_target_ids = changed.skill_targets.iter().map(|target| target.id.clone()).collect();
    let saved = skill_store::save_settings(&db, &changed).unwrap();
    let reloaded = skill_store::load_settings(&db).unwrap();
    assert_eq!(saved.default_target_ids, reloaded.default_target_ids);
    assert_eq!(saved.skill_targets.len(), reloaded.skill_targets.len());

    let mut unsafe_settings = defaults.clone();
    unsafe_settings.skill_targets[0].path = "relative/skills".into();
    assert!(skill_store::save_settings(&db, &unsafe_settings).is_err());
}

#[test]
fn refresh_result_shape_is_serializable_for_the_frontend() {
    let result = StoreRefreshResult {
        fetched: 100,
        added: 42,
        total: 42,
        has_more: true,
        warnings: vec!["one page failed".into()],
    };
    let json = serde_json::to_value(&result).unwrap();
    assert_eq!(json["hasMore"], true);
    assert_eq!(json["added"], 42);
}
