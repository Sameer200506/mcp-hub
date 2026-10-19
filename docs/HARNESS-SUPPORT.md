# Supported harnesses, schemas and limitations

Every adapter in this project encodes a schema taken from a vendor's published documentation.
Where a location or a field could not be verified, the adapter reports
**`PendingVerification`** in the UI instead of inventing a path or a key. This page is the
authoritative list of what is written, where, and what is deliberately left alone.

| Harness | Adapter status | Configuration file | Transports |
| --- | --- | --- | --- |
| [OpenCode](#opencode) | `Verified` | `~/.config/opencode/opencode.json` (or `.jsonc`) | stdio, Streamable HTTP |
| [Cline](#cline) | `Verified` | `%APPDATA%\<editor>\User\globalStorage\saoudrizwan.claude-dev\settings\cline_mcp_settings.json` or `~/.cline/data/settings/cline_mcp_settings.json` | stdio, Streamable HTTP, SSE |
| [Google Antigravity](#google-antigravity) | `Verified` | `~/.gemini/config/mcp_config.json` | stdio, Streamable HTTP, SSE |
| [Generic JSON profile](#generic-json-profiles) | `PendingVerification` until you confirm the mapping | whatever file you configure | whatever the mapping declares |

Server names become JSON keys, so they are restricted to `A-Z a-z 0-9 . _ -` (enforced when a
definition is saved).

---

## OpenCode

**Resolved configuration file (first match wins):**

1. the path override in Settings (`config_overrides["opencode"]`), if set;
2. `%USERPROFILE%\.config\opencode\opencode.json`;
3. `%USERPROFILE%\.config\opencode\opencode.jsonc`.

Project-level `opencode.json` files are listed by the detection API for inspection, but a
synchronization writes the global file unless you point the override somewhere else.

**Entry shape** — definitions live under the `mcp` object, keyed by server name:

```jsonc
{
  "mcp": {
    "filesystem": {                     // stdio
      "type": "local",
      "command": ["npx", "-y", "@modelcontextprotocol/server-filesystem"],  // executable + args in one array
      "environment": { "ROOT": "C:/data" },
      "cwd": "C:/data",
      "enabled": true
    },
    "search": {                         // remote
      "type": "remote",
      "url": "https://mcp.example.test/mcp",
      "headers": { "Authorization": "{env:SEARCH_TOKEN}" },
      "enabled": true
    }
  }
}
```

**Capabilities:** env, headers, working directory, enable flag. `timeout` and `oauth` are known
extra fields: they are preserved if they already exist in an entry and are never invented.

**Never written / never touched:** fields the adapter does not understand, unrelated top-level
keys, project-level files, and the `type` values `local`/`remote` on entries it does not update.

**Notes and limits:**

* SSE definitions are skipped with a warning — SSE is not part of the documented OpenCode schema.
* JSONC comments are honoured on read; a file this adapter rewrites is written back as plain JSON
  (a backup of the original bytes is kept).
* OpenCode reads its configuration at startup; restart the CLI/TUI (or start a new session) after a
  sync.

---

## Cline

**Resolved configuration file (first match wins):**

1. `CLINE_MCP_SETTINGS_PATH` (the variable Cline's CLI honours), if set;
2. the path override in Settings, if set;
3. the first existing of the editor profiles below;
4. `%USERPROFILE%\.cline\data\settings\cline_mcp_settings.json` (CLI).

Detected editor profiles under `%APPDATA%`: **VS Code** (`Code`), **VS Code Insiders**
(`Code - Insiders`), **VSCodium**, **Cursor**, **Windsurf**. Each maps to
`<editor>\User\globalStorage\saoudrizwan.claude-dev\settings\cline_mcp_settings.json`.
The Agents page shows which file was chosen; the Settings override pins a different one.

**Entry shape** — definitions live under `mcpServers`, keyed by server name:

```jsonc
{
  "mcpServers": {
    "filesystem": {                     // stdio
      "type": "stdio",
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem"],
      "env": { "ROOT": "C:/data" },
      "disabled": false
    },
    "search": {                         // remote
      "type": "streamableHttp",         // or "sse" for a legacy SSE definition
      "url": "https://mcp.example.test/mcp",
      "headers": { "Authorization": "{env:SEARCH_TOKEN}" },
      "disabled": false
    }
  }
}
```

**Capabilities:** env, headers, enable flag, `autoApprove` (preserved, never invented).

**Never written / never touched:** the `cwd` field (Cline has no documented working-directory
field — a definition with a working directory is written without it and the preview explains the
drop), extension-internal state, and every file other than the one resolved above.

**Notes and limits:**

* The transport is always written explicitly. Cline treats a missing `type` as legacy SSE, so
  omitting it would silently change behaviour.
* The extension and the CLI keep separate files with the same schema; the app edits one file per
  sync and tells you which.

---

## Google Antigravity

**Resolved configuration file (first match wins):**

1. the path override in Settings, if set;
2. `%USERPROFILE%\.gemini\config\mcp_config.json` (the global config shared by the App, CLI and IDE
   install variants);
3. `%USERPROFILE%\.gemini\antigravity\mcp_config.json` (legacy per-product file), only when it
   exists and the global one does not;
4. otherwise the global path (created on first sync).

A workspace file (`.agents/mcp_config.json` inside a project) is listed for inspection and is
written **only** when the path override points at it.

**Entry shape** — definitions live under `mcpServers`, keyed by server name, and **never contain a
`type` field**:

```jsonc
{
  "mcpServers": {
    "filesystem": {                     // stdio
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem"],
      "env": { "ROOT": "C:/data" },
      "disabled": false
    },
    "search": {                         // remote
      "serverUrl": "https://mcp.example.test/mcp",
      "headers": { "Authorization": "{env:SEARCH_TOKEN}" },
      "disabled": false
    }
  }
}
```

The remote URL field is `serverUrl` (`url` is not an Antigravity field and is removed if it was
present), and a legacy `type` key found on an entry this adapter updates is removed.

**Capabilities:** env, headers, enable flag. No working-directory field is documented, so a
definition with a `cwd` is written without it (with a warning).

**Never written / never touched:** `type` (removed where found), OAuth token files
(`~/.gemini/antigravity/mcp_oauth_tokens.json` and friends), project workspace files unless the
override selects them, and any other key in the document.

**Notes and limits:**

* Three install variants (App, CLI, IDE) share the global file; the adapter always shows the path
  it will write.
* Antigravity is a young product: its config surface can still change between builds. This adapter
  reports what it resolved and keeps the preview exact, so a schema change is visible before it is
  written.

---

## Generic JSON profiles

For any other harness you configure yourself: pick a JSON file, the path to the MCP container (dot
path such as `mcpServers` or `mcp.servers`), and how that format expresses a server entry
(transport field name and values, URL field, command/args/env/headers/cwd/disabled field names).

* The profile starts as **`PendingVerification`** and stays that way until you mark it verified;
  the preview keeps warning you until then. This is intentional — the app cannot verify a schema it
  has never seen.
* An **empty field name means "this format has no such field"**. An empty key is never written.
* The command field (stdio) and URL field (remote) are mandatory: a profile that leaves them blank
  is rejected with `VALIDATION_FAILED` rather than writing a broken entry.
* Writes are restricted to the configured file; anything else fails with `PERMISSION_DENIED`.

Example profile (Settings → Generic JSON profiles):

```json
{
  "id": "my-editor",
  "label": "My Editor",
  "configPath": "C:\\Users\\me\\AppData\\Roaming\\MyEditor\\mcp.json",
  "mcpPath": "mcpServers",
  "serverNameKey": "name",
  "transportMapping": {
    "stdioUses": "type",
    "stdioValue": "stdio",
    "remoteUses": "type",
    "streamableHttpValue": "streamableHttp",
    "sseValue": "sse",
    "urlField": "url",
    "commandField": "command",
    "argsField": "args",
    "envField": "env",
    "cwdField": "cwd",
    "headersField": "headers",
    "disabledField": "disabled"
  },
  "customFields": [],
  "verified": false
}
```

---

## Capability matrix

| | stdio | Streamable HTTP | SSE | env | headers | cwd | enable flag | autoApprove |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| OpenCode | ✅ | ✅ | ❌ (skipped with a warning) | ✅ | ✅ | ✅ | ✅ `enabled` | — |
| Cline | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ (dropped with a warning) | ✅ `disabled` | ✅ preserved |
| Antigravity | ✅ | ✅ | ✅ | ✅ | ✅ | ❌ (dropped with a warning) | ✅ `disabled` | — |
| Generic profile | per mapping | per mapping | per mapping | per mapping | per mapping | per mapping | per mapping | — |

---

## Limitations

Scope and platform:

* Windows desktop application. The credential store backend is Windows-only; on other platforms
  the app refuses to store secrets instead of creating a plaintext fallback.
* No MCP gateway, proxy, router or hosted service, and no server lifecycle management. The app
  edits configuration files and nothing else.
* Synchronization is always **preview → confirm → write**. Unattended/automatic synchronization is
  not implemented; `sync mode` and `auto-sync on change` are stored settings for a later phase.
* The connection probe is not an MCP handshake: it proves the command starts or the endpoint
  answers, nothing more.
* Backups accumulate and are never pruned automatically; a restore needs the original path to still
  exist (or be creatable) and writable.
* Configuration formats other than JSON/JSONC are not supported (no TOML, YAML or JSON5), and a
  container path is a single dot path — no array indices or wildcards.
* A synchronization normalizes what it touches: `enabled`/`disabled` follows the convention of the
  target harness, and a JSONC file is rewritten as plain JSON.

Per harness:

* **OpenCode** — no SSE; only the global config is written (project files are inspection-only);
  JSONC comments are normalized on write; `timeout`/`oauth` are preserved but never generated.
* **Cline** — no working directory (`cwd` is dropped with a warning); the transport is always
  written explicitly; the extension and the CLI use different files, and only the resolved one is
  edited.
* **Antigravity** — `type` is never written and a legacy `type` key is removed when an entry is
  updated; the URL field is `serverUrl`; workspace files are only written when the override points
  at them.
* **Generic profiles** — `PendingVerification` by default; mandatory command/URL field names; the
  mapping describes one convention per profile.

Verification status of this document:

* The schemas above were derived from the vendors' documentation and are locked in by the fixture
  tests in `src-tauri/tests/adapter_fixtures.rs`, which assert the exact JSON written for every
  harness. Those tests are the contract: if a harness changes its schema, the fixtures must change
  with it before a synchronization can write the new shape.
* No vendor version numbers are pinned here on purpose. The app reports the configuration file it
  actually resolved, shows the exact bytes it will write, and refuses to write when a file is
  malformed — so a schema change shows up as a preview you can decline, not as a silent write.
* Where a location or field could not be verified, the adapter says so (`PendingVerification`,
  detection confidence `notDetected`) instead of guessing.
