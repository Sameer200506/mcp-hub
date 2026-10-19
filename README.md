# MCP HUB (Universal MCP manager — local-first Windows desktop app)

A local-first Windows desktop application that keeps **MCP (Model Context Protocol) server
definitions** in one place and synchronizes them — safely and deliberately — into the
configuration files of the coding-agent harnesses you already use.

* Desktop app: **Tauri 2** (Rust backend) + **React 19 / TypeScript / Vite / Tailwind**.
* Registry: **SQLite** in your user profile, credentials in the **Windows Credential Manager**.
* Local only: no accounts, no cloud, no telemetry, no background network traffic.

> **Scope.** This is the registry + safe-synchronization product (phase 1 + 2). It is *not* an
> MCP gateway, proxy or router: it never sits between an agent and an MCP server, and it never
> runs your servers in the background. It also does not replace any harness — it edits the same
> JSON files you would edit by hand, with backups, previews and verification.

---

## What it does

| Capability | Notes |
| --- | --- |
| One registry for every MCP definition | stdio, Streamable HTTP and (legacy) SSE; env vars, headers, working directory, enable flag, tags, per-harness targets |
| Harness detection | Reports which harnesses are installed, which config file was found, and how confident that detection is |
| Preview before writing | Exact per-harness JSON, a line diff, warnings (unsupported transports, dropped fields, credentials that will be written) and conflict prompts |
| Safe writes | Backup first, temp file + atomic replace, hash check before writing, read-back verification, idempotent re-sync |
| Backups and restore | Timestamped backups with SHA-256 records; restore verifies integrity and takes a safety backup first |
| Secrets | Windows Credential Manager; values are masked in every preview, diff, export and log; `{env:NAME}` / `{file:path}` references stay references |
| Drift detection | A file watcher marks externally modified configs; nothing is ever overwritten automatically |
| Import / export | Portable registry JSON with redaction, merge or replace |

Supported harnesses and their exact schemas: **[docs/HARNESS-SUPPORT.md](docs/HARNESS-SUPPORT.md)**.
Security model and secrets handling: **[SECURITY.md](SECURITY.md)**.

---

## Requirements (Windows 10/11, x64)

| Requirement | Why | Check |
| --- | --- | --- |
| Node.js 20+ (24 recommended) | frontend toolchain | `node --version` |
| Rust stable **MSVC** toolchain | backend | `rustc --version` |
| Visual Studio 2022 Build Tools with **Desktop development with C++** | MSVC linker, Windows SDK | `cl.exe` reachable from a VS prompt |
| WebView2 Runtime | app shell (preinstalled on Windows 11) | `Get-AppxPackage *WebView2*` |
| WebView2 *evergreen bootstrapper* only if WebView2 is absent | — | <https://developer.microsoft.com/microsoft-edge/webview2/> |

Rust and the C++ workload can be installed quickly with:

```powershell
winget install Rustlang.Rustup
rustup default stable-msvc
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

If `cargo` is not on your `PATH` in a fresh terminal, call it explicitly:
`%USERPROFILE%\.cargo\bin\cargo.exe`.

---

## Install, develop, build

```powershell
# 1. frontend dependencies
npm install

# 2. run the app with hot-reloading UI (Vite on port 1420 + Tauri window)
npm run tauri dev

# 3. type-check + production frontend bundle
npm run build

