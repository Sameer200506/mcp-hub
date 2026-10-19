# Security model

Universal MCP Manager edits files that other tools trust: the configuration of your coding agents.
This document describes what is protected, how, and — just as importantly — what the application
does **not** protect against.

## 1. Assets and threat model

| Asset | Where it lives | Primary risk |
| --- | --- | --- |
| MCP credentials (tokens, API keys) | Windows Credential Manager | disclosure via files, logs, backups or the UI |
| Harness configuration files | `~/.config/opencode/…`, `%APPDATA%\…\cline_mcp_settings.json`, `~/.gemini/config/mcp_config.json`, user-defined JSON | corruption, clobbering a hand edit, leaking what was there before |
| The registry itself | `%APPDATA%\UniversalMcpManager\registry.sqlite3` | integrity (a wrong entry gets written to every harness) |

Assumed adversary / failure modes:

* another **local user** on the same machine reading files (a per-user data directory and the
  per-user credential store are the countermeasure);
* a **hostile or buggy harness** rewriting its own config while the app is open (hash checks);
* **accidental** destruction of a config by this app (backups, atomic replace, previews);
* **untrusted content** arriving through a configuration file (malformed JSON, unexpected fields,
  huge files) — parsing is strict and content is never executed or rendered as HTML;
* another **process running as your own user** — explicitly out of scope (see §6).

## 2. Credentials

* Secrets are stored with the Windows Credential Manager API (`CredWriteW` / `CredReadW` /
  `CredDeleteW`), credential type `CRED_TYPE_GENERIC` with **local-machine persistence**, i.e. the
  same protection Windows uses for "generic credentials" of your user profile.
  Target name format: `UniversalMCPManager|<server id>|<env|headers>|<KEY>`.
* The SQLite database stores **only the fact that a credential exists** (`secret_refs`); the value
  never enters the database, the logs, an export file, a diff, or the UI.
* Every value read from the store is registered in a per-operation list of sensitive strings. Any
  JSON rendered for previews, diffs, toasts or history — and every error message that quotes
  subprocess output — is scrubbed against that list and replaced with `••••••••`.
* Exports (`Servers → Export`) always strip secret values and mark the document as
  `includesSecrets: false`. Literal values that *look* like credentials
  (`sk-…`, `ghp_…`, `xoxb-…`, `Bearer …`, ≥32-char opaque strings) are redacted on export too,
  with a warning naming the affected key.
* Importing a definition whose `kind` is `secret` either re-stores the value if the file carries
  one, or requires that a value already exists in your credential store; otherwise the import is
  rejected for that entry instead of writing an empty secret into a harness file.
* **Prefer `{env:NAME}` / `{file:path}` references.** They are written verbatim into the harness
  config (the harness resolves them at launch), so no credential is copied to disk by this app at
  all. `resolve_vars` never expands them.
* There is deliberately **no fallback plaintext store**. On a platform without a supported
  credential store the app reports `SECRET_STORE_ERROR` and asks you to use literal values or
  references.

### 2.1 What the Credential Manager does and does not protect

Honest limits, so you can make an informed choice:

* Any process running **as your own user** can enumerate and read generic credentials. The store
  protects values from *other users* and from casual inspection of files — it is not an
  application-isolation boundary, and this app does not claim to be one.
* The credential target names are predictable (`UniversalMCPManager|<id>|<slot>|<KEY>`), so the
  *existence* of a credential is not hidden.
* Once a synchronization has written a value into a harness configuration file, that file contains
  the value in plaintext — that is a requirement of every harness supported today. Use
  `{env:NAME}` references wherever the harness resolves them, and keep those config files under
  the same file permissions you already apply to your other secrets.
* Deleting a server deletes its stored credentials; they are not retained in a recycle bin.

## 3. Filesystem and write safety

All filesystem work happens in the Rust backend. The React frontend cannot write files: it only
calls Tauri commands, and every command goes through the same service layer that the integration
tests exercise.

`services::safe_io` enforces, for every write:

1. **Refuse symlinks.** A config path that is a symbolic link is never followed.
2. **Read and snapshot first.** Existing content and its SHA-256 are recorded before anything is
   modified. Nothing is ever truncated up-front.
3. **Hash check (`FILE_CHANGED_ON_DISK`).** If the file on disk no longer matches the hash the
   preview was built from, the write is refused. Your file keeps whatever the other process wrote.
4. **Backup before write.** The previous content is copied into the backup directory (with its own
   SHA-256 record) when backups are enabled, which is the default.
5. **Temp file + atomic replace.** The new content is written to a temporary file *in the same
   directory* and then moved over the original, so a crash or a full disk cannot leave a truncated
   config.
