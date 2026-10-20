//! Agent skills store: GitHub-hosted skill folders (`<path>/<skill>/SKILL.md`)
//! that can be installed into the app library and deployed to harness skill
//! directories (opencode, Claude Code or a custom folder).
//!
//! Network access only happens on explicit refresh/install actions. Every
//! downloaded path is validated before it is written, and deployments into a
//! harness folder are refused unless the caller opted into overwriting.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::database::{now_rfc3339, Db};
use crate::errors::{AppError, AppResult};
use crate::models::{
    InstalledSkill, SkillCatalogPage, SkillEntry, SkillFile, SkillInstallRequest,
    SkillInstallResult, SkillRefreshResult, SkillSource, StoreSettings,
};

pub const STORE_SETTINGS_KEY: &str = "store_settings";
const GITHUB_API: &str = "https://api.github.com";
const RAW_HOST: &str = "https://raw.githubusercontent.com";
const USER_AGENT: &str = concat!("UniversalMcpManager/", env!("CARGO_PKG_VERSION"));
const API_TIMEOUT: Duration = Duration::from_secs(25);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SKILL_BYTES: usize = 12 * 1024 * 1024;
const MAX_SKILL_FILES: usize = 250;

// --------------------------------------------------------------- wire format

#[derive(Debug, Deserialize, Default)]
struct GitTree {
    #[serde(default)]
    tree: Vec<GitTreeEntry>,
    #[serde(default)]
    truncated: bool,
}

#[derive(Debug, Deserialize, Default)]
struct GitTreeEntry {
    #[serde(default)]
    path: String,
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    sha: String,
}

// ----------------------------------------------------------------- settings

pub fn load_settings(db: &Db) -> AppResult<StoreSettings> {
    Ok(db
        .get_setting_json::<StoreSettings>(STORE_SETTINGS_KEY)?
        .unwrap_or_else(StoreSettings::defaults))
}

pub fn save_settings(db: &Db, settings: &StoreSettings) -> AppResult<StoreSettings> {
    validate_settings(settings)?;
    db.set_setting_json(STORE_SETTINGS_KEY, settings)?;
    Ok(settings.clone())
}

fn validate_settings(settings: &StoreSettings) -> AppResult<()> {
    for source in &settings.skill_sources {
        if !is_valid_repo(&source.repo) {
            return Err(AppError::validation(format!(
                "Skill source '{}' must use the owner/repo form.",
                source.label
            )));
        }
        if source.id.trim().is_empty()
            || !source
                .id
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
        {
            return Err(AppError::validation(
                "Skill source ids may only contain letters, digits, dot, dash or underscore.",
            ));
        }
        if source.path.contains("..") || source.path.starts_with('/') || source.path.contains('\\') {
            return Err(AppError::validation(format!(
                "Skill source '{}' has an unsafe path. Use a repository-relative folder.",
                source.label
            )));
        }
        if let Some(branch) = &source.branch {
            if branch.trim().is_empty()
                || branch.contains("..")
                || branch.contains('\\')
                || branch.chars().any(char::is_whitespace)
            {
                return Err(AppError::validation(format!(
                    "Skill source '{}' has an unsafe branch name.",
                    source.label
                )));
            }
        }
    }

    let mut seen = std::collections::HashSet::new();
    for target in &settings.skill_targets {
        if target.id.trim().is_empty() || !seen.insert(target.id.trim().to_string()) {
            return Err(AppError::validation(
                "Every skill target needs a unique id (for example 'opencode' or 'claude').",
            ));
        }
        if target.path.trim().is_empty() || !Path::new(target.path.trim()).is_absolute() {
            return Err(AppError::validation(format!(
                "Skill target '{}' must be an absolute directory.",
                target.label
            )));
        }
    }
    for id in &settings.default_target_ids {
        if !settings.skill_targets.iter().any(|target| &target.id == id) {
            return Err(AppError::validation(format!(
                "The default skill target '{id}' is not in the target list."
            )));
        }
    }
    Ok(())
}

