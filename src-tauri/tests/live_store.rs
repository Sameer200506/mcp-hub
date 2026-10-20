//! Manual, network-dependent smoke test for the store.
//!
//! Run explicitly:
//! `cargo test --manifest-path src-tauri/Cargo.toml --test live_store -- --ignored --nocapture`
//!
//! It contacts registry.modelcontextprotocol.io, api.github.com and
//! raw.githubusercontent.com, so it is never part of the default test run.

use std::path::Path;

use universal_mcp_manager_lib::database::Db;
use universal_mcp_manager_lib::models::{
    SkillInstallRequest, SkillSource, SkillTarget, StoreSettings,
};
use universal_mcp_manager_lib::services::{mcp_store, skill_store};

#[test]
#[ignore = "network: contacts the official MCP registry and GitHub"]
fn live_registry_page_parses_and_prefills() {
    let db = Db::open_in_memory().expect("in-memory database");
    let result = mcp_store::refresh_catalog(&db, 1, true).expect("registry refresh");
    println!("registry page: fetched={} added={} total={}", result.fetched, result.added, result.total);
    assert!(result.added > 0, "the registry should return at least one latest server");

    let page = mcp_store::catalog_page(&db, None, Some(500), None).expect("catalog page");
    let with_package = page
        .entries
        .iter()
        .find(|entry| !entry.packages.is_empty())
        .expect("at least one npm/pypi package");
    let input = mcp_store::entry_prefill(with_package);
    println!(
        "package entry: {} -> name={} command={:?} args={:?}",
        with_package.name, input.name, input.command, input.args
    );
    assert!(input.command.is_some());
    assert!(!input.name.is_empty());
    assert!(!input.targets.iter().any(|target| target == "unset"));

    if let Some(remote) = page
        .entries
        .iter()
        .find(|entry| entry.remotes.iter().any(|remote| !remote.url.is_empty()))
    {
        let input = mcp_store::entry_prefill(remote);
        println!(
            "remote entry: {} -> transport={:?} url={:?}",
            remote.name, input.transport, input.url
        );
        assert!(input.url.is_some());
    }
}

#[test]
#[ignore = "network: contacts api.github.com and raw.githubusercontent.com"]
fn live_skill_refresh_install_and_uninstall() {
    let temp = tempfile::tempdir().expect("temp dir");
    let db = Db::open_in_memory().expect("in-memory database");

    let mut settings = StoreSettings::defaults();
    settings.skill_sources = vec![SkillSource {
        id: "anthropic".into(),
        label: "Anthropic skills".into(),
        repo: "anthropics/skills".into(),
        branch: Some("main".into()),
        path: "skills".into(),
    }];
    settings.skill_targets = vec![SkillTarget {
        id: "temp-target".into(),
        label: "Temp harness".into(),
        path: temp.path().join("harness-skills").to_string_lossy().into_owned(),
    }];
    settings.default_target_ids = vec!["temp-target".into()];
    skill_store::save_settings(&db, &settings).expect("settings save");

    let result = skill_store::refresh_skills(&db, Some("anthropic".into())).expect("skills refresh");
    println!("skills refresh: sources={} skills={} warnings={:?}", result.sources, result.skills, result.warnings);
    assert!(result.skills > 0);

    let page = skill_store::skill_catalog(&db, Some("anthropic".into()), None).expect("skill catalog");
    let described = page
        .entries
        .iter()
        .filter(|entry| !entry.description.trim().is_empty())
        .count();
    println!(
        "skill catalog: {} entries, {} with a description",
        page.entries.len(),
        described
    );
    assert!(described > 0, "SKILL.md frontmatter descriptions should be cached");
    let skill = page
        .entries
        .iter()
        .find(|entry| entry.files.iter().any(|file| file.path == "SKILL.md"))
        .expect("a skill with SKILL.md");
    println!("installing: {} ({} files) from {}", skill.name, skill.files.len(), skill.path);

    let installed = skill_store::install_skill(
        &db,
        temp.path(),
        &SkillInstallRequest {
            skill_id: skill.id.clone(),
            target_ids: vec!["temp-target".into()],
            overwrite: false,
        },
    )
    .expect("skill install");
    println!(
        "installed: library={} deployed={:?} skipped={:?}",
        installed.library_path, installed.deployed_paths, installed.skipped
    );
    assert!(Path::new(&installed.library_path).join("SKILL.md").exists());
    assert_eq!(installed.deployed_paths.len(), 1);
    assert!(Path::new(&installed.deployed_paths[0]).join("SKILL.md").exists());
    assert_eq!(db.list_installed_skills().expect("installed list").len(), 1);

    // Installing again without overwrite is refused.
    let again = skill_store::install_skill(
        &db,
        temp.path(),
        &SkillInstallRequest {
            skill_id: skill.id.clone(),
            target_ids: vec![],
            overwrite: false,
        },
    );
    assert!(again.is_err());
    skill_store::uninstall_skill(&db, &skill.id, true).expect("uninstall");
    assert!(!Path::new(&installed.library_path).exists());
    assert!(!Path::new(&installed.deployed_paths[0]).exists());
    assert!(db.list_installed_skills().expect("installed list").is_empty());
}
