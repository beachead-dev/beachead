# Backlog

Deferred improvements, bug fixes, and future features for implementation.

---

## Bug Fixes

### ~~Duplicate Primary Workspace Detection~~ ✅ FIXED

**Fixed:** 2026-05-10  
**Affected area:** `src-tauri/src/persona_manager.rs`, `src-tauri/src/db_ops.rs`

**What was done:** Added `persona_with_workspace_path()` query to `db_ops.rs` that checks if another persona already uses a given primary workspace path. Added uniqueness validation in `PersonaManager::create()` and `PersonaManager::update()`. Returns a clear error: "Workspace path is already used by persona '<name>'. Each persona must have a unique primary workspace." The constraint applies only to the primary workspace — future additional/secondary workspaces can be shared across personas.

---

### ~~"Documentation" Menu Text Overlaps Content in Help~~ ✅ FIXED

**Fixed:** 2026-05-15  
**Affected area:** `src/styles.css` (help page layout)

**What was done:** Fixed the help page CSS layout. Changed `min-height: calc(100vh - 0px)` to `height: calc(100vh)` with `min-height: 0` and `overflow: hidden` on the container. Added `overflow-y: auto` to the sidebar to prevent content overflow. Added `min-height: 0` to `.help-content` (required for flex children with overflow). Added `position: relative; z-index: 1` to the sidebar title to ensure it stays above scrolling content.

---

### ~~MCP Container Bearer Token Not Passed to Container~~ ✅ FIXED

**Fixed:** 2026-05-10  
**Affected area:** `src-tauri/src/mcp_container_manager.rs`

**What was done:** Changed `"BEACHEAD_BEARER_TOKEN=".to_string()` to `format!("BEACHEAD_BEARER_TOKEN={}", bearer_token)`. Auth middleware now wired into server.py. Token passed via URL query parameter (`?token=<value>`) for MCP client compatibility. Docker image rebuilt and verified.

---

### OAuth Flow Non-Functional (Interactive Command in Non-Interactive Context)

**Priority:** Medium  
**Affected area:** `src-tauri/src/sbx.rs`, `src-tauri/src/credential_manager.rs`, `src-tauri/src/agent_manager.rs` (seed logic), `src/pages/AgentsPage.tsx`