fn is_valid_repo(repo: &str) -> bool {
    let Some((owner, name)) = repo.split_once('/') else {
        return false;
    };
    if owner.is_empty() || name.is_empty() || name.contains('/') {
        return false;
    }
    owner
        .chars()
        .chain(name.chars())
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
}

// ------------------------------------------------------------------ refresh

/// Refresh the cached skills for one source, or for every configured source.
pub fn refresh_skills(db: &Db, source_id: Option<String>) -> AppResult<SkillRefreshResult> {
    let settings = load_settings(db)?;
    let single = source_id.is_some();
    let sources: Vec<SkillSource> = match source_id {
        Some(id) => settings
            .skill_sources
            .iter()
            .filter(|source| source.id == id)
            .cloned()
            .collect(),
        None => settings.skill_sources.clone(),
    };
    if sources.is_empty() {
        return Err(AppError::validation(
            "No skill source matches that id. Open the store settings and add a source.",
        ));
    }
    if db.get_setting_json::<StoreSettings>(STORE_SETTINGS_KEY)?.is_none() {
        db.set_setting_json(STORE_SETTINGS_KEY, &settings)?;
    }

    let mut warnings = Vec::new();
    let mut sources_refreshed = 0usize;
    let mut skills_found = 0usize;
    for source in sources {
        let mut source_warnings = Vec::new();
        match list_source_skills(&source, &mut source_warnings) {
            Ok(entries) => {
                skills_found += entries.len();
                sources_refreshed += 1;
                warnings.extend(source_warnings);
                db.replace_source_skills(&source.id, &entries)?;
            }
            Err(error) => {
                if single {
                    return Err(error);
                }
                warnings.push(format!("{}: {}", source.label, error.message));
            }
        }
    }
    Ok(SkillRefreshResult {
        sources: sources_refreshed,
        skills: skills_found,
        warnings,
    })
}

/// Read cached skills, optionally filtered by source and a free-text query.
pub fn skill_catalog(
    db: &Db,
    source_id: Option<String>,
    query: Option<String>,
) -> AppResult<SkillCatalogPage> {
    let clean = query.as_deref().map(str::trim).filter(|value| !value.is_empty());
    let entries = db.list_skill_entries(source_id.as_deref(), clean)?;
    let total = entries.len();
    let last_refreshed_at = entries.iter().map(|entry| entry.fetched_at.clone()).max();
    Ok(SkillCatalogPage {
        entries,
        total,
        last_refreshed_at,
        warnings: Vec::new(),
    })
}

fn list_source_skills(source: &SkillSource, warnings: &mut Vec<String>) -> AppResult<Vec<SkillEntry>> {
    let client = build_client(API_TIMEOUT)?;
    let branch = source
        .branch
        .clone()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "main".to_string());
    let tree = fetch_source_tree(&client, source, &branch)?;
    if tree.truncated {
        warnings.push(format!(
            "{}: GitHub truncated the repository tree; only part of the skills list is available.",
            source.label
        ));
    }

    // Tree paths are relative to the resolved folder (e.g. `skills`), while raw
    // URLs and the recorded source path must be repository-relative.
    let base = source.path.trim().trim_matches('/').to_string();
    let repo_path = |relative: &str| {
        if base.is_empty() {
            relative.to_string()
        } else {
            format!("{base}/{relative}")
        }
    };

    // Every blob named SKILL.md marks a skill root; all blobs below that root
    // belong to the skill (scripts, references, assets, ...).
    let mut roots: BTreeMap<String, Vec<SkillFile>> = BTreeMap::new();
    for entry in &tree.tree {
        if entry.r#type != "blob" {
            continue;
        }
        let normalized = entry.path.replace('\\', "/");
        let file_name = normalized.rsplit('/').next().unwrap_or("");
        if file_name != "SKILL.md" {
            continue;
        }
        let relative_root = normalized
            .trim_end_matches("/SKILL.md")
            .trim_end_matches("SKILL.md")
            .trim_end_matches('/')
            .to_string();
        if relative_root.is_empty() {
            continue;
        }
        let prefix = format!("{relative_root}/");
        let files = tree
            .tree
            .iter()
            .filter(|candidate| {
                candidate.r#type == "blob" && candidate.path.replace('\\', "/").starts_with(&prefix)
            })
            .map(|candidate| SkillFile {
                path: candidate.path[prefix.len()..].to_string(),
                url: raw_url(&source.repo, &branch, &repo_path(&candidate.path)),
            })
            .collect();
        roots.insert(repo_path(&relative_root), files);
    }

    let mut entries = Vec::new();
    for (root, files) in roots {
        let skill_name = root.rsplit('/').next().unwrap_or(&root).to_string();
        let description = files
            .iter()
            .find(|file| file.path == "SKILL.md")
            .and_then(|file| fetch_description(&client, &file.url).ok())
            .unwrap_or_default();
        entries.push(SkillEntry {
            id: format!("{}/{}", source.id, root),
            source_id: source.id.clone(),
            source_label: source.label.clone(),
            name: skill_name,
            description,
            path: root,
            files,
            fetched_at: now_rfc3339(),
        });
    }
    Ok(entries)
}

