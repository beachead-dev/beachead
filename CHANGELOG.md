# Changelog

All notable changes to Beachead will be documented here.

## [Unreleased]

## [0.17.1] - 2026-07-23

### Fixes

- **Session creation on sbx 0.35.0** — `sbx create` was invoked with `-q/--quiet`, which sbx 0.35.0 changed to suppress *all* output including the sandbox name, causing session start to fail with "sbx did not return a sandbox name." Removed `-q`; the sandbox name is now read from the standard `Created sandbox '<name>'` output. (Affects 0.17.0.)
- **Longer sandbox creation timeout** — raised the `sbx create` timeout from 90s to 300s so first-use agent image pulls on a cold cache or slow network don't time out.

## [0.17.0] - 2026-07-23

### Fixes

- **sbx 0.35.0 compatibility:** Policy listing and rule removal now use `sbx policy ls --json` instead of parsing the text table. sbx 0.35.0 changed the default `sbx policy ls` output to a summarized per-policy overview (moving the detailed rule table behind `--wide`), which broke the old parser — producing an empty or incorrect Policies list and breaking network rule removal. The legacy text parser has been retired.
- **Minimum sbx version for policy features:** Policy listing and removal now require sbx 0.35.0 or later. On older sbx versions these operations surface a clear version-requirement error instead of failing opaquely.
- **Default policy uses `sbx policy init`:** Setting the global default policy now calls `sbx policy init` instead of the deprecated `sbx policy set-default` alias (renamed in sbx 0.34.0), so it keeps working after the alias is removed. No user-facing change to the mode options (balanced / allow / deny).

### Maintenance

- **Removed dead `sbx` wrapper code:** Deleted the unused `SbxCli` methods `kit_add()`, `kit_inspect()` (and the `KitInspectResult` struct), `exec_it()`, and `secret_set_scoped()`, plus unused fields on the internal policy-listing struct. These had no callers — `kit_add`/`kit_inspect` were leftovers from an abandoned live-kit-update approach (the app applies kit changes on session restart, not via `sbx kit add`, which as of sbx 0.35.0 recreates the container). No user-facing behavior changes; `kit_validate()` (used for custom agent kits) is retained.

### Known Issues

- **Default Policy buttons temporarily disabled:** The Balanced / Deny All / Allow All buttons on the Policies page are disabled in this release. Under sbx 0.34.0+, `sbx policy init` is a one-time initialization, so re-invoking it to switch the baseline could error or reset the global policy and wipe custom rules. The buttons will be re-enabled with a corrected reset-then-init flow (with confirmation) in the next release. Network allow/deny rule management is unaffected.

## [0.1.6] - 2026-06-18

### Fixes

- **Frontend resource path in installed packages** — Tauri encodes `../dist` as `_up_/dist` inside the resource directory. The dist resolver now checks both `$RESOURCE/dist/` and `$RESOURCE/_up_/dist/` to locate the frontend files in installed .deb/.app/.msi packages.

## [0.1.5] - 2026-06-18

### Fixes

- **Token injection on all HTML paths** — `ServeDir` was serving `index.html` directly for explicit `/index.html` requests, bypassing token injection. Added explicit route so the file is always served through the injection handler regardless of request path.

## [0.1.4] - 2026-06-17

### Fixes

- **Frontend not loading on installed .deb/.app/.msi** — The frontend `dist/` files were not bundled in the installer package. Added `dist/` to Tauri's `bundle.resources` and updated the runtime path resolution to use Tauri's `resource_dir()` API, which correctly locates the files regardless of install location.

## [0.1.3] - 2026-06-17

### Security

- **API token authentication:** All `/api/*` routes (except `/api/health`) now require a per-launch bearer token. The token is generated fresh each launch (256-bit random) and injected into `index.html` at serve time. Prevents other local processes and malicious websites from driving the orchestrator via localhost.
- **CORS tightened to exact-origin allowlist:** Replaces the previous `starts_with` predicate that accepted look-alike origins like `http://localhost.attacker.com`. Production is unaffected (same-origin webview).
- **Secret scanner: MCP bearer token detection:** Pre-push secret scanning now catches the memory MCP bearer token (`host.docker.internal:<port>/mcp?token=...`) before it can be pushed to a remote via Repo Sync.

### Fixes

- **WebKitGTK cache-busting:** `index.html` is served with `Cache-Control: no-store` to prevent the webview from caching a stale page across launches (the token changes every launch).
- **Token meta injection order:** The auth token `<meta>` tag is injected first in `<head>` (before scripts) to guarantee it's in the DOM when module scripts evaluate.

## [0.1.2] - 2026-06-10

### Features

- Remove running sessions directly — new ✕ button on active tabs and detached sessions stops and removes in one action
- "Removing..." progress indicator with pulsing status dot while removal is in progress
- Detach button changed to ⏏ (eject) icon for clarity

### Fixes

- **sbx 0.32.0 compatibility:** Removed `-g/--global` flag from policy commands (global is now the default scope); use `--sandbox` for per-sandbox scoping
- **sbx 0.32.0 compatibility:** Updated policy list parser to handle new 6-column output format (STATUS column removed); backwards-compatible with 0.31.x
- **sbx 0.32.0 compatibility:** Policy rule removal now uses `--resource` instead of `--id` (the POLICY/RULE column value is no longer the internal rule ID)
- **sbx 0.32.0 compatibility:** Sandbox lookup in stop endpoint checks both `id` and `name` fields (0.32.0 dropped the `id` field from `sbx ls --json`)
- Extended `sbx create` timeout from 30s to 90s to handle first-launch image pulls
- Frontend shows progressive status messages during sandbox creation ("Creating sandbox..." → "Pulling sandbox image...")
- Removed stale Status column from Policies table UI
- Fixed CSS nth-child targeting for Resources column after Status removal

## [0.1.0] - 2026-05-25

Initial public release.

### Features

- Create and manage AI agent personas with per-persona configuration
- Launch sandboxed agent sessions in Docker Sandbox microVMs
- Per-persona long-term memory via local MCP server (Docker-managed)
- Global network policy management and traffic log
- Repo Sync — mount remote git repositories into agent workspaces
- Credential management via OS keychain (`sbx secret`)
- Multi-workspace mounts per session
- Save and restore sandbox templates
- Built-in agents: Claude Code, Codex, GitHub Copilot, Kiro, Cursor, Gemini
- Custom agent registration via kit reference
