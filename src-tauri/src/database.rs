//! SQLite persistence for the central registry, sync history, backups and settings.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::errors::{AppError, AppResult};
use crate::models::{
    BackupInfo, HarnessState, InstalledSkill, McpServer, SkillEntry, StoreEntry, SyncHistoryEntry,
    TransportType, VarValue,
};

/// Directory used for the SQLite database, logs and backups.
pub fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("UniversalMcpManager")
}

pub struct Db {
    conn: Mutex<Connection>,
    path: PathBuf,
}

#[derive(Debug, Default)]
pub struct SyncCounts {
    pub successful: usize,
    pub failed: usize,
    pub last_sync_at: Option<String>,
}

impl Db {
    pub fn open(path: &Path) -> AppResult<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        Self::migrate(&conn)?;
        Ok(Self { conn: Mutex::new(conn), path: path.to_path_buf() })
    }

    pub fn open_in_memory() -> AppResult<Self> {
        let conn = Connection::open_in_memory()?;
        Self::migrate(&conn)?;
        Ok(Self { conn: Mutex::new(conn), path: PathBuf::from(":memory:") })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn migrate(conn: &Connection) -> AppResult<()> {
        conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS mcp_servers (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                name_key TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                transport TEXT NOT NULL,
                command TEXT,
                args_json TEXT NOT NULL DEFAULT '[]',
                env_json TEXT NOT NULL DEFAULT '{}',
                headers_json TEXT NOT NULL DEFAULT '{}',
                url TEXT,
                cwd TEXT,
                enabled INTEGER NOT NULL DEFAULT 1,
                targets_json TEXT NOT NULL DEFAULT '[]',
                tags_json TEXT NOT NULL DEFAULT '[]',
                notes TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                last_sync_at TEXT
            );
            CREATE UNIQUE INDEX IF NOT EXISTS idx_mcp_servers_name_key ON mcp_servers (name_key);

            CREATE TABLE IF NOT EXISTS sync_history (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                harness_id TEXT NOT NULL,
                mcp_id TEXT,
                mcp_name TEXT,
                action TEXT NOT NULL,
                status TEXT NOT NULL,
                message TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_sync_history_created ON sync_history (created_at DESC);

            CREATE TABLE IF NOT EXISTS backups (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                harness_id TEXT NOT NULL,
                original_path TEXT NOT NULL,
                backup_path TEXT NOT NULL,
                sha256 TEXT NOT NULL,
                size INTEGER NOT NULL,
                reason TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS app_settings (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS secret_refs (
                id TEXT PRIMARY KEY,
                mcp_id TEXT NOT NULL,
                slot TEXT NOT NULL,
                key TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_secret_refs_mcp ON secret_refs (mcp_id);

            CREATE TABLE IF NOT EXISTS harness_state (
                harness_id TEXT PRIMARY KEY,
                last_sync_at TEXT,
                last_error TEXT,
                last_known_hash TEXT,
                externally_modified INTEGER NOT NULL DEFAULT 0
            );

            -- Locally cached catalog for the MCP store (official registry).
            CREATE TABLE IF NOT EXISTS store_entries (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                name TEXT NOT NULL,
                title TEXT,
                description TEXT NOT NULL DEFAULT '',
                version TEXT,
                homepage TEXT,
                repository TEXT,
                packages_json TEXT NOT NULL DEFAULT '[]',
                remotes_json TEXT NOT NULL DEFAULT '[]',
                fetched_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_store_entries_source ON store_entries (source);
            CREATE INDEX IF NOT EXISTS idx_store_entries_name ON store_entries (name COLLATE NOCASE);

            CREATE TABLE IF NOT EXISTS store_meta (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            -- Skills cached from GitHub-hosted skill repositories.
            CREATE TABLE IF NOT EXISTS skill_entries (
                id TEXT PRIMARY KEY,
                source_id TEXT NOT NULL,
                source_label TEXT NOT NULL DEFAULT '',
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                path TEXT NOT NULL,
                files_json TEXT NOT NULL DEFAULT '[]',
                fetched_at TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_skill_entries_source ON skill_entries (source_id);

            -- Skills installed into the app library and optionally deployed.
            CREATE TABLE IF NOT EXISTS installed_skills (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL,
                description TEXT NOT NULL DEFAULT '',
                source_id TEXT NOT NULL,
                source_path TEXT NOT NULL,
                library_path TEXT NOT NULL,
                deployed_paths_json TEXT NOT NULL DEFAULT '[]',
                installed_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    fn with_conn<T>(&self, f: impl FnOnce(&Connection) -> AppResult<T>) -> AppResult<T> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| AppError::db("database lock poisoned"))?;
        f(&guard)
    }

    // ---------------------------------------------------------------- servers

    fn row_to_server(row: &rusqlite::Row<'_>) -> rusqlite::Result<McpServer> {
        let transport: String = row.get("transport")?;
        let args_json: String = row.get("args_json")?;
        let env_json: String = row.get("env_json")?;
        let headers_json: String = row.get("headers_json")?;
        let targets_json: String = row.get("targets_json")?;
        let tags_json: String = row.get("tags_json")?;
        let enabled: i64 = row.get("enabled")?;

        Ok(McpServer {
            id: row.get("id")?,
            name: row.get("name")?,
            description: row.get("description")?,
            transport: match transport.as_str() {
                "stdio" => TransportType::Stdio,
                "sse" => TransportType::Sse,
                _ => TransportType::StreamableHttp,
            },
            command: row.get("command")?,
            args: serde_json::from_str(&args_json).unwrap_or_default(),
            env: serde_json::from_str(&env_json).unwrap_or_default(),
            headers: serde_json::from_str(&headers_json).unwrap_or_default(),
            url: row.get("url")?,
            cwd: row.get("cwd")?,
            enabled: enabled != 0,
            targets: serde_json::from_str(&targets_json).unwrap_or_default(),
            tags: serde_json::from_str(&tags_json).unwrap_or_default(),
            notes: row.get("notes")?,
            created_at: row.get("created_at")?,
            updated_at: row.get("updated_at")?,
            last_sync_at: row.get("last_sync_at")?,
        })
    }

    pub fn list_servers(&self) -> AppResult<Vec<McpServer>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare("SELECT * FROM mcp_servers ORDER BY name COLLATE NOCASE ASC")?;
            let rows = stmt.query_map([], Self::row_to_server)?;
            let mut servers = Vec::new();
            for row in rows {
                servers.push(row?);
            }
            Ok(servers)
        })
    }

    pub fn get_server(&self, id: &str) -> AppResult<McpServer> {
        self.with_conn(|conn| {
            conn.query_row("SELECT * FROM mcp_servers WHERE id = ?1", params![id], Self::row_to_server)
                .optional()?
                .ok_or_else(|| AppError::not_found(format!("MCP server {id} does not exist")))
        })
    }

    pub fn find_server_by_name(&self, name: &str) -> AppResult<Option<McpServer>> {
        let key = normalize_name(name);
        self.with_conn(|conn| {
            Ok(conn
                .query_row("SELECT * FROM mcp_servers WHERE name_key = ?1", params![key], Self::row_to_server)
                .optional()?)
        })
    }

    pub fn insert_server(&self, server: &McpServer) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO mcp_servers
                   (id, name, name_key, description, transport, command, args_json, env_json, headers_json,
                    url, cwd, enabled, targets_json, tags_json, notes, created_at, updated_at, last_sync_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)"#,
                params![
                    server.id,
                    server.name,
                    normalize_name(&server.name),
                    server.description,
                    server.transport.as_str(),
                    server.command,
                    serde_json::to_string(&server.args)?,
                    serde_json::to_string(&server.env)?,
                    serde_json::to_string(&server.headers)?,
                    server.url,
                    server.cwd,
                    server.enabled as i64,
                    serde_json::to_string(&server.targets)?,
                    serde_json::to_string(&server.tags)?,
                    server.notes,
                    server.created_at,
                    server.updated_at,
                    server.last_sync_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn update_server(&self, server: &McpServer) -> AppResult<()> {
        self.with_conn(|conn| {
            let changed = conn.execute(
                r#"UPDATE mcp_servers SET
                     name = ?2, name_key = ?3, description = ?4, transport = ?5, command = ?6,
                     args_json = ?7, env_json = ?8, headers_json = ?9, url = ?10, cwd = ?11,
                     enabled = ?12, targets_json = ?13, tags_json = ?14, notes = ?15, updated_at = ?16
                   WHERE id = ?1"#,
                params![
                    server.id,
                    server.name,
                    normalize_name(&server.name),
                    server.description,
                    server.transport.as_str(),
                    server.command,
                    serde_json::to_string(&server.args)?,
                    serde_json::to_string(&server.env)?,
                    serde_json::to_string(&server.headers)?,
                    server.url,
                    server.cwd,
                    server.enabled as i64,
                    serde_json::to_string(&server.targets)?,
                    serde_json::to_string(&server.tags)?,
                    server.notes,
                    server.updated_at,
                ],
            )?;
            if changed == 0 {
                return Err(AppError::not_found(format!("MCP server {} does not exist", server.id)));
            }
            Ok(())
        })
    }

    pub fn delete_server(&self, id: &str) -> AppResult<()> {
        self.with_conn(|conn| {
            let changed = conn.execute("DELETE FROM mcp_servers WHERE id = ?1", params![id])?;
            if changed == 0 {
                return Err(AppError::not_found(format!("MCP server {id} does not exist")));
            }
            conn.execute("DELETE FROM secret_refs WHERE mcp_id = ?1", params![id])?;
            Ok(())
        })
    }

    pub fn set_server_enabled(&self, id: &str, enabled: bool, updated_at: &str) -> AppResult<()> {
        self.with_conn(|conn| {
            let changed = conn.execute(
                "UPDATE mcp_servers SET enabled = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, enabled as i64, updated_at],
            )?;
            if changed == 0 {
                return Err(AppError::not_found(format!("MCP server {id} does not exist")));
            }
            Ok(())
        })
    }

    pub fn mark_synced(&self, id: &str, at: &str) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute("UPDATE mcp_servers SET last_sync_at = ?2 WHERE id = ?1", params![id, at])?;
            Ok(())
        })
    }

    // ------------------------------------------------------------ secret refs

    pub fn register_secret_ref(&self, mcp_id: &str, slot: &str, key: &str) -> AppResult<()> {
        let id = format!("{mcp_id}:{slot}:{key}");
        self.with_conn(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO secret_refs (id, mcp_id, slot, key, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id, mcp_id, slot, key, Utc::now().to_rfc3339()],
            )?;
            Ok(())
        })
    }

    pub fn remove_secret_ref(&self, mcp_id: &str, slot: &str, key: &str) -> AppResult<()> {
        let id = format!("{mcp_id}:{slot}:{key}");
        self.with_conn(|conn| {
            conn.execute("DELETE FROM secret_refs WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    pub fn list_secret_refs(&self, mcp_id: &str) -> AppResult<Vec<(String, String)>> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT slot, key FROM secret_refs WHERE mcp_id = ?1 ORDER BY slot, key")?;
            let rows = stmt.query_map(params![mcp_id], |row| Ok((row.get(0)?, row.get(1)?)))?;
            let mut refs = Vec::new();
            for row in rows {
                refs.push(row?);
            }
            Ok(refs)
        })
    }

    // ----------------------------------------------------------------- history

    pub fn record_sync(
        &self,
        harness_id: &str,
        mcp_id: Option<&str>,
        mcp_name: Option<&str>,
        action: &str,
        status: &str,
        message: &str,
    ) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sync_history (harness_id, mcp_id, mcp_name, action, status, message, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![harness_id, mcp_id, mcp_name, action, status, message, Utc::now().to_rfc3339()],
            )?;
            Ok(())
        })
    }

    pub fn sync_history(&self, limit: u32) -> AppResult<Vec<SyncHistoryEntry>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, harness_id, mcp_id, mcp_name, action, status, message, created_at
                 FROM sync_history ORDER BY id DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit], |row| {
                Ok(SyncHistoryEntry {
                    id: row.get(0)?,
                    harness_id: row.get(1)?,
                    mcp_id: row.get(2)?,
                    mcp_name: row.get(3)?,
                    action: row.get(4)?,
                    status: row.get(5)?,
                    message: row.get(6)?,
                    created_at: row.get(7)?,
                })
            })?;
            let mut entries = Vec::new();
            for row in rows {
                entries.push(row?);
            }
            Ok(entries)
        })
    }

    pub fn sync_counts(&self) -> AppResult<SyncCounts> {
        self.with_conn(|conn| {
            let successful: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sync_history WHERE status IN ('success', 'noChange')",
                [],
                |row| row.get(0),
            )?;
            let failed: i64 = conn.query_row(
                "SELECT COUNT(*) FROM sync_history WHERE status = 'failed'",
                [],
                |row| row.get(0),
            )?;
            let last_sync_at: Option<String> = conn
                .query_row("SELECT MAX(created_at) FROM sync_history", [], |row| row.get(0))
                .optional()?
                .flatten();
            Ok(SyncCounts {
                successful: successful as usize,
                failed: failed as usize,
                last_sync_at,
            })
        })
    }

    // ----------------------------------------------------------------- backups

    pub fn insert_backup(&self, info: &BackupInfo) -> AppResult<i64> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO backups (harness_id, original_path, backup_path, sha256, size, reason, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    info.harness_id,
                    info.original_path,
                    info.backup_path,
                    info.sha256,
                    info.size as i64,
                    info.reason,
                    info.created_at
                ],
            )?;
            Ok(conn.last_insert_rowid())
        })
    }

    pub fn list_backups(&self, harness_id: Option<&str>) -> AppResult<Vec<BackupInfo>> {
        self.with_conn(|conn| {
            let (sql, filter) = match harness_id {
                Some(_) => ("SELECT * FROM backups WHERE harness_id = ?1 ORDER BY id DESC LIMIT 200", true),
                None => ("SELECT * FROM backups ORDER BY id DESC LIMIT 200", false),
            };
            let mut stmt = conn.prepare(sql)?;
            let map_row = |row: &rusqlite::Row<'_>| -> rusqlite::Result<BackupInfo> {
                Ok(BackupInfo {
                    id: row.get("id")?,
                    harness_id: row.get("harness_id")?,
                    original_path: row.get("original_path")?,
                    backup_path: row.get("backup_path")?,
                    sha256: row.get("sha256")?,
                    size: row.get::<_, i64>("size")? as u64,
                    reason: row.get("reason")?,
                    created_at: row.get("created_at")?,
                    exists: Path::new(&row.get::<_, String>("backup_path")?).exists(),
                })
            };
            let mut out = Vec::new();
            if filter {
                let rows = stmt.query_map(params![harness_id.unwrap_or_default()], map_row)?;
                for row in rows {
                    out.push(row?);
                }
            } else {
                let rows = stmt.query_map([], map_row)?;
                for row in rows {
                    out.push(row?);
                }
            }
            Ok(out)
        })
    }

    pub fn get_backup(&self, id: i64) -> AppResult<BackupInfo> {
        let all = self.list_backups(None)?;
        all.into_iter()
            .find(|item| item.id == id)
            .ok_or_else(|| AppError::not_found(format!("Backup {id} does not exist")))
    }

    // ---------------------------------------------------------------- settings

    pub fn get_setting_json<T: DeserializeOwned>(&self, key: &str) -> AppResult<Option<T>> {
        let raw: Option<String> = self.with_conn(|conn| {
            Ok(conn
                .query_row("SELECT value FROM app_settings WHERE key = ?1", params![key], |row| row.get(0))
                .optional()?)
        })?;
        match raw {
            Some(value) => Ok(Some(serde_json::from_str(&value).map_err(|err| {
                AppError::db(format!("stored setting '{key}' is not valid JSON: {err}"))
            })?)),
            None => Ok(None),
        }
    }

    pub fn set_setting_json<T: Serialize>(&self, key: &str, value: &T) -> AppResult<()> {
        let encoded = serde_json::to_string(value)?;
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, encoded],
            )?;
            Ok(())
        })
    }

    pub fn delete_setting(&self, key: &str) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM app_settings WHERE key = ?1", params![key])?;
            Ok(())
        })
    }

    // ------------------------------------------------------------------ store

    fn row_to_store_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoreEntry> {
        let packages_json: String = row.get("packages_json")?;
        let remotes_json: String = row.get("remotes_json")?;
        Ok(StoreEntry {
            id: row.get("id")?,
            source: row.get("source")?,
            name: row.get("name")?,
            title: row.get("title")?,
            description: row.get("description")?,
            version: row.get("version")?,
            homepage: row.get("homepage")?,
            repository: row.get("repository")?,
            packages: serde_json::from_str(&packages_json).unwrap_or_default(),
            remotes: serde_json::from_str(&remotes_json).unwrap_or_default(),
            fetched_at: row.get("fetched_at")?,
        })
    }

    pub fn upsert_store_entry(&self, entry: &StoreEntry) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO store_entries
                   (id, source, name, title, description, version, homepage, repository,
                    packages_json, remotes_json, fetched_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
                   ON CONFLICT(id) DO UPDATE SET
                     source = excluded.source,
                     name = excluded.name,
                     title = excluded.title,
                     description = excluded.description,
                     version = excluded.version,
                     homepage = excluded.homepage,
                     repository = excluded.repository,
                     packages_json = excluded.packages_json,
                     remotes_json = excluded.remotes_json,
                     fetched_at = excluded.fetched_at"#,
                params![
                    entry.id,
                    entry.source,
                    entry.name,
                    entry.title,
                    entry.description,
                    entry.version,
                    entry.homepage,
                    entry.repository,
                    serde_json::to_string(&entry.packages)?,
                    serde_json::to_string(&entry.remotes)?,
                    entry.fetched_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn list_store_entries(
        &self,
        query: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> AppResult<Vec<StoreEntry>> {
        let needle = query.map(|value| format!("%{}%", value.trim()));
        self.with_conn(|conn| {
            let mut out = Vec::new();
            if let Some(needle) = needle {
                let mut stmt = conn.prepare(
                    "SELECT * FROM store_entries
                     WHERE name LIKE ?1 OR IFNULL(title, '') LIKE ?1 OR description LIKE ?1
                     ORDER BY name COLLATE NOCASE ASC LIMIT ?2 OFFSET ?3",
                )?;
                let rows = stmt.query_map(params![needle, limit, offset], Self::row_to_store_entry)?;
                for row in rows {
                    out.push(row?);
                }
            } else {
                let mut stmt = conn.prepare(
                    "SELECT * FROM store_entries
                     ORDER BY name COLLATE NOCASE ASC LIMIT ?1 OFFSET ?2",
                )?;
                let rows = stmt.query_map(params![limit, offset], Self::row_to_store_entry)?;
                for row in rows {
                    out.push(row?);
                }
            }
            Ok(out)
        })
    }

    pub fn count_store_entries(&self, query: Option<&str>) -> AppResult<usize> {
        let needle = query.map(|value| format!("%{}%", value.trim()));
        self.with_conn(|conn| {
            let count: i64 = if let Some(needle) = needle {
                conn.query_row(
                    "SELECT COUNT(*) FROM store_entries
                     WHERE name LIKE ?1 OR IFNULL(title, '') LIKE ?1 OR description LIKE ?1",
                    params![needle],
                    |row| row.get(0),
                )?
            } else {
                conn.query_row("SELECT COUNT(*) FROM store_entries", [], |row| row.get(0))?
            };
            Ok(count as usize)
        })
    }

    pub fn get_store_entry(&self, id: &str) -> AppResult<StoreEntry> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT * FROM store_entries WHERE id = ?1",
                params![id],
                Self::row_to_store_entry,
            )
            .optional()?
            .ok_or_else(|| AppError::not_found(format!("Store entry {id} is not in the local cache. Refresh the catalog first.")))
        })
    }

    pub fn clear_store_source(&self, source: &str) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM store_entries WHERE source = ?1", params![source])?;
            Ok(())
        })
    }

    pub fn store_last_refreshed(&self, source: &str) -> AppResult<Option<String>> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT MAX(fetched_at) FROM store_entries WHERE source = ?1",
                    params![source],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()?
                .flatten())
        })
    }

    pub fn get_store_meta(&self, key: &str) -> AppResult<Option<String>> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row("SELECT value FROM store_meta WHERE key = ?1", params![key], |row| row.get(0))
                .optional()?)
        })
    }

    pub fn set_store_meta(&self, key: &str, value: Option<&str>) -> AppResult<()> {
        self.with_conn(|conn| {
            match value {
                Some(value) => {
                    conn.execute(
                        "INSERT INTO store_meta (key, value) VALUES (?1, ?2)
                         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        params![key, value],
                    )?;
                }
                None => {
                    conn.execute("DELETE FROM store_meta WHERE key = ?1", params![key])?;
                }
            }
            Ok(())
        })
    }

    // ----------------------------------------------------------------- skills

    fn row_to_skill_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillEntry> {
        let files_json: String = row.get("files_json")?;
        Ok(SkillEntry {
            id: row.get("id")?,
            source_id: row.get("source_id")?,
            source_label: row.get("source_label")?,
            name: row.get("name")?,
            description: row.get("description")?,
            path: row.get("path")?,
            files: serde_json::from_str(&files_json).unwrap_or_default(),
            fetched_at: row.get("fetched_at")?,
        })
    }

    /// Replace every cached skill of one source with a freshly fetched list.
    pub fn replace_source_skills(&self, source_id: &str, entries: &[SkillEntry]) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM skill_entries WHERE source_id = ?1", params![source_id])?;
            for entry in entries {
                conn.execute(
                    r#"INSERT OR REPLACE INTO skill_entries
                       (id, source_id, source_label, name, description, path, files_json, fetched_at)
                       VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
                    params![
                        entry.id,
                        entry.source_id,
                        entry.source_label,
                        entry.name,
                        entry.description,
                        entry.path,
                        serde_json::to_string(&entry.files)?,
                        entry.fetched_at,
                    ],
                )?;
            }
            Ok(())
        })
    }

    pub fn list_skill_entries(
        &self,
        source_id: Option<&str>,
        query: Option<&str>,
    ) -> AppResult<Vec<SkillEntry>> {
        let needle = query.map(|value| format!("%{}%", value.trim()));
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT * FROM skill_entries
                 WHERE (?1 IS NULL OR source_id = ?1)
                   AND (?2 IS NULL OR name LIKE ?2 OR description LIKE ?2)
                 ORDER BY name COLLATE NOCASE ASC",
            )?;
            let rows = stmt.query_map(params![source_id, needle], Self::row_to_skill_entry)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn get_skill_entry(&self, id: &str) -> AppResult<SkillEntry> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT * FROM skill_entries WHERE id = ?1",
                params![id],
                Self::row_to_skill_entry,
            )
            .optional()?
            .ok_or_else(|| AppError::not_found(format!("Skill {id} is not in the local cache. Refresh the skills catalog first.")))
        })
    }

    fn row_to_installed_skill(row: &rusqlite::Row<'_>) -> rusqlite::Result<InstalledSkill> {
        let deployed_json: String = row.get("deployed_paths_json")?;
        Ok(InstalledSkill {
            id: row.get("id")?,
            name: row.get("name")?,
            description: row.get("description")?,
            source_id: row.get("source_id")?,
            source_path: row.get("source_path")?,
            library_path: row.get("library_path")?,
            deployed_paths: serde_json::from_str(&deployed_json).unwrap_or_default(),
            installed_at: row.get("installed_at")?,
            updated_at: row.get("updated_at")?,
        })
    }

    pub fn list_installed_skills(&self) -> AppResult<Vec<InstalledSkill>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT * FROM installed_skills ORDER BY name COLLATE NOCASE ASC",
            )?;
            let rows = stmt.query_map([], Self::row_to_installed_skill)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
    }

    pub fn get_installed_skill(&self, id: &str) -> AppResult<Option<InstalledSkill>> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT * FROM installed_skills WHERE id = ?1",
                    params![id],
                    Self::row_to_installed_skill,
                )
                .optional()?)
        })
    }

    pub fn upsert_installed_skill(&self, skill: &InstalledSkill) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                r#"INSERT INTO installed_skills
                   (id, name, description, source_id, source_path, library_path,
                    deployed_paths_json, installed_at, updated_at)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
                   ON CONFLICT(id) DO UPDATE SET
                     name = excluded.name,
                     description = excluded.description,
                     source_id = excluded.source_id,
                     source_path = excluded.source_path,
                     library_path = excluded.library_path,
                     deployed_paths_json = excluded.deployed_paths_json,
                     updated_at = excluded.updated_at"#,
                params![
                    skill.id,
                    skill.name,
                    skill.description,
                    skill.source_id,
                    skill.source_path,
                    skill.library_path,
                    serde_json::to_string(&skill.deployed_paths)?,
                    skill.installed_at,
                    skill.updated_at,
                ],
            )?;
            Ok(())
        })
    }

    pub fn delete_installed_skill(&self, id: &str) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM installed_skills WHERE id = ?1", params![id])?;
            Ok(())
        })
    }

    // ------------------------------------------------------------ harness state

    pub fn get_harness_state(&self, harness_id: &str) -> AppResult<Option<HarnessState>> {
        self.with_conn(|conn| {
            Ok(conn
                .query_row(
                    "SELECT harness_id, last_sync_at, last_error, last_known_hash, externally_modified
                     FROM harness_state WHERE harness_id = ?1",
                    params![harness_id],
                    |row| {
                        Ok(HarnessState {
                            harness_id: row.get(0)?,
                            last_sync_at: row.get(1)?,
                            last_error: row.get(2)?,
                            last_known_hash: row.get(3)?,
                            externally_modified: row.get::<_, i64>(4)? != 0,
                        })
                    },
                )
                .optional()?)
        })
    }

    pub fn list_harness_states(&self) -> AppResult<Vec<HarnessState>> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT harness_id, last_sync_at, last_error, last_known_hash, externally_modified
                 FROM harness_state",
            )?;
            let rows = stmt.query_map([], |row| {
                Ok(HarnessState {
                    harness_id: row.get(0)?,
                    last_sync_at: row.get(1)?,
                    last_error: row.get(2)?,
                    last_known_hash: row.get(3)?,
                    externally_modified: row.get::<_, i64>(4)? != 0,
                })
            })?;
            let mut states = Vec::new();
            for row in rows {
                states.push(row?);
            }
            Ok(states)
        })
    }

    pub fn upsert_harness_state(&self, state: &HarnessState) -> AppResult<()> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO harness_state (harness_id, last_sync_at, last_error, last_known_hash, externally_modified)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(harness_id) DO UPDATE SET
                   last_sync_at = excluded.last_sync_at,
                   last_error = excluded.last_error,
                   last_known_hash = excluded.last_known_hash,
                   externally_modified = excluded.externally_modified",
                params![
                    state.harness_id,
                    state.last_sync_at,
                    state.last_error,
                    state.last_known_hash,
                    state.externally_modified as i64
                ],
            )?;
            Ok(())
        })
    }
}

pub fn normalize_name(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Helper used by tests and the registry service.
pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

/// Count how many registry entries target a harness.
pub fn count_targets(servers: &[McpServer], harness_id: &str) -> usize {
    servers
        .iter()
        .filter(|server| server.targets.iter().any(|target| target == harness_id))
        .count()
}

#[allow(dead_code)]
pub(crate) fn empty_env() -> std::collections::BTreeMap<String, VarValue> {
    Default::default()
}