fn fetch_source_tree(
    client: &reqwest::blocking::Client,
    source: &SkillSource,
    branch: &str,
) -> AppResult<GitTree> {
    let path = source.path.trim().trim_matches('/');
    if path.is_empty() {
        return get_tree(client, &source.repo, branch, true);
    }

    let segments: Vec<&str> = path.split('/').filter(|segment| !segment.is_empty()).collect();
    let mut tree = get_tree(client, &source.repo, branch, false)?;
    let mut sha: Option<String> = None;
    for (index, segment) in segments.iter().enumerate() {
        let node = tree
            .tree
            .iter()
            .find(|entry| entry.r#type == "tree" && entry.path == *segment)
            .ok_or_else(|| {
                AppError::validation(format!(
                    "The repository '{}' has no '{}' directory (branch '{}').",
                    source.repo, path, branch
                ))
            })?;
        sha = Some(node.sha.clone());
        if index + 1 < segments.len() {
            tree = get_tree(client, &source.repo, &node.sha, false)?;
        }
    }
    let sha = sha.ok_or_else(|| AppError::validation(format!("Could not resolve '{}'.", path)))?;
    get_tree(client, &source.repo, &sha, true)
}

fn get_tree(
    client: &reqwest::blocking::Client,
    repo: &str,
    rev: &str,
    recursive: bool,
) -> AppResult<GitTree> {
    let mut url = Url::parse(&format!(
        "{GITHUB_API}/repos/{repo}/git/trees/{}",
        encode_path_segment(rev)
    ))?;
    if recursive {
        url.query_pairs_mut().append_pair("recursive", "1");
    }
    let response = client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .map_err(|error| {
            AppError::io(format!("Could not reach GitHub: {error}")).with_remediation(
                "Check your internet connection and try again. Skills are only fetched when you click Refresh.",
            )
        })?;
    let status = response.status();
    if status == reqwest::StatusCode::FORBIDDEN || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
        return Err(AppError::io("GitHub rate-limited the skill catalog request.").with_remediation(
            "Unauthenticated GitHub API requests are limited to 60 per hour. Wait a few minutes, or keep only the skill sources you need.",
        ));
    }
    let body = response
        .text()
        .map_err(|error| AppError::io(format!("Could not read the GitHub response: {error}")))?;
    if !status.is_success() {
        let snippet: String = body.chars().take(200).collect();
        return Err(AppError::io(format!(
            "GitHub answered HTTP {status} for {repo}{}.",
            if snippet.trim().is_empty() { String::new() } else { format!(": {snippet}") }
        )));
    }
    serde_json::from_str(&body)
        .map_err(|error| AppError::io(format!("GitHub returned an unexpected document: {error}")))
}

fn fetch_description(client: &reqwest::blocking::Client, url: &str) -> AppResult<String> {
    let response = client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .send()
        .map_err(|error| AppError::io(format!("Could not download SKILL.md: {error}")))?;
    if !response.status().is_success() {
        return Err(AppError::io(format!(
            "SKILL.md answered HTTP {}",
            response.status()
        )));
    }
    let text = response
        .text()
        .map_err(|error| AppError::io(format!("Could not read SKILL.md: {error}")))?;
    let (_, description) = parse_frontmatter(&text);
    Ok(description.unwrap_or_default())
}