**Problem:** The `sbx secret set -g openai --oauth` command is interactive — it opens a browser and waits for user completion. The backend runs it via `exec_multi_command` with piped stdio (non-interactive), so it always fails. Additionally, the agent seed logic uses insert-or-skip, so corrected `auth_methods` values never propagate to existing installs (violates steering rule #7 on upsert semantics).

**Two sub-problems:**
1. **OAuth button doesn't work:** The command needs a TTY or at minimum needs to capture the browser URL and present it to the user.
2. **Seed data doesn't update:** `seed_builtin_agents()` skips existing records. Auth method corrections (removing OAuth from Claude/Cursor/Droid/Gemini, keeping it only on Codex) never reach existing databases.

**Solution:**

1. **Fix seed logic (upsert):** Change `seed_builtin_agents()` to use upsert semantics — update `metadata` JSON for existing built-in agents on every startup. This ensures auth_methods, descriptions, mcp_config_path, and other metadata fields stay current across app updates.

2. **Make OAuth flow interactive via PTY:** 
   - When the user clicks "OAuth" for the `openai` service, spawn `sbx secret set -g openai --oauth` in a PTY (reuse `PtyBridge` infrastructure)
   - Open a small terminal modal/panel in the UI showing the command output (browser URL, status messages)
   - The command will print a URL and wait — user clicks the URL, completes auth in browser, command exits successfully
   - On exit (success or failure), close the modal and refresh the secrets list
   - This is the same pattern needed for any interactive sbx command (device flow for Kiro, OAuth for Codex)

3. **Frontend gating:** Only show the OAuth button for services where the agent's `auth_methods` includes `"oauth"` (already implemented). Remove the button entirely until the PTY-based flow is built, or show it disabled with a tooltip explaining it's not yet supported.

**Implementation order:**
1. Seed upsert (~30 lines in `agent_manager.rs` + migration or ALTER approach)
2. Interactive command modal (new component, ~100-150 lines frontend + backend endpoint that spawns PTY and returns WebSocket URL)
3. Wire OAuth button to the interactive modal instead of the current fire-and-forget POST

**Scope:** Medium feature. Seed upsert is quick. The interactive terminal modal is reusable for device flow (Kiro) and any future interactive sbx commands.

---

### ~~Kit allowedDomains Persists in Global Policy~~ ✅ RESOLVED

**Fixed:** 2026-05-17  
**Affected area:** Kit generator, policy management, session manager

**What was done:** The new sbx release introduced per-sandbox policy scoping. MCP port allow rules are now added with `sbx policy allow network <sandbox-name> localhost:<port>` instead of the global `-g` flag. Per-sandbox rules are automatically cleaned up when the sandbox is removed via `sbx rm`. The kit generator does NOT emit `network.allowedDomains` (was already removed earlier). The orchestrator no longer pollutes the global policy with per-session rules.

---

### New Session UI Design Broken

**Priority:** Medium  
**Affected area:** Session creation form/modal

**Problem:** The new session creation UI has layout or design issues that need fixing.

**Solution:** Review and fix the session launcher form layout, spacing, and responsiveness.

---

### PTY Exit → DB Status Sync

**Priority:** Low (workaround: app restart triggers recovery which corrects status)  
**Affected area:** `src-tauri/src/pty_bridge.rs`, `src-tauri/src/session_manager.rs`

**Problem:** When an agent process exits (crashes, auth failure, normal exit) while the app is running, the PTY broadcast reader detects EOF and sets the in-memory `PtySessionStatus::Stopped`, but the SQLite session record stays as "running". The UI continues showing the session as active until the next app restart.

**Solution (Option A):** Add an `mpsc::Sender<SessionId>` exit notification channel:
1. Create a `tokio::sync::mpsc::channel` in the session manager at startup
2. Pass the sender into `PtyBridge::spawn()` — store it in `PtySession`
3. When the reader thread detects EOF (process exit), send the `SessionId` through the channel
4. Session manager spawns a background task that listens on the receiver and calls `db_ops::update_session_status(conn, &id, &SessionStatus::Stopped, None)` for each exit notification
5. Optionally notify the frontend via a server-sent event or WebSocket message so the UI updates immediately

**Scope:** ~50 lines of Rust across pty_bridge.rs and session_manager.rs. No frontend changes required (the 3-second reconciliation fetch would pick up the status change, or add a push notification for instant UI update).

---

### Port Unpublish Missing Port Spec

**Priority:** Low  
**Affected area:** `src/pages/SessionsPage.tsx` (PortManagerView), `src-tauri/src/routes/sandboxes.rs`

**Problem:** The `handleUnpublish` function calls `api.del(/api/sandboxes/${sandboxId}/ports)` without specifying which port to unpublish. The backend DELETE endpoint needs the port spec to pass to `sbx ports <sandbox> --unpublish <spec>`.

**Solution:** Pass the port spec in the request body or as a query parameter. Update the frontend to send `{ port_spec: "${host_port}:${sandbox_port}/${protocol}" }` and the backend to extract and forward it.

---

### Dynamic Port Binding

**Priority:** Medium  
**Affected area:** `src-tauri/src/server.rs`

**Problem:** The Axum server is hardcoded to port 9876. If that port is in use, the server fails silently and the UI is non-functional.

**Solution:** Use port 0 (OS-assigned) or try a range of ports. Communicate the chosen port to the frontend via Tauri state or an environment variable injected into the WebView.

---

### sbx create -q Fragility

**Priority:** Low  
**Affected area:** `src-tauri/src/sbx.rs` (`extract_sandbox_name`)

**Problem:** The `-q` flag on `sbx create` may still output verbose text on some sbx versions. The `extract_sandbox_name` parser handles this but is pattern-based and fragile.

**Solution:** Monitor sbx CLI updates for output format changes. Consider using `--name` flag and trusting the name we provided rather than parsing stdout.

---

### sbx policy ls Text Parser Fragility

**Priority:** Low  
**Affected area:** `src-tauri/src/sbx.rs` (`parse_policy_text`)

**Problem:** The policy list parser depends on column alignment in `sbx policy ls` text output. Column widths could change between sbx versions.

**Solution:** Prefer `sbx policy ls --json` if available in future sbx versions. Current parser works but should be tested against new sbx releases.

---

### Graceful Shutdown

**Priority:** Medium  
**Affected area:** `src-tauri/src/main.rs`, `src-tauri/src/server.rs`

**Problem:** When the app closes, sandboxes keep running but sessions aren't cleanly marked in the DB. The next startup has to run full recovery.

**Solution:** Add a Tauri `on_exit` hook that marks all "running" sessions as "stopped" in the DB before shutdown. Don't stop the sandboxes (user may want to resume).

---

### Periodic Session Health Check

**Priority:** Low  
**Affected area:** `src-tauri/src/session_manager.rs`

**Problem:** Running sessions are only validated against `sbx ls` on startup. If a sandbox stops mid-session (OOM, crash), the UI doesn't know until restart.

**Solution:** Spawn a background task that periodically (every 30s) calls `sbx ls --json` and reconciles session statuses. Combined with the PTY exit sync (above), this provides belt-and-suspenders detection.

---

### Application Logging System

**Priority:** High  
**Affected area:** Backend (all modules), frontend, new log viewer UI

**Description:** Implement structured logging across the entire application. Currently errors go to stderr via `eprintln!` with no persistence, no log levels, and no way for users to view them. Need a unified logging system that captures app errors, sbx CLI output, Docker/container logs, and MCP server logs.

**Scope:**
- **App logs:** Replace all `eprintln!` with structured logging (e.g., `tracing` crate with file + console subscribers). Log levels: error, warn, info, debug. Persist to rotating log files in the app data directory.
- **sbx CLI logs:** Capture stdout/stderr from all `sbx` command invocations. Store per-session or per-operation. Include the command that was run, exit code, and full output.
- **Container logs:** Periodically capture `docker logs` output for MCP containers. Surface errors (container crashes, startup failures) to the user.
- **MCP server logs:** The Python MCP server already logs to stdout inside the container. Capture via `docker logs` and surface relevant errors.
- **Log viewer UI:** New page or panel where users can browse logs filtered by source (app, sbx, containers), level, and time range. Searchable. Auto-scrolling tail mode for live logs.
- **Log retention:** Configurable max size / max age. Auto-rotate old logs.

**Implementation notes:**
- Use the `tracing` crate (already standard in the Rust/Tokio ecosystem) with `tracing-subscriber` for formatting and `tracing-appender` for file output
- Frontend: new "Logs" page or tab in system settings area
- Consider `tauri-plugin-log` for cross-platform log file management
- Research: VS Code's Output panel, Docker Desktop's log viewer, Warp's error reporting for UX patterns

---

### Error Message UX Overhaul

**Priority:** Medium  
**Affected area:** Frontend error display components, backend error responses

**Description:** Current error messages have several UX problems: some are too verbose (full stack traces or raw CLI output shown to users), there's no way to dismiss/close error notifications, and error presentation is inconsistent across the app. Research how other desktop applications handle error display and implement a consistent pattern.

**Problems to fix:**
- Error toasts/banners that can't be closed (block UI or accumulate)
- Raw technical errors shown to users (e.g., full Docker error strings, SQL errors)
- No distinction between user-actionable errors ("install Docker") and internal errors ("unexpected state")
- No error history — once dismissed, errors are gone (ties into logging above)
- Inconsistent error placement (some inline, some modal, some toast)

**Research:**
- How VS Code handles errors (notification center with dismiss, "Show Details" expand, "Don't Show Again")
- How Docker Desktop shows errors (toast with action buttons, log link)
- How Slack/Discord handle connection errors (inline banner, auto-retry, manual retry button)

**Implementation:**
- Define error severity levels for the frontend: `info`, `warning`, `error`, `fatal`
- Create a notification/toast system with: auto-dismiss for info/warning, manual dismiss for errors, persistent banner for fatal
- Each error has: short user-friendly message, optional "Details" expandable with technical info, optional action button ("Retry", "Open Settings", "View Logs")
- Backend: structure error responses with `{ message: "...", details: "...", code: "...", actionable: bool }`
- Add a notification history panel (drawer or page) so dismissed errors can be reviewed later
- Tie into the logging system — "View Logs" button on errors links to the relevant log entry

---

## UX Improvements

### Move Templates Tab from Agents to Docker Page

**Priority:** Medium  
**Effort:** Low  
**Affected area:** `src/pages/AgentsPage.tsx`, `src/pages/DockerPage.tsx`

**Problem:** The Templates tab is currently under Agents, but templates are Docker sandbox images — they're a Docker resource, not an agent configuration. The Agents page should focus on agent types and credentials. The Docker page already manages sandboxes and containers, so templates belong there.

**Implementation:**
1. Remove from AgentsPage: delete `TemplateInfo` interface, `templates` state, `handleRemoveTemplate`, the templates tab button, and the `{view === "templates" && ...}` render block. Remove `api.get<TemplateInfo[]>("/api/templates")` from `fetchData`.
2. Add to DockerPage: create a `TemplatesTab` component following the same pattern as `SandboxesTab`/`ContainersTab` (usePolling, action handlers, table rendering). Add "templates" to the `DockerTab` type union and add the tab button.
3. Add "Clean up old images" button: identify templates with the same tag but different image IDs (stale versions from sbx updates). Button removes all but the newest for each tag. Uses `api.del(/api/templates/${encodeURIComponent(tag)})` — may need a backend change to target by image ID rather than tag if multiple share the same tag.
4. Update AgentsPage tab default to `"agents" | "credentials"` (remove "templates" from the union type).

**Note:** The `sbx template rm` command removes by tag, but multiple entries can share the same tag (different image IDs from updates). May need to investigate whether `sbx template rm` removes all entries for a tag or just one. If it removes all, the "clean up" button just removes and re-pulls. If it targets a specific image ID, the API may need a new parameter.

---

### No Visual Indicator When Agent Exits

**Priority:** Medium  
**Affected area:** `src/pages/SessionsPage.tsx`

**Problem:** When an agent process exits, the terminal just goes silent. No status change, no notification.

**Solution:** Depends on PTY exit sync (above). Once the DB status updates, the frontend reconciliation would move the session to "stopped". Could also show a banner in the terminal view: "Agent process exited."

---

### Session Rename

**Priority:** Low  
**Affected area:** Frontend + backend (new API endpoint)

**Problem:** No way to rename a session after creation.

**Solution:** Add `PUT /api/sessions/{id}/name` endpoint that updates the sandbox_id display name in the DB. Note: can't rename the actual sbx sandbox — this would be a display-only alias.

---

### Stop Confirmation

**Priority:** Low  
**Affected area:** `src/pages/SessionsPage.tsx`

**Problem:** No confirmation before stopping a session (only before removing). Accidental stop kills the agent process.

**Solution:** Add a Tauri native dialog confirmation before calling the stop API.

---

### Session Metadata Display

**Priority:** Low  
**Affected area:** `src/pages/SessionsPage.tsx`

**Problem:** No way to see session duration/uptime or which persona a session belongs to without context.

**Solution:** Add a tooltip or expandable detail row showing persona name, created time, duration, and sandbox status.

---

### Templates View Lacks Descriptive Information

**Priority:** Medium  
**Affected area:** `src/pages/AgentsPage.tsx` (templates section)

**Problem:** The templates list under Agents shows template names but provides no context about what each template is, what it contains, or when to use it. Users can see templates exist but can't understand their purpose without external knowledge.

**Solution:** Add descriptive information to each template entry — at minimum show the source sandbox/agent, creation date, and size. Ideally add a description field (either from `sbx template` metadata if available, or user-editable). Consider a detail/expand view that shows template contents or configuration summary.

---

### Support/Donate Link in Help and README

**Priority:** Medium  
**Affected area:** `src/pages/HelpPage.tsx`, `README.md`

**Description:** Add a "Support this project" link in the app's Help section and in the README. Links to a donation platform (GitHub Sponsors, Ko-fi, Buy Me a Coffee, or Open Collective — TBD).

**Implementation:**
- Add a "Support" card or link in the Help page with a brief message and external link
- Add a "Support" section to README.md with badge/link
- Choose donation platform (research options first)
- Optionally add a FUNDING.yml for GitHub Sponsors button on the repo

---

## Security Items

### Sandbox OAuth Opens Host Browser Without User Consent

**Priority:** Medium  
**Affected area:** Docker Sandboxes behavior (not directly controllable by Beachhead)

**Problem:** When an agent inside a sandbox initiates an OAuth flow (e.g., Claude Code running `/login`), the Docker Sandboxes proxy automatically opens a browser window on the host machine without any permission prompt from Beachhead. The user has no opportunity to approve or deny the URL before it opens.

This is a Docker Sandboxes architectural behavior — the sandbox daemon proxies URL-open requests from the microVM to the host. Beachhead has no hook into this because it happens below the `sbx` CLI layer.

**Risk:** A compromised or malicious agent could open arbitrary URLs on the host (phishing pages, exploit URLs) without user awareness. The sandbox network policy controls which domains the agent can *reach*, but not which URLs it can ask the host to *open in a browser*.

**Mitigation options:**
1. **Document the behavior** — Add a note in the Help page and persona creation flow explaining that agents with sandbox auth can open browser windows on the host.
2. **Network policy restriction** — Use `sbx policy deny network` to restrict which domains the sandbox can reach. This limits what OAuth endpoints are reachable but doesn't prevent the browser-open action itself.
3. **Monitor sbx releases** — Docker may add a permission prompt or URL allowlist for browser-open requests in future versions. Track their changelog.
4. **Pre-session warning** — Show a one-time warning when launching a session with an interactive-auth agent: "This agent may open browser windows on your host for authentication."

**Current status:** No fix available at the Beachhead level. This is a Docker Sandboxes platform limitation. Option 4 (pre-session warning) is the most actionable near-term mitigation.

---

### WebSocket Authentication

**Priority:** Medium  
**Affected area:** `src-tauri/src/routes/sessions.rs`

**Problem:** WebSocket connections to `/api/sessions/{id}/terminal` aren't authenticated. Any local process could connect and interact with the agent.

**Solution:** Generate a per-session token at creation time, require it as a query parameter on WebSocket upgrade. Frontend includes it automatically.

---

### ~~Resize Message Validation~~ ✅ FIXED

**Fixed:** 2026-05-15  
**Affected area:** `src-tauri/src/pty_bridge.rs`

**What was done:** Added `clamp(1, 500)` for rows and `clamp(1, 1000)` for cols in both the `resize()` public method and the WebSocket resize message handler in `attach_ws()`. Extreme or zero values from malicious input are now safely bounded before reaching the PTY.

---

### Docker Sandboxes Daemon Restart Detection and Handling

**Priority:** Medium  
**Affected area:** `src-tauri/src/sbx.rs`, `src-tauri/src/server.rs`, frontend (global banner component)

**Problem:** When Docker Sandboxes receives an update, all `sbx` commands fail with: `ERROR: ensure daemon: cannot prompt for restart: stdin is not a terminal; run the command in an interactive terminal to confirm the restart`. The app surfaces this as a generic sbx CLI error with no actionable guidance. The user has to know to open a terminal and run an sbx command interactively to trigger the restart prompt.

**Solution:** Detect the restart-needed state, show a persistent global warning banner, and provide a restart button that handles the full lifecycle.

**Implementation:**

1. **Detection (backend):**
   - In `sbx.rs`, pattern-match stderr for `cannot prompt for restart` (or `needs to restart`)
   - Add a new error variant `OrchestratorError::SbxRestartRequired` (or a flag on `SbxError`)
   - When any sbx command hits this error, set an `AtomicBool` or shared state flag (`sbx_restart_required`) in `AppState`
   - Add `GET /api/system/sbx-status` endpoint that returns the restart-required flag (polled by frontend or checked on any sbx error response)

2. **Restart endpoint (backend):**
   - Add `POST /api/system/sbx-restart` endpoint
   - Spawns an sbx command (e.g., `sbx version`) with `y\n` piped to stdin to confirm the restart
   - Waits for the command to complete (the daemon restarts, stopping all sandboxes)
   - After success: clears the `sbx_restart_required` flag, triggers session status reconciliation (mark all "running" sessions as "stopped" in DB)
   - Returns success/failure to frontend

3. **Warning banner (frontend):**
   - Global banner component rendered at the top of the layout (above all page content)
   - Shows when `sbx_restart_required` is true
   - Message: "Docker Sandboxes has been updated and needs to restart. All running sandboxes will be stopped."
   - "Restart Now" button triggers the restart endpoint
   - Loading state while restart is in progress
   - On success: dismiss banner, refresh session list
   - On failure: show error, keep banner visible

4. **Edge cases:**
   - Restart while sessions are running: warn user that active sessions will be terminated
   - Restart failure: surface the error, don't clear the flag
   - Multiple concurrent restart attempts: debounce/disable button during operation
   - Recovery after restart: session reconciliation marks sessions as stopped, UI refreshes via existing polling

**Scope:** ~150-200 lines backend (error detection, shared state, endpoint, stdin piping), ~80-100 lines frontend (banner component, API call, layout integration).

---

## New Features

### Delete Memory Data Option When Disabling Memory

**Priority:** Medium  
**Affected area:** `src-tauri/src/routes/personas.rs`, `src-tauri/src/mcp_container_manager.rs`, persona form UI

**Description:** When a user disables memory on a persona, offer a checkbox/switch to also delete the stored memory data (Docker volume). Currently disabling memory removes the container but preserves the volume, so re-enabling memory restores previous data.

**Implementation:**
- Add `delete_data: Option<bool>` field to `UpdatePersonaRequest` (only relevant when `memory_enabled` goes from true to false)
- If `delete_data` is true, call `docker volume rm beachead-memory-{persona_id}` after removing the container
- Add a `remove_volume` method to `McpContainerManager` using bollard's volume API
- Frontend: show a confirmation dialog with a "Delete stored memories" checkbox when toggling memory off

---

### ~~Docker Management Tab~~ ✅ DONE

**Completed:** 2026-05-12  
**Affected area:** New `/docker` page + backend endpoints + sidebar navigation

**What was done:** Full implementation of Docker Management Tab with two sub-tabs (Sandboxes and Containers). Sandboxes tab shows sandbox list from `sbx ls` with Name, Agent, Status, ID columns and Start/Stop/Remove actions. Containers tab shows all Docker containers (managed MCP containers by default, all containers with Show All toggle) with Persona Name, Image, Port, Status, Volume Name, Created Date columns and Start/Stop/Remove actions. Features: polling-based data freshness (10s interval), immediate refresh on actions and toggle changes, confirmation dialogs for destructive operations, volume deletion option for container removal, stale data indicator, managed/unmanaged filtering, proper button state derivation based on container status (running/stopped/exited/created). Backend: sandbox action endpoints with 30s timeout, container action endpoints via bollard, live Docker status enrichment, unmanaged container direct removal. 6 property-based tests, 35 unit tests, 16 backend integration tests.

---

### Steering Injection into Sandboxes

**Priority:** Medium  
**Affected area:** Kit generator, persona/session creation UI

**Description:** Allow users to inject custom steering/instructions into sandboxes as part of persona configuration or session creation. Steering content would be included in the generated kit's `memory` field or as an `initFile` that the agent reads on startup.

**Implementation:**
- Add a "Steering" text area to the persona form (markdown content)
- Add an optional per-session steering override in the session launcher
- Kit generator includes steering in `spec.yaml` `memory` field or as a file at `${WORKDIR}/.beachead/steering.md`
- Agent-specific: some agents read system prompts from files, others from environment variables — may need agent-specific injection paths

---

### Ghost Crab (OpenClaw in a Sandbox)

**Priority:** Low (research/exploration)  
**Affected area:** New feature — agent integration

**Description:** Run OpenClaw (open-source AI agent framework) inside a Docker Sandbox, managed by Beachead. This would be a custom agent type that uses an Agent Kit to package OpenClaw with its dependencies and configuration.

**Implementation:**
- Create an Agent Kit (`kind: agent`) that installs OpenClaw in the sandbox
- Register as a custom agent type in Beachead
- May need custom MCP server integration for OpenClaw's tool system
- Investigate OpenClaw's auth requirements and workspace expectations
- Consider whether OpenClaw needs its own credential flow or reuses existing provider keys

---

### Speech-to-Text Input (Long Term)

**Priority:** Low (long-term research project)  
**Affected area:** New subsystem — audio capture + transcription

**Description:** Voice input for agent interaction — speak commands/questions that get transcribed and sent to the terminal or a chat interface. Could run as a local webservice or inside its own sandbox.

**Implementation options:**
1. **Local webservice:** Run Whisper (or similar) locally, capture audio from browser via WebRTC, transcribe, inject into terminal stdin
2. **Sandbox-based:** Run a transcription service in its own Docker Sandbox, expose via MCP or HTTP, Beachead captures audio and forwards
3. **Cloud API:** Use OpenAI Whisper API or Google Speech-to-Text (requires network access, adds latency)

**Considerations:**
- Audio capture in Tauri WebView (getUserMedia API)
- Latency requirements for real-time vs push-to-talk
- Model size vs accuracy tradeoffs for local inference
- Privacy implications of cloud transcription
- Integration point: inject as terminal input? As a separate chat panel? As MCP tool input?


---

## Design / Branding

*(No open items)*

---

## Integrations

### Git Integration Panel

**Priority:** Low  
**Effort:** Low  
**Affected area:** New panel in SessionPanel, backend `sbx exec` calls

**Description:** Add a "Git" tab alongside Terminal/Files/Ports in the session panel. Shows current branch, recent commits, and diff summary for the mounted workspace. Runs `git` commands inside the sandbox via `sbx exec` and displays formatted results.

**Implementation:**
- New `GitView` component in SessionPanel
- Backend endpoint: `GET /api/sessions/{id}/git/status` → runs `sbx exec <sandbox> git status --porcelain`
- Additional endpoints for log, diff, branch list
- Display as formatted cards/tables, not raw terminal output

---

### ~~Repo Sync — Isolated Git Remote Management~~ ✅ DONE

**Completed:** 2026-05-14  

---

### Repo Sync — Configurable Check Interval UI

**Priority:** Low  
**Affected area:** `src/components/RepoSettingsPanel.tsx`, `src-tauri/src/repo_sync_manager.rs`

**Description:** Add a user-configurable "Check Interval" field to the per-repo settings panel so users can control how often the background checker polls for new commits. Currently the interval is stored in the DB (`check_interval_seconds`, default 300) but there's no UI to change it.

**Implementation:**
- Add a numeric input field to `RepoSettingsPanel` for check interval (in seconds)
- Include the value in the `UpdateRepoRequest` when saving settings
- **Important:** Add backend validation in `update_repo()` to reject values outside the 30–3600 second range (Req 16.8). Currently the field is accepted without range checking.
**Affected area:** New sidebar menu item, new pages, new backend module, DB schema additions, persona workspace config

**What was done:** Full implementation of isolated git remote management with two-directory architecture (workspace = no remotes, mirror = has remotes + credentials). Git CLI wrapper with timeout, error classification, and credential injection. GIT_ASKPASS credential helper binary with OS keyring integration. Secret scanner for pre-push detection of .env files, private keys, and API tokens. Full sync operations: pull from agent, push to remote, fetch from remote, push to agent. Commit review with cherry-pick and squash support. Background sync status checker with sidebar notification badge. Per-repo configuration (branch strategy, attribution, secret scan mode). Frontend page with scan, enable, sync operations, and settings panel. Export/import support (repos exported, credentials excluded). Property-based tests for git CLI argument building, error classification, and secret scanner pattern matching.

---

### Session Output Logging / Export

**Priority:** Medium  
**Effort:** Low  
**Affected area:** `src-tauri/src/pty_bridge.rs`, new export endpoint

**Description:** Save terminal output to a file (markdown or plain text) for documentation, review, or sharing. The broadcast channel already has all output flowing through it — just add a subscriber that writes to disk.

**Implementation:**
- Add an optional file-writing subscriber to the broadcast channel per session
- Backend endpoint: `POST /api/sessions/{id}/export` → returns the saved transcript
- Frontend: "Export" button in session panel that downloads the file
- Format options: plain text (raw terminal), or stripped ANSI codes for clean markdown

---

### Desktop Notifications

**Priority:** Low  
**Effort:** Low  
**Affected area:** Frontend + `@tauri-apps/plugin-notification`

**Description:** System desktop notifications for key events: agent process exits, session stops unexpectedly, long-running task completes (detected by terminal going idle after activity).

**Implementation:**
- Install `tauri-plugin-notification`
- Trigger notifications from the frontend when session status changes to "stopped" unexpectedly
- Optional: detect terminal idle (no output for N seconds after sustained activity) as "task complete" heuristic
- User preference to enable/disable in System Settings

---

### MCP Tool Marketplace / Registry

**Priority:** Medium  
**Effort:** Medium  
**Affected area:** New UI section, persona MCP server management

**Description:** A curated list of known MCP servers (filesystem, GitHub, Slack, databases, web search, etc.) that users can one-click add to a persona. Builds on existing additional MCP server entries per persona.

**Implementation:**
- Bundled JSON registry of known MCP servers with name, URL template, description, required auth
- UI: browsable/searchable list in the persona form's MCP section
- "Add to persona" button pre-fills the MCP server entry fields
- Could later support fetching registry updates from a remote URL

---

### Session Quick Launch Presets

**Priority:** Low  
**Effort:** Low  
**Affected area:** New DB table, session launcher UI

**Description:** Save a persona + session name pattern + optional steering as a "quick launch" preset. One click to start a pre-configured session without going through the launcher form.

**Implementation:**
- New `presets` table: id, name, persona_id, session_name_template, steering
- UI: "Save as preset" button after launching, preset list in session sidebar header
- Session name template supports `{n}` for auto-incrementing number
- Presets appear as quick-action buttons above the session list

---

### Clipboard Bridge

**Priority:** Low  
**Effort:** Medium  
**Affected area:** `src/pages/SessionsPage.tsx` (TerminalView), Tauri clipboard API

**Description:** Bidirectional clipboard between host and terminal. xterm.js handles selection-to-clipboard natively, but pasting from host clipboard into the terminal needs explicit handling in Tauri's WebView.

**Implementation:**
- Install `@tauri-apps/plugin-clipboard-manager`
- Override xterm.js paste handler to read from system clipboard via Tauri API
- Add right-click context menu with Copy/Paste options
- Handle Ctrl+Shift+V (Linux) and Cmd+V (macOS) keyboard shortcuts
- Ensure binary data (non-UTF8) is handled gracefully

---

## Project Operations

### ~~Licensing Model Decision~~ ✅ DONE

**Completed:** 2026-05-11  
**Decision:** MIT License with voluntary donation model (Buy Me a Coffee / GitHub Sponsors / similar). Maximum adoption, zero friction, community-friendly. LICENSE file added to repo root.

---

### Build Process and CI/CD

**Priority:** High  
**Effort:** Medium  
**Affected area:** New CI config files, release automation

**Description:** Set up automated build, test, and release pipeline. Decide on hosting (GitHub, AWS CodeCommit, or both).

**Options to evaluate:**

| Platform | Pros | Cons |
|----------|------|------|
| **GitHub Actions** | Free for open source, marketplace actions for Tauri, community standard | Costs for private repos, Microsoft-owned |
| **AWS CodeCommit + CodePipeline** | Already in AWS ecosystem, private by default | Less community tooling, more setup |
| **Both** (mirror) | GitHub for community/CI, CodeCommit for private/backup | Maintenance of two remotes |

**Pipeline stages needed:**
1. **PR checks:** `cargo test`, `cargo clippy`, `npx tsc --noEmit`, `npx vitest run`
2. **Build matrix:** Linux x86_64, macOS x86_64, macOS ARM64, Windows x86_64
3. **Release:** Tag-triggered, builds all platforms, uploads artifacts
4. **Signing:** Code signing for macOS (.dmg) and Windows (.msi)

**Tauri-specific:**
- Use `tauri-apps/tauri-action` GitHub Action for cross-platform builds
- macOS requires Apple Developer certificate for notarization
- Windows requires code signing certificate for SmartScreen trust
- Linux builds need the webkit2gtk dev packages in CI

**Action items:**
1. Decide on primary hosting platform
2. Set up CI config (`.github/workflows/ci.yml` or `buildspec.yml`)
3. Configure release workflow with artifact upload
4. Set up code signing (if distributing publicly)
5. Document the build/release process in CONTRIBUTING.md

---

### Linter and Formatter Configuration

**Priority:** Medium  
**Effort:** Low (config setup) + Medium (initial warning triage)  
**Affected area:** CI pipeline, all source files

**Description:** Add static analysis tooling for all three languages in the project. Should be done alongside CI/CD setup so enforcement is automated from day one.

**Tools to add:**
- **Rust — Clippy:** Add `#![warn(clippy::pedantic, clippy::nursery, clippy::unwrap_used)]` to `main.rs`. Add `clippy.toml` if needed for project-specific allows. Run `cargo clippy -- -D warnings` in CI. Expect 100+ initial warnings to triage (most will be `module_name_repetitions`, `missing_errors_doc`, `must_use_candidate`).
- **Rust — rustfmt:** Already added `rustfmt.toml`. Run `cargo fmt --check` in CI.
- **TypeScript — ESLint:** Add `eslint` + `@typescript-eslint/eslint-plugin` + `eslint-plugin-react-hooks`. Configure `eslint.config.mjs`. Add `"lint": "eslint src --ext .ts,.tsx"` to package.json. Expect ~50 initial warnings (unused vars in tests, missing return types).
- **Python — Ruff:** Add `[tool.ruff]` section to `pyproject.toml` with `select = ["E", "F", "B", "S", "I", "ANN"]`. Run `ruff check` in CI. Expect minimal issues (codebase is already clean).

**Implementation order:**
1. Set up CI/CD first (prerequisite — no point adding configs without enforcement)
2. Add ruff config (Python — fewest expected issues, quick win)
3. Add ESLint config (TypeScript — moderate triage)
4. Add Clippy enforcement (Rust — most triage, do last)

**Scope:** Config files are 5 minutes each. Initial warning triage is 2–4 hours total across all three.

---

### RepoSyncManager Encapsulation Refactor

**Priority:** Low  
**Effort:** Low-Medium (30–60 minutes)  
**Affected area:** `src-tauri/src/repo_sync_manager.rs`, `src-tauri/src/routes/repo_sync.rs`

**Problem:** All fields on `RepoSyncManager` are `pub`, violating Rust API Guidelines (C-STRUCT-PRIVATE). This exposes internal state — notably `check_handle: Option<JoinHandle<()>>` which lets callers abort the background sync task, and `mirrors_dir: RwLock<PathBuf>` which lets callers bypass validation in `update_mirrors_dir()`.

**Solution:**
1. Make all fields `pub(crate)` or private
2. Add getter methods where external access is needed: `db()`, `git()`, `get_mirrors_dir()`, `get_cached_status()`
3. Update route handlers in `routes/repo_sync.rs` to use getters instead of direct field access
4. Keep `cached_status` accessible via a read-only getter (the `DashMap` is already concurrent-safe)

**Risk:** Low. All callers are within the same crate. This is a mechanical refactor with no behavior change.

---

### Website (beachead.net)

**Priority:** Medium  
**Effort:** Medium  
**Affected area:** External — new web project

**Description:** Create a public-facing website at beachead.net for the application. Hosting on AWS Amplify (or similar static hosting).

**Expected content:**
- **Landing page:** Hero section with tagline, key features, screenshot/demo
- **Download page:** Platform-specific download links (.deb, .AppImage, .dmg, .msi)
- **Documentation:** Mirror of in-app help content (or link to GitHub docs)
- **Getting started guide:** Prerequisites, install, first session walkthrough
- **Changelog:** Release notes per version
- **About/Contact:** Project info, maintainer, links to repo

**Technical stack options:**
| Option | Pros | Cons |
|--------|------|------|
| **AWS Amplify + Next.js/Astro** | Serverless, scales, custom domain easy | AWS costs (minimal for static) |
| **GitHub Pages + Hugo/Jekyll** | Free, simple, auto-deploys from repo | Limited to static, no server functions |
| **Cloudflare Pages + Astro** | Free tier generous, fast CDN, easy DNS | Another vendor account |

**Action items:**
1. Choose static site generator (Astro recommended — modern, fast, markdown-friendly)
2. Set up AWS Amplify with beachead.net domain
3. Design landing page (can reuse README content as starting point)
4. Set up auto-deploy from a `website/` directory or separate repo
5. Add download links once CI/CD produces release artifacts
6. Set up analytics (privacy-respecting: Plausible or Fathom)


---

## Completed

### Custom Application Icons and Title Graphics

**Completed:** 2026-05-06  
**Affected area:** `src-tauri/icons/`, `src/components/Sidebar.tsx`, `index.html`, `src/styles.css`

**What was done:**
- Generated all Tauri app icon sizes from 1024×1024 source (`icon-light.png`) via `cargo tauri icon`
- Replaced text-only `<h1>Beachead</h1>` in sidebar with branded logo images
- Sidebar shows transparent worldmark (wide logo with name) when expanded, "BH" icon when collapsed (<140px)
- Logo/icon switches between light and dark variants based on active theme (light/dark/system)
- Added 32×32 favicon to `index.html`
- Source assets stored in `assets/branding/` for future use
- CSS container queries handle responsive logo/icon switching

---

### Multi-Workspace Mounts

**Completed:** 2026-05-11  
**Affected area:** DB schema, persona manager, session manager, sbx CLI, export/import, frontend (PersonasPage + SessionsPage)

**What was done:** Full implementation of multiple workspace mounts per persona. Added `additional_workspaces` table (migration v5) with FK cascade delete. Validation in PersonaManager: path canonicalization, null byte rejection, absolute path enforcement, existence check, sensitive directory warnings, duplicate detection, primary collision check, label validation (64 char max, no control chars). SbxCli passes additional paths as positional args with `:ro` suffix for read-only. SessionManager loads and passes workspaces at session start. ExportImportManager includes workspaces in backup/restore. Frontend: persona form with dynamic workspace list (directory picker, labels, read-only toggle, reorder), client-side duplicate detection, persona card shows all workspaces with labels/badges, Mounts tab in session panel. 12 property-based tests + 12 frontend tests.