# 4. production desktop build (installers land in src-tauri\target\release\bundle)
npm run tauri build
```

Useful narrower commands:

```powershell
npm run test                      # frontend unit tests (Vitest, jsdom)
cargo test --manifest-path src-tauri\Cargo.toml         # all Rust tests
cargo test --manifest-path src-tauri\Cargo.toml --test sync_pipeline
cargo build --manifest-path src-tauri\Cargo.toml        # backend only
```

### Where the app keeps its data

Everything lives in your user profile — nothing is written into the repository:

| Path | Contents |
| --- | --- |
| `%APPDATA%\UniversalMcpManager\registry.sqlite3` | registry, settings, sync history, backup index, secret *references* (never secret values) |
| `%APPDATA%\UniversalMcpManager\backups\` | timestamped backups, named `<harness>__<timestamp>__<filename>` |
| `%APPDATA%\UniversalMcpManager\logs\universal-mcp-manager.log` | daily-rotating log file (path also shown in Settings) |

The data and backup directories are configurable in **Settings**. Backups are never pruned
automatically — delete old files when you no longer want them.

---

## How a synchronization runs

1. **Detect** — the Agents page lists each harness, the config file it resolved to, and its
   `Verified` / `PendingVerification` adapter status.
2. **Preview** — `Preview sync` builds, for every selected server and targeted harness: the JSON
   entry that would be written (secrets masked), a diff against the current file, warnings and
   any conflicts (a different server already occupies that key).
3. **Confirm** — the apply button stays disabled while the preview is `blocked` (unwritable file,
   malformed JSON, transport that the harness cannot express). Conflicts need an explicit
   choice per entry: *update*, *rename* or *skip*.
4. **Write** — every write is: backup → serialize → temp file in the same directory → atomic
   replace → read-back verification. If the file changed between preview and write, the write is
   refused with `FILE_CHANGED_ON_DISK` instead of clobbering someone else's edit.
5. **Report** — per-harness status (`success`, `partial`, `skipped`, `failed`), bytes written,
   backup paths and any warnings; failures are isolated so one broken harness never blocks the
   others. A second run with no changes writes nothing at all.

Restoring a backup uses the same machinery and additionally re-backups the current file first
(`restore-safety`), so a restore is itself undoable.

---

## Secrets

* Choose **Secret** for a value → it is stored in the Windows Credential Manager under
  `UniversalMCPManager|<server id>|<env|headers>|<KEY>`.
* The SQLite database only stores the fact that a credential exists.
* Secret values are replaced with `••••••••` in previews, diffs, exports and logs. The real value
  is written only into the harness file at sync time, because that is the only place the harness
  can read it from.
* Prefer **`{env:NAME}`** (or `{file:path}`) references where the harness resolves them: the
  reference is written verbatim and no credential is ever copied into the config file.

Details, threat model and the honest limits of the Credential Manager: **[SECURITY.md](SECURITY.md)**.

---

## Sample definitions

`samples/registry.sample.json` is an importable registry document with four representative
entries (stdio + remote, one legacy-SSE, one disabled, one using an `{env:…}` reference) and no
real credentials:

**Servers → Import** → pick the absolute path of the file → `merge`. See
[samples/README.md](samples/README.md) for what each entry demonstrates.

---

## Tests

| Suite | Where | Covers |
| --- | --- | --- |
| Rust integration — adapters | `src-tauri/tests/adapter_fixtures.rs` | exact JSON per harness, enable/disable conventions, blank-mapping safety |
| Rust integration — sync engine | `src-tauri/tests/sync_pipeline.rs` | merge/preserve, backups, stale hashes, external edits, idempotency, partial failures, conflicts, removals, restore, tampering |
| Rust integration — registry & secrets | `src-tauri/tests/registry_and_secrets.rs` | CRUD, duplicate detection, import/export redaction, sample-file import, live Credential Manager round-trip |
| Rust unit tests | `#[cfg(test)]` modules in `src-tauri/src/**` | JSONC handling, diffing, safe-I/O primitives, path policy, error scrubbing |
| Frontend | `src/**/*.test.ts(x)` | masking helpers, JSON highlighting (HTML escaping) |

All tests run against `tempfile` workspaces and per-harness path overrides; they never touch your
real harness files. Two guarded tests exercise the real Windows Credential Manager and clean up
after themselves (they skip themselves if the store is unavailable). A dedicated test imports
`samples/registry.sample.json`, so the shipped sample cannot rot.

---

## Troubleshooting

**`cargo` is not recognized.** Open a new terminal, or run
`& "$env:USERPROFILE\.cargo\bin\cargo.exe"`. `rustup` adds the path only to new sessions.