// ------------------------------------------------------------------ install

/// Download one skill into `<data_dir>/skills/<name>` and deploy it to the
/// selected harness targets. Returns the paths that were written.
pub fn install_skill(
    db: &Db,
    data_dir: &Path,
    request: &SkillInstallRequest,
) -> AppResult<SkillInstallResult> {
    let settings = load_settings(db)?;
    let entry = db.get_skill_entry(&request.skill_id)?;
    let folder = safe_component(&entry.name)?;

    let library_root = data_dir.join("skills");
    std::fs::create_dir_all(&library_root)?;
    let library_path = library_root.join(&folder);
    if library_path.exists() && !request.overwrite {
        return Err(AppError::conflict(format!(
            "Skill '{}' is already installed in the app library. Enable overwrite to replace it.",
            entry.name
        )));
    }

    let temp = library_root.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp)?;
    if let Err(error) = download_skill(&entry, &temp) {
        let _ = std::fs::remove_dir_all(&temp);
        return Err(error);
    }
    if library_path.exists() {
        std::fs::remove_dir_all(&library_path)?;
    }
    if let Err(error) = std::fs::rename(&temp, &library_path) {
        let _ = std::fs::remove_dir_all(&temp);
        return Err(AppError::io(format!(
            "Could not finalize the skill install in the app library: {error}"
        )));
    }

    let existing = db.get_installed_skill(&entry.id)?;
    // Keep previously deployed copies in the record (so uninstall can clean
    // them up), but drop paths that no longer exist on disk.
    let mut deployed_paths: Vec<String> = existing
        .as_ref()
        .map(|skill| skill.deployed_paths.clone())
        .unwrap_or_default()
        .into_iter()
        .filter(|path| Path::new(path).exists())
        .collect();
    let mut skipped = Vec::new();
    let mut warnings = Vec::new();
    for target_id in &request.target_ids {
        let Some(target) = settings.skill_targets.iter().find(|target| &target.id == target_id) else {
            warnings.push(format!("Unknown skill target '{target_id}'."));
            continue;
        };
        let destination = Path::new(&target.path).join(&folder);
        match deploy_dir(&library_path, &destination, request.overwrite) {
            Ok(true) => {
                let deployed = destination.to_string_lossy().to_string();
                if !deployed_paths.contains(&deployed) {
                    deployed_paths.push(deployed);
                }
            }
            Ok(false) => skipped.push(format!("{} ({})", target.label, destination.display())),
            Err(error) => warnings.push(format!("{}: {}", target.label, error.message)),
        }
    }

    let timestamp = now_rfc3339();
    let installed = InstalledSkill {
        id: entry.id.clone(),
        name: entry.name.clone(),
        description: entry.description.clone(),
        source_id: entry.source_id.clone(),
        source_path: entry.path.clone(),
        library_path: library_path.to_string_lossy().to_string(),
        deployed_paths: deployed_paths.clone(),
        installed_at: existing
            .as_ref()
            .map(|skill| skill.installed_at.clone())
            .unwrap_or_else(|| timestamp.clone()),
        updated_at: timestamp,
    };
    db.upsert_installed_skill(&installed)?;

    Ok(SkillInstallResult {
        name: entry.name,
        library_path: library_path.to_string_lossy().to_string(),
        deployed_paths,
        skipped,
        warnings,
    })
}

