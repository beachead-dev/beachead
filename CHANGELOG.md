# Changelog

All notable changes to Beachead will be documented here.

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