**`error: linker 'link.exe' not found` / `LINK : fatal error LNK1104`.** Install the VS 2022 Build
Tools *Desktop development with C++* workload (see above) and reopen the terminal.

**`cargo test` fails with `STATUS_ENTRYPOINT_NOT_FOUND (0xC0000139)` right at startup.** The test
binary linked the GUI stack but was built without the application manifest. This project fixes
that in `src-tauri/build.rs`, which owns `src-tauri/windows/app.manifest` for *every* target and
disables `tauri-build`'s own copy. If you rename or move that file (or restructure the crate so
that `build.rs` no longer covers a test target), the symptom comes back — keep the manifest wiring
intact.

**The app window is blank / WebView2 missing.** Install the WebView2 Runtime (see Requirements).

**`npm run tauri dev` aborts with "Port 1420 is already in use".** Another `vite`/dev instance is
still running: close it, or `Get-NetTCPConnection -LocalPort 1420 | Select OwningProcess` and stop
that process.

**A write was refused with `FILE_CHANGED_ON_DISK`.** The harness file changed after the preview was
built (another editor, another tool, or the harness itself). Re-run the preview — your file was not
touched.

**`CONFIG_UNWRITABLE` / `PERMISSION_DENIED`.** The config is read-only, ACL-protected, a symlink,
or locked by another process. Clear the read-only flag, close the owning app, or point the harness
override at a writable copy (Settings → per-harness path override).

**My harness config is not where the app looks.** The app follows the vendor's documented location
and always shows the resolved path on the Agents page. Use the per-harness path override in
Settings to pin a different file; Cline additionally honours `CLINE_MCP_SETTINGS_PATH`.

**A restore says the backup was modified.** Backups are integrity-checked against the
SHA-256 recorded at creation. Tampered or truncated backup files are refused on purpose — use an
older backup or a copy you trust.

**Antivirus locks the temporary file during a write.** Writes use a temp file in the target
directory. If a scanner holds it, retry; the original file was never truncated.

**JSONC config with comments was rewritten as plain JSON.** OpenCode configs may contain comments;
they are honoured on read and normalized on write. Backups keep the original bytes if you need to
compare.

**I want the raw log.** Settings shows the log path; it is
`%APPDATA%\UniversalMcpManager\logs\universal-mcp-manager.log`. Credential values never appear in
it.

**`npm audit` reports advisories (vitest/tinypool).** They are in the **development** test runner
(`vitest` → `@vitest/mocker`, `tinypool`) and require running untrusted test code; none of the
flagged packages are shipped in the app bundle or the Tauri binary (`dependencies` is unaffected).
Clearing them means a major `vitest` upgrade, which is deliberately not done here — the suite was
verified on `vitest 3.x`.

---

## Known limitations

The full list lives in [docs/HARNESS-SUPPORT.md](docs/HARNESS-SUPPORT.md#limitations). In short:

* The OS credential store backend is **Windows-only**; on other platforms the app refuses to store
  secrets rather than falling back to a plain file.
* No MCP gateway/proxy/router, no server lifecycle management, no hosted service.
* Synchronization is always preview + confirm. The `sync mode`, `auto-sync on change` and
  `confirm before write` settings are persisted choices, but automatic (unattended) syncing is not
  implemented in this phase.
* Harness schemas are verified against vendor documentation and covered by fixture tests, but they
  cannot be validated against every application build; adapters whose schema cannot be verified are
  reported as `PendingVerification` instead of guessing.

---

## Architecture

```
src/                      React UI (never writes files; all mutations go through Tauri commands)
src/services/api.ts       typed wrapper over the invoke() surface
src-tauri/src/
  adapters/               per-harness schema knowledge (opencode, cline, antigravity, generic JSON)
  services/               sync engine, safe I/O, backups, validation, diffing, watcher, registry
  security/               path policy, credential store
  database.rs             SQLite (registry, settings, history, backup index, secret refs)
  commands.rs             the Tauri command surface
  build.rs + windows/     application manifest wiring (see Troubleshooting)
src-tauri/tests/          integration tests
```
