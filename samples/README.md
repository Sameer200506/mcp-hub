# Samples

## `registry.sample.json`

An importable registry document in exactly the format **Servers → Export** produces. It contains
no credentials and points at example endpoints (`*.example.test`) that are safe to leave in place.

How to use it:

1. **Servers → Import** → select this file by its absolute path (for example
   `C:\Users\<you>\Desktop\PROJECTS\MCP HUB\samples\registry.sample.json`).
2. Choose the mode:
   * `merge` — adds entries and updates entries with a matching name;
   * `replace` — deletes the existing registry first, then imports (the app warns and lists what it
     removed).
3. Open a server and check **Targets** before synchronizing: it decides which harnesses receive it.

What each entry demonstrates:

| Entry | Demonstrates |
| --- | --- |
| `filesystem` | stdio definition with an environment value, a working directory, and multiple targets |
| `github` | remote (Streamable HTTP) definition with an `{env:GITHUB_TOKEN}` reference in a header — the value is never copied into a config file by the app |
| `legacy-search` | legacy SSE transport targeted only at harnesses that support it |
| `retired-demo` | a disabled entry (`enabled: false`) that stays in the registry but is written as disabled |

Notes:

* `id` values in the file are only used for matching during import; missing or conflicting ids are
  replaced with new UUIDs.
* `kind` can be `literal`, `envRef` (`{env:NAME}` / `{file:path}`) or `secret`. This sample avoids
  `secret` on purpose: a secret row is imported without a value only when a value already exists in
  your Windows Credential Manager, otherwise the import refuses that entry instead of storing an
  empty credential.
* After importing, run **Preview sync** and read the warnings: they name every field a harness
  cannot express (for example the `cwd` that Cline and Antigravity drop).
