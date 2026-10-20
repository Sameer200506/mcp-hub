//! MCP store: the official MCP registry as a local, searchable catalog.
//!
//! The store never writes to a harness file. `refresh_catalog` caches registry
//! metadata in SQLite (only on explicit user action) and `entry_prefill`
//! converts one entry into the same `McpServerInput` the "Add server" dialog
//! uses, so installation goes through the existing validated registry path.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;
use url::Url;

use crate::database::{now_rfc3339, Db};
use crate::errors::{AppError, AppResult};
use crate::models::{
    McpServerInput, StoreCatalogPage, StoreEntry, StoreEnvVar, StorePackage, StoreRefreshResult,
    StoreRemote, TransportType, VarKind, VarValue,
};

pub const OFFICIAL_SOURCE: &str = "official";
const REGISTRY_URL: &str = "https://registry.modelcontextprotocol.io/v0/servers";
const PAGE_SIZE: u32 = 100;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
const USER_AGENT: &str = concat!("UniversalMcpManager/", env!("CARGO_PKG_VERSION"));
const CURSOR_KEY: &str = "store.cursor.official";

// --------------------------------------------------------------- wire format

#[derive(Debug, Deserialize)]
struct RegistryPage {
    #[serde(default)]
    servers: Vec<RegistryRecord>,
    #[serde(default)]
    metadata: RegistryMetadata,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RegistryMetadata {
    #[serde(default)]
    next_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RegistryRecord {
    server: RegistryServer,
    #[serde(rename = "_meta", default)]
    meta: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryServer {
    name: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    description: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    website_url: Option<String>,
    #[serde(default)]
    repository: Option<RegistryRepository>,
    #[serde(default)]
    packages: Vec<RegistryPackage>,
    #[serde(default)]
    remotes: Vec<RegistryRemote>,
}

#[derive(Debug, Deserialize)]
struct RegistryRepository {
    #[serde(default)]
    url: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RegistryPackage {
    #[serde(default)]
    registry_type: String,
    #[serde(default)]
    identifier: String,
    #[serde(default)]
    version: Option<String>,
    #[serde(default)]
    runtime_hint: Option<String>,
    #[serde(default)]
    transport: Option<RegistryTransport>,
    #[serde(default)]
    environment_variables: Vec<RegistryEnvVar>,
}

#[derive(Debug, Deserialize, Default)]
struct RegistryTransport {
    #[serde(default)]
    r#type: String,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct RegistryEnvVar {
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    is_required: bool,
    #[serde(default)]
    is_secret: bool,
}

#[derive(Debug, Deserialize, Default)]
struct RegistryRemote {
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    url: String,
}

// ------------------------------------------------------------------ refresh

/// Fetch additional pages from the official registry into the local cache.
///
/// `pages` is clamped to 1..=5 (100 records per page). `restart` clears the
/// cache and starts from the first page; otherwise it continues from the saved
/// cursor so the UI can offer "Load more".
pub fn refresh_catalog(db: &Db, pages: u32, restart: bool) -> AppResult<StoreRefreshResult> {
    let pages = pages.clamp(1, 5);
    // A restart starts from the first page but does not clear the cache until
    // at least one page has been fetched successfully.
    let mut cursor = if restart {
        None
    } else {
        db.get_store_meta(CURSOR_KEY)?
    };

    let mut fetched = 0usize;
    let mut added = 0usize;
    let mut warnings = Vec::new();
    let mut collected: Vec<StoreEntry> = Vec::new();

    for page_index in 0..pages {
        let page = match fetch_page(cursor.as_deref()) {
            Ok(page) => page,
            Err(error) if page_index > 0 => {
                warnings.push(error.message);
                break;
            }
            Err(error) => return Err(error),
        };
        let (entries, raw_count) = convert_records(page.servers);
        fetched += raw_count;
        added += entries.len();
        collected.extend(entries);
        cursor = page.metadata.next_cursor;
        if cursor.is_none() {
            break;
        }
        if page_index + 1 < pages {
            // Be polite to the public registry: sequential pages are rare
            // (normally one page per click) and a short pause is enough.
            std::thread::sleep(Duration::from_millis(200));
        }
    }

    let timestamp = now_rfc3339();
    if restart {
        db.clear_store_source(OFFICIAL_SOURCE)?;
    }
    for entry in collected {
        let entry = StoreEntry { fetched_at: timestamp.clone(), ..entry };
        db.upsert_store_entry(&entry)?;
    }

    db.set_store_meta(CURSOR_KEY, cursor.as_deref())?;
    let total = db.count_store_entries(None)?;
    Ok(StoreRefreshResult {
        fetched,
        added,
        total,
        has_more: cursor.is_some(),
        warnings,
    })
}

/// Read the cached catalog, optionally filtered by a free-text query.
pub fn catalog_page(
    db: &Db,
    query: Option<String>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> AppResult<StoreCatalogPage> {
    let clean = query.as_deref().map(str::trim).filter(|value| !value.is_empty());
    let limit = limit.unwrap_or(300).clamp(1, 500);
    let offset = offset.unwrap_or(0);
    let entries = db.list_store_entries(clean, limit, offset)?;
    let total = db.count_store_entries(clean)?;
    let has_more = db.get_store_meta(CURSOR_KEY)?.is_some();
    let last_refreshed_at = db.store_last_refreshed(OFFICIAL_SOURCE)?;
    Ok(StoreCatalogPage {
        entries,
        total,
        source: OFFICIAL_SOURCE.to_string(),
        last_refreshed_at,
        has_more,
    })
}

/// Convert one cached entry into an `McpServerInput` ready for review/install.
pub fn entry_prefill(entry: &StoreEntry) -> McpServerInput {
    let raw_name = entry
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&entry.name);
    let mut input = McpServerInput {
        id: None,
        name: sanitize_server_name(raw_name),
        description: entry.description.clone(),
        transport: TransportType::Stdio,
        command: None,
        args: Vec::new(),
        env: Default::default(),
        headers: Default::default(),
        url: None,
        cwd: None,
        enabled: true,
        targets: Vec::new(),
        tags: vec!["store".to_string()],
        notes: store_notes(entry),
    };

    match pick_package(entry) {
        Some(package) => apply_package(&mut input, package),
        None => {
            if let Some(remote) = entry.remotes.iter().find(|remote| !remote.url.trim().is_empty()) {
                input.transport = if remote.transport_type.eq_ignore_ascii_case("sse") {
                    TransportType::Sse
                } else {
                    TransportType::StreamableHttp
                };
                input.url = Some(remote.url.trim().to_string());
            }
        }
    }

    if input.description.trim().is_empty() {
        input.description = entry.title.clone().unwrap_or_else(|| entry.name.clone());
    }
    input
}

fn store_notes(entry: &StoreEntry) -> String {
    let mut notes = format!("Installed from the official MCP registry ({})", entry.name);
    if let Some(repository) = entry.repository.as_deref().filter(|value| !value.trim().is_empty()) {
        notes.push_str(&format!("\nRepository: {repository}"));
    }
    if let Some(homepage) = entry.homepage.as_deref().filter(|value| !value.trim().is_empty()) {
        notes.push_str(&format!("\nWebsite: {homepage}"));
    }
    notes
}

fn pick_package(entry: &StoreEntry) -> Option<&StorePackage> {
    entry
        .packages
        .iter()
        .filter(|package| !package.identifier.trim().is_empty())
        .filter(|package| {
            package.transport_type.is_empty()
                || package.transport_type.eq_ignore_ascii_case("stdio")
        })
        .min_by_key(|package| package_rank(&package.registry_type))
}

fn package_rank(registry_type: &str) -> u8 {
    match registry_type.to_ascii_lowercase().as_str() {
        "npm" => 0,
        "pypi" => 1,
        "oci" => 2,
        _ => 3,
    }
}

fn apply_package(input: &mut McpServerInput, package: &StorePackage) {
    let registry_type = package.registry_type.to_ascii_lowercase();
    let runtime = package
        .runtime_hint
        .clone()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| match registry_type.as_str() {
            "npm" => "npx".to_string(),
            "pypi" => "uvx".to_string(),
            "oci" => "docker".to_string(),
            _ => "npx".to_string(),
        });
    let runtime_lower = runtime.to_ascii_lowercase();
    let version = package
        .version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    input.command = Some(runtime);
    input.args = match registry_type.as_str() {
        "npm" => {
            let spec = match version {
                Some(version) => format!("{}@{}", package.identifier.trim(), version),
                None => package.identifier.trim().to_string(),
            };
            if runtime_lower.ends_with("npx") {
                vec!["-y".to_string(), spec]
            } else {
                vec![spec]
            }
        }
        "pypi" => match version {
            Some(version) if runtime_lower.ends_with("uvx") => vec![
                "--from".to_string(),
                format!("{}=={}", package.identifier.trim(), version),
                package.identifier.trim().to_string(),
            ],
            _ => vec![package.identifier.trim().to_string()],
        },
        "oci" => {
            let image = match version {
                Some(version) => format!("{}:{}", package.identifier.trim(), version),
                None => package.identifier.trim().to_string(),
            };
            vec!["run".to_string(), "-i".to_string(), "--rm".to_string(), image]
        }
        _ => vec![package.identifier.trim().to_string()],
    };

    for variable in &package.environment_variables {
        let key = variable.name.trim();
        if key.is_empty() {
            continue;
        }
        // Only required or secret values are pre-created. Optional plain
        // variables are left out so an empty value cannot override a harness
        // or server default.
        if !variable.is_required && !variable.is_secret {
            continue;
        }
        let kind = if variable.is_secret { VarKind::Secret } else { VarKind::Literal };
        input
            .env
            .insert(key.to_string(), VarValue { value: String::new(), kind });
    }
}

/// Make a registry-provided title safe to use as a harness key.
pub fn sanitize_server_name(raw: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for character in raw.chars() {
        let keep = character.is_ascii_alphanumeric() || character == '.' || character == '_' || character == '-';
        if keep {
            out.push(character);
            last_dash = character == '-';
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    let trimmed = out.trim_matches(|character| character == '-' || character == '.').to_string();
    let mut name = if trimmed.is_empty() { "store-server".to_string() } else { trimmed };
    if name.len() > 100 {
        name.truncate(100);
        name = name.trim_end_matches('-').to_string();
    }
    if name.is_empty() {
        "store-server".to_string()
    } else {
        name
    }
}

// ------------------------------------------------------------------ plumbing

fn fetch_page(cursor: Option<&str>) -> AppResult<RegistryPage> {
    let mut url = Url::parse(REGISTRY_URL)?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("limit", &PAGE_SIZE.to_string());
        if let Some(cursor) = cursor {
            pairs.append_pair("cursor", cursor);
        }
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(FETCH_TIMEOUT)
        .build()
        .map_err(|error| AppError::io(format!("Could not create the store HTTP client: {error}")))?;
    let response = client
        .get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .send()
        .map_err(|error| {
            AppError::io(format!("Could not reach the MCP registry: {error}")).with_remediation(
                "Check your internet connection and try again. The store only contacts registry.modelcontextprotocol.io when you click Refresh.",
            )
        })?;

    let status = response.status();
    let body = response
        .text()
        .map_err(|error| AppError::io(format!("Could not read the MCP registry response: {error}")))?;
    if !status.is_success() {
        let snippet: String = body.chars().take(300).collect();
        return Err(AppError::io(format!(
            "The MCP registry answered HTTP {status}{}.",
            if snippet.trim().is_empty() { String::new() } else { format!(": {snippet}") }
        ))
        .with_remediation("Try again in a moment; the public registry can be briefly unavailable."));
    }
    serde_json::from_str(&body)
        .map_err(|error| AppError::io(format!("The MCP registry returned an unexpected document: {error}")))
}

fn convert_records(records: Vec<RegistryRecord>) -> (Vec<StoreEntry>, usize) {
    let raw_count = records.len();
    let mut by_name: HashMap<String, StoreEntry> = HashMap::new();
    for record in records {
        if !record.is_latest() {
            continue;
        }
        let name = record.server.name.clone();
        by_name.insert(name, record.server.into_entry());
    }
    let mut entries: Vec<StoreEntry> = by_name.into_values().collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    (entries, raw_count)
}

impl RegistryRecord {
    fn is_latest(&self) -> bool {
        self.meta
            .as_ref()
            .and_then(|meta| meta.get("io.modelcontextprotocol.registry/official"))
            .and_then(|official| official.get("isLatest"))
            .and_then(|value| value.as_bool())
            .unwrap_or(true)
    }
}

impl RegistryServer {
    fn into_entry(self) -> StoreEntry {
        StoreEntry {
            id: format!("{OFFICIAL_SOURCE}:{}", self.name),
            source: OFFICIAL_SOURCE.to_string(),
            name: self.name,
            title: self.title,
            description: self.description,
            version: self.version,
            homepage: self.website_url,
            repository: self
                .repository
                .map(|repository| repository.url)
                .filter(|value| !value.trim().is_empty()),
            packages: self
                .packages
                .into_iter()
                .map(|package| StorePackage {
                    registry_type: package.registry_type,
                    identifier: package.identifier,
                    version: package.version,
                    runtime_hint: package.runtime_hint,
                    transport_type: package
                        .transport
                        .map(|transport| transport.r#type)
                        .filter(|value| !value.is_empty())
                        .unwrap_or_else(|| "stdio".to_string()),
                    environment_variables: package
                        .environment_variables
                        .into_iter()
                        .map(|variable| StoreEnvVar {
                            name: variable.name,
                            description: variable.description,
                            is_required: variable.is_required,
                            is_secret: variable.is_secret,
                        })
                        .collect(),
                })
                .collect(),
            remotes: self
                .remotes
                .into_iter()
                .map(|remote| StoreRemote {
                    transport_type: remote.r#type,
                    url: remote.url,
                })
                .collect(),
            fetched_at: now_rfc3339(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page_json() -> &'static str {
        r#"{
          "servers": [
            {
              "server": {
                "name": "acme/example-server",
                "title": "Example Server",
                "description": "An example MCP server for tests.",
                "version": "1.2.0",
                "websiteUrl": "https://example.com",
                "repository": { "url": "https://github.com/acme/example-server", "source": "github" },
                "packages": [
                  {
                    "registryType": "npm",
                    "identifier": "@acme/example-mcp",
                    "version": "1.2.0",
                    "transport": { "type": "stdio" },
                    "environmentVariables": [
                      { "name": "ACME_API_KEY", "description": "API key", "isRequired": true, "isSecret": true },
                      { "name": "ACME_BASE_URL", "description": "Optional override" }
                    ]
                  }
                ]
              },
              "_meta": { "io.modelcontextprotocol.registry/official": { "isLatest": true } }
            },
            {
              "server": {
                "name": "acme/example-server",
                "title": "Example Server",
                "description": "Old version.",
                "version": "0.9.0",
                "packages": [ { "registryType": "npm", "identifier": "@acme/example-mcp", "version": "0.9.0" } ]
              },
              "_meta": { "io.modelcontextprotocol.registry/official": { "isLatest": false } }
            },
            {
              "server": {
                "name": "acme/remote-server",
                "description": "A remote SSE server.",
                "version": "2.0.0",
                "remotes": [ { "type": "sse", "url": "https://mcp.example.com/sse" } ]
              }
            },
            {
              "server": {
                "name": "acme/python-server",
                "description": "A PyPI server.",
                "version": "3.1.4",
                "packages": [ { "registryType": "pypi", "identifier": "acme-mcp", "version": "3.1.4", "transport": { "type": "stdio" } } ]
              }
            }
          ],
          "metadata": { "nextCursor": "acme/python-server:3.1.4", "count": 4 }
        }"#
    }

    fn parsed_entries() -> Vec<StoreEntry> {
        let page: RegistryPage = serde_json::from_str(page_json()).expect("fixture parses");
        convert_records(page.servers).0
    }

    #[test]
    fn only_latest_versions_are_cached() {
        let entries = parsed_entries();
        assert_eq!(entries.len(), 3);
        let example = entries.iter().find(|entry| entry.name == "acme/example-server").unwrap();
        assert_eq!(example.version.as_deref(), Some("1.2.0"));
    }

    #[test]
    fn npm_package_becomes_an_npx_definition() {
        let entries = parsed_entries();
        let entry = entries.iter().find(|entry| entry.name == "acme/example-server").unwrap();
        let input = entry_prefill(entry);
        assert_eq!(input.name, "Example-Server");
        assert_eq!(input.transport, TransportType::Stdio);
        assert_eq!(input.command.as_deref(), Some("npx"));
        assert_eq!(input.args, vec!["-y".to_string(), "@acme/example-mcp@1.2.0".to_string()]);
        let key = input.env.get("ACME_API_KEY").expect("secret env is pre-created");
        assert_eq!(key.kind, VarKind::Secret);
        assert!(key.value.is_empty());
        assert!(!input.env.contains_key("ACME_BASE_URL"), "optional plain vars stay out");
        assert_eq!(input.tags, vec!["store".to_string()]);
        assert!(input.notes.contains("github.com/acme/example-server"));
    }

    #[test]
    fn remote_sse_entry_maps_to_sse_transport() {
        let entries = parsed_entries();
        let entry = entries.iter().find(|entry| entry.name == "acme/remote-server").unwrap();
        let input = entry_prefill(entry);
        assert_eq!(input.transport, TransportType::Sse);
        assert_eq!(input.url.as_deref(), Some("https://mcp.example.com/sse"));
        assert!(input.command.is_none());
    }

    #[test]
    fn pypi_package_uses_uvx_with_an_exact_version() {
        let entries = parsed_entries();
        let entry = entries.iter().find(|entry| entry.name == "acme/python-server").unwrap();
        let input = entry_prefill(entry);
        assert_eq!(input.command.as_deref(), Some("uvx"));
        assert_eq!(
            input.args,
            vec![
                "--from".to_string(),
                "acme-mcp==3.1.4".to_string(),
                "acme-mcp".to_string()
            ]
        );
    }

    #[test]
    fn names_are_reduced_to_harness_safe_keys() {
        assert_eq!(sanitize_server_name("io.github.acme/My Server!"), "io.github.acme-My-Server");
        assert_eq!(sanitize_server_name("---"), "store-server");
        assert_eq!(sanitize_server_name("Ünïcode"), "n-code");
        assert!(sanitize_server_name(&"a".repeat(200)).len() <= 100);
    }
}