fn download_skill(entry: &SkillEntry, destination: &Path) -> AppResult<()> {
    if entry.files.is_empty() {
        return Err(AppError::io(
            "This skill has no cached files. Refresh the skills catalog and try again.",
        ));
    }
    if entry.files.len() > MAX_SKILL_FILES {
        return Err(AppError::validation(format!(
            "The skill contains {} files, which is more than the {MAX_SKILL_FILES}-file safety limit.",
            entry.files.len()
        )));
    }
    let client = build_client(DOWNLOAD_TIMEOUT)?;
    let mut total = 0usize;
    for file in &entry.files {
        let relative = safe_relative_path(&file.path)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        ensure_raw_url(&file.url)?;
        let response = client
            .get(&file.url)
            .header("User-Agent", USER_AGENT)
            .send()
            .map_err(|error| {
                AppError::io(format!("Could not download '{}': {error}", file.path))
            })?;        if !response.status().is_success() {
            return Err(AppError::io(format!(
                "Could not download '{}' (HTTP {}).",
                file.path,
                response.status()
            )));
        }
        let bytes = response
            .bytes()
            .map_err(|error| AppError::io(format!("Could not read '{}': {error}", file.path)))?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(AppError::validation(format!(
                "'{}' is larger than the {} MiB per-file limit.",
                file.path,
                MAX_FILE_BYTES / (1024 * 1024)
            )));
        }
        total += bytes.len();
        if total > MAX_SKILL_BYTES {
            return Err(AppError::validation(format!(
                "The skill exceeds the {} MiB total size limit.",
                MAX_SKILL_BYTES / (1024 * 1024)
            )));
        }
        let target = destination.join(&relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&target, &bytes)?;
    }
    Ok(())
}

fn deploy_dir(source: &Path, destination: &Path, overwrite: bool) -> AppResult<bool> {
    if destination.exists() {
        if !overwrite {
            return Ok(false);
        }
        std::fs::remove_dir_all(destination)?;
    }
    copy_dir(source, destination)?;
    Ok(true)
}

fn copy_dir(source: &Path, destination: &Path) -> AppResult<()> {
    std::fs::create_dir_all(destination)?;
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &target)?;
        }
        // Symlinks are intentionally skipped: a downloaded skill must never
        // place links into a harness configuration directory.
    }
    Ok(())
}

/// Uninstall a skill from the library and, when requested, from the harness
/// folders that were recorded at install time (inside configured targets only).
pub fn uninstall_skill(db: &Db, skill_id: &str, remove_deployed: bool) -> AppResult<()> {
    let settings = load_settings(db)?;
    let installed = db
        .get_installed_skill(skill_id)?
        .ok_or_else(|| AppError::not_found(format!("Skill {skill_id} is not installed.")))?;

    let library = Path::new(&installed.library_path);
    if library.exists() {
        std::fs::remove_dir_all(library)?;
    }
    if remove_deployed {
        for deployed in &installed.deployed_paths {
            let deployed = Path::new(deployed);
            let allowed = settings
                .skill_targets
                .iter()
                .any(|target| path_is_inside(deployed, Path::new(&target.path)));
            if allowed && deployed.exists() {
                std::fs::remove_dir_all(deployed)?;
            }
        }
    }
    db.delete_installed_skill(skill_id)?;
    Ok(())
}

pub fn path_is_inside(child: &Path, parent: &Path) -> bool {
    if cfg!(windows) {
        let normalize = |path: &Path| -> String {
            let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
            resolved.to_string_lossy().replace('/', "\\").to_lowercase()
        };
        let child = normalize(child);
        let parent = normalize(parent);
        if child == parent {
            return false;
        }
        let prefix = if parent.ends_with('\\') {
            parent
        } else {
            format!("{parent}\\")
        };
        child.starts_with(&prefix)
    } else {
        let child = std::fs::canonicalize(child).unwrap_or_else(|_| child.to_path_buf());
        let parent = std::fs::canonicalize(parent).unwrap_or_else(|_| parent.to_path_buf());
        child != parent && child.starts_with(&parent)
    }
}

// ------------------------------------------------------------------ helpers

fn build_client(timeout: Duration) -> AppResult<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|error| AppError::io(format!("Could not create the store HTTP client: {error}")))
}