6. **Read-back verification.** After the replace, the file is read again and compared. A mismatch
   surfaces as `VERIFICATION_FAILED` instead of being reported as a success.
7. **Idempotency.** If the serialized result is byte-identical to what is on disk, nothing is
   written at all (mtimes stay untouched, watchers are not triggered).

Additional guards:

* **Malformed JSON is never overwritten.** A file that cannot be parsed is reported
  (`CONFIG_UNWRITABLE`) with a remediation suggestion, and synchronization for that harness is
  blocked until you fix or restore it.
* **Unrelated keys are preserved.** Updates merge into the existing entry; harness-specific extras
  (`autoApprove`, `disabledTools`, `oauth`, …) survive a sync.
* **Path policy.** Adaptors write only to the config location they resolved for that harness, or to
  the file you configured for a generic profile. The generic adapter refuses paths outside its
  configured file (`PERMISSION_DENIED`), and `..` traversal is rejected. Settings require absolute
  paths.
* **Nothing is deleted blindly.** Removing an entry from a harness requires an explicit removal
  request in the preview; empty MCP containers are pruned, nothing else.

### 3.1 Backups and restore

* Backups are stored as `<backupDir>\<harness>__<timestamp>__<filename>` with a SHA-256 record in
  SQLite.
* A restore re-verifies the backup's hash first; a backup that was modified after creation is
  refused (`VALIDATION_FAILED`, "backup integrity check failed").
* A restore takes a `restore-safety` backup of the *current* file before replacing it, and reports
  the exact paths it touched, so a wrong restore can itself be undone.
* Backups contain whatever the file contained, **including any plaintext secrets that were already
  in it**. Treat the backup directory as sensitive. Using `{env:…}` references keeps credentials
  out of both the config and its backups.

## 4. Process execution and network

* The app never launches your MCP servers as a side effect of syncing. Editing a definition,
  previewing, or writing a config starts nothing.
* **Test connection** is the single, explicit exception:
  * for stdio definitions it starts the configured command **without a shell** (no
    `cmd.exe`, no expansion), passes the resolved environment and working directory, waits up to
    4 seconds, then kills it if it is still running;
  * for remote definitions it performs one HTTP request with an 8-second timeout;
  * the result carries `executedProcess: true` and a disclaimer stating that no MCP `initialize`
    handshake is performed, so a successful probe means "the command starts" / "the endpoint
    answers", not "the server works";
  * captured stderr is truncated to a summary and scrubbed of credential values before it is shown
    or logged.
* There is no telemetry, no update check, no analytics and no outgoing request other than the one
  you trigger with **Test connection**.
* Node/npm/`npx` are never invoked by the backend; commands are written into configs for the
  harness to run.

## 5. Data at rest and logging

* Registry rows live in SQLite at `%APPDATA%\UniversalMcpManager\registry.sqlite3` (WAL mode).
* Logs are written to `%APPDATA%\UniversalMcpManager\logs\universal-mcp-manager.log`
  (daily rotation). Diagnostics record actions, paths and error codes — never credential values.
  Subprocess output and error strings pass through the scrubber before they are logged or surfaced.
* The webview can reach the filesystem only through the commands listed in `src-tauri/src/lib.rs`
  and the permissions in `src-tauri/capabilities/default.json` (`core:default`, `dialog:default`,
  `opener:default`). There is no generic "read arbitrary file" or "write arbitrary file" command:
  * the single command that accepts a caller-supplied path (`get_harness_config_at_path`) validates
    it against the allowlist of known configuration locations for that harness, including any path
    override you configured, and refuses everything else;
  * `opener` is used only to reveal a configuration file/folder in Explorer;
  * export and import paths come from a file picker *and* are re-validated in the backend
    (absolute path, symlink refusal, JSON schema of the document).

## 6. Explicit non-goals

* Protecting against a hostile process running **as the same user** (it can read the credential
  store, the configs and the backups).
* Protecting against a compromised harness that rewrites its own configuration — the app only
  guarantees that *it* does not clobber such changes silently; it detects and reports them.
* Confidentiality of a credential **after** it has been written into a harness config file, if you
  chose a literal value instead of a reference.
* Being an MCP gateway, broker or sandbox: no traffic inspection, no policy enforcement, no
  server isolation.
* Multi-user or remote access: this is a single-user local tool.

## 7. Reporting a problem

If you find a security issue, please do not open a public issue with exploit details. Include the
app version, the affected file(s), and the steps to reproduce, and share it privately with the
maintainer of your copy of this repository.