fn encode_path_segment(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        let character = byte as char;
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.' | '~') {
            out.push(character);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn raw_url(repo: &str, branch: &str, path: &str) -> String {
    let encoded_branch = encode_path_segment(branch);
    let encoded_path = path
        .split('/')
        .map(encode_path_segment)
        .collect::<Vec<_>>()
        .join("/");
    format!("{RAW_HOST}/{repo}/{encoded_branch}/{encoded_path}")
}

/// Skill downloads may only target the GitHub raw-content host. This guards
/// against a tampered local cache pointing the downloader somewhere else.
fn ensure_raw_url(url: &str) -> AppResult<()> {
    let parsed = Url::parse(url)
        .map_err(|_| AppError::validation("The skill contains an invalid download URL."))?;
    if parsed.scheme() != "https" || parsed.host_str() != Some("raw.githubusercontent.com") {
        return Err(AppError::validation(
            "Skill downloads must come from raw.githubusercontent.com over HTTPS.",
        ));
    }
    Ok(())
}

/// Reject absolute paths, `..` traversal, drive/ADS colons and NUL bytes.
pub fn safe_relative_path(relative: &str) -> AppResult<PathBuf> {
    let normalized = relative.replace('\\', "/");
    let mut path = PathBuf::new();
    for part in normalized.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(AppError::validation(format!(
                "The skill contains an unsafe path: {relative}"
            )));
        }
        if part.contains(':') || part.contains('\0') {
            return Err(AppError::validation(format!(
                "The skill contains an unsafe path: {relative}"
            )));
        }
        path.push(part);
    }
    Ok(path)
}

/// Sanitize a skill name into a single directory component.
pub fn safe_component(name: &str) -> AppResult<String> {
    let cleaned: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-') {
                character
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = cleaned.trim_matches(|character| character == '-' || character == '.');
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        return Err(AppError::validation(format!(
            "The skill name '{name}' cannot be used as a folder name."
        )));
    }
    Ok(trimmed.to_string())
}

/// Read `name` / `description` from a SKILL.md YAML frontmatter block.
///
/// Supports plain, single-quoted, double-quoted and folded/literal block
/// scalars (`description: >` / `description: |`), which are all common in
/// published skills. Multi-line blocks are flattened with single spaces.
pub fn parse_frontmatter(content: &str) -> (Option<String>, Option<String>) {
    let text = content.trim_start_matches('\u{feff}');
    let mut lines = text.lines().peekable();
    if lines.next().map(str::trim) != Some("---") {
        return (None, None);
    }
    let mut name = None;
    let mut description = None;
    while let Some(line) = lines.next() {
        let trimmed = line.trim();
        if trimmed == "---" {
            break;
        }
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, raw_value)) = line.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        let raw_value = raw_value.trim();

        if key == "description" && matches!(raw_value, ">" | "|" | ">-" | "|-" | ">+" | "|+") {
            let mut parts: Vec<String> = Vec::new();
            loop {
                let Some(next) = lines.peek().map(|value| value.to_string()) else {
                    break;
                };
                let next_trimmed = next.trim();
                if next_trimmed == "---" {
                    break;
                }
                let indented = next.starts_with(' ') || next.starts_with('\t');
                if !indented && !next_trimmed.is_empty() {
                    break;
                }
                lines.next();
                if !next_trimmed.is_empty() {
                    parts.push(next_trimmed.to_string());
                }
            }
            if !parts.is_empty() {
                description = Some(parts.join(" "));
            }
            continue;
        }

        let value = unquote(raw_value);
        if value.is_empty() {
            continue;
        }
        match key.as_str() {
            "name" => name = Some(value),
            "description" => description = Some(value),
            _ => {}
        }
    }
    (name, description)
}

fn unquote(value: &str) -> String {
    if value.len() >= 2 {
        let first = value.chars().next().unwrap_or(' ');
        let last = value.chars().last().unwrap_or(' ');
        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            return value[1..value.len() - 1].to_string();
        }
    }
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frontmatter_is_parsed_with_quotes() {
        let content = "---\nname: 1password-cli\ndescription: \"Read secrets from 1Password.\"\n---\n\n# Body\n";
        let (name, description) = parse_frontmatter(content);
        assert_eq!(name.as_deref(), Some("1password-cli"));
        assert_eq!(description.as_deref(), Some("Read secrets from 1Password."));
    }

    #[test]
    fn frontmatter_absent_is_not_an_error() {
        let (name, description) = parse_frontmatter("# Just markdown\n");
        assert!(name.is_none());
        assert!(description.is_none());
    }

    #[test]
    fn folded_frontmatter_descriptions_are_flattened() {
        let content = "---\nname: pdf\ndescription: >\n  Work with PDF files:\n  extract text and fill forms.\n---\n";
        let (name, description) = parse_frontmatter(content);
        assert_eq!(name.as_deref(), Some("pdf"));
        assert_eq!(
            description.as_deref(),
            Some("Work with PDF files: extract text and fill forms.")
        );
    }

    #[test]
    fn unsafe_relative_paths_are_rejected() {
        assert!(safe_relative_path("../escape.md").is_err());
        assert!(safe_relative_path("scripts/../../escape.sh").is_err());
        assert!(safe_relative_path("C:/Windows/file").is_err());
        assert!(safe_relative_path("assets/logo.png").is_ok());
        assert_eq!(
            safe_relative_path("scripts/run.ps1").unwrap(),
            PathBuf::from("scripts").join("run.ps1")
        );
    }

    #[test]
    fn folder_names_are_reduced_to_one_component() {
        assert_eq!(safe_component("1password-cli").unwrap(), "1password-cli");
        assert_eq!(safe_component("My Skill!").unwrap(), "My-Skill");
        assert!(safe_component("..").is_err());
        assert!(safe_component("   ").is_err());
    }

    #[test]
    fn default_settings_are_valid_and_point_at_harness_folders() {
        let settings = StoreSettings::defaults();
        validate_settings(&settings).expect("defaults validate");
        assert!(settings.skill_sources.iter().any(|source| source.repo == "openclaw/openclaw"));
        assert!(settings
            .skill_targets
            .iter()
            .any(|target| target.id == "opencode" && target.path.ends_with("skills")));
        assert_eq!(settings.default_target_ids, vec!["opencode".to_string()]);
    }

    #[test]
    fn settings_validation_rejects_traversal_and_relative_targets() {
        let mut settings = StoreSettings::defaults();
        settings.skill_sources[0].path = "../outside".into();
        assert!(validate_settings(&settings).is_err());

        let mut settings = StoreSettings::defaults();
        settings.skill_targets[0].path = "relative/skills".into();
        assert!(validate_settings(&settings).is_err());

        let mut settings = StoreSettings::defaults();
        settings.skill_sources[0].repo = "not-a-repo".into();
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn deployment_copies_every_file_into_the_target() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("library").join("my-skill");
        std::fs::create_dir_all(source.join("scripts")).unwrap();
        std::fs::write(source.join("SKILL.md"), "---\nname: my-skill\n---\n").unwrap();
        std::fs::write(source.join("scripts").join("run.ps1"), "Write-Host ok").unwrap();

        let destination = temp.path().join("opencode").join("my-skill");
        assert!(deploy_dir(&source, &destination, false).unwrap());
        assert!(destination.join("SKILL.md").exists());
        assert!(destination.join("scripts").join("run.ps1").exists());

        // A second deploy without overwrite is skipped.
        assert!(!deploy_dir(&source, &destination, false).unwrap());
        // With overwrite it is replaced.
        assert!(deploy_dir(&source, &destination, true).unwrap());
    }

    #[test]
    fn paths_inside_targets_are_detected() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("skills");
        let inside = target.join("skill-a");
        let outside = temp.path().join("elsewhere");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        assert!(path_is_inside(&inside, &target));
        assert!(!path_is_inside(&outside, &target));
    }

    #[test]
    fn downloads_are_limited_to_the_raw_github_host() {
        assert!(
            ensure_raw_url("https://raw.githubusercontent.com/acme/skills/main/SKILL.md").is_ok()
        );
        assert!(ensure_raw_url("http://raw.githubusercontent.com/acme/skills/main/SKILL.md").is_err());
        assert!(ensure_raw_url("https://example.test/SKILL.md").is_err());
        assert!(ensure_raw_url("not a url").is_err());
    }
}
