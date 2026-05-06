# Backlog

Deferred improvements, bug fixes, and future features for implementation.

---

## Bug Fixes

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

## UX Improvements

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

## Security Items

### WebSocket Authentication

**Priority:** Medium  
**Affected area:** `src-tauri/src/routes/sessions.rs`

**Problem:** WebSocket connections to `/api/sessions/{id}/terminal` aren't authenticated. Any local process could connect and interact with the agent.

**Solution:** Generate a per-session token at creation time, require it as a query parameter on WebSocket upgrade. Frontend includes it automatically.

---

### Resize Message Validation

**Priority:** Low  
**Affected area:** `src-tauri/src/pty_bridge.rs`

**Problem:** The resize control message (`\x01` + JSON) has no validation on rows/cols values. Malicious input could set extreme values.

**Solution:** Clamp rows to 1–500, cols to 1–1000 before calling `master.resize()`.

---

## New Features

### Sandbox Management Tab

**Priority:** Medium  
**Affected area:** New page + route + backend endpoint

**Description:** Add a "Sandboxes" tab in the sidebar (above System Settings) that displays the output of `sbx ls` in a table format. Each sandbox row shows name, agent, status, workspace, and published ports. Actions per sandbox: Start (resume), Stop, Remove. This provides direct sandbox management independent of sessions — useful for sandboxes created outside the app or orphaned sandboxes.

**Implementation:**
- New `SandboxesPage` component
- Backend: `GET /api/sandboxes` already exists (calls `sbx ls --json`)
- Add `POST /api/sandboxes/{name}/start` (calls `sbx run <name>` non-interactively or just starts it)
- Existing `POST /api/sandboxes/{id}/stop` and `DELETE /api/sandboxes/{id}` may need adjustment
- Add route to sidebar navigation

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

### Custom Application Icons and Title Graphics

**Priority:** Medium  
**Affected area:** `src-tauri/icons/`, `src/components/Sidebar.tsx`, `index.html`, `src-tauri/tauri.conf.json`

**Description:** Replace the default Tauri placeholder icons and the text-only "Beachead" title in the sidebar header with custom branded graphics. Includes app icon (taskbar, dock, window title bar), sidebar logo/wordmark, and installer icons.

**Required assets:**

App icons (replace files in `src-tauri/icons/`):
| File | Size | Format | Used by |
|------|------|--------|---------|
| `32x32.png` | 32×32 px | PNG | Windows taskbar, Linux tray |
| `128x128.png` | 128×128 px | PNG | Linux app launcher, Tauri default |
| `128x128@2x.png` | 256×256 px | PNG | HiDPI Linux |
| `icon.ico` | Multi-size (16, 32, 48, 256) | ICO | Windows executable icon, taskbar |
| `icon.icns` | Multi-size (16–1024) | ICNS | macOS dock, Finder, Spotlight |

Additional recommended sizes for the ICO file:
- 16×16, 24×24, 32×32, 48×48, 64×64, 256×256

Additional recommended sizes for the ICNS file:
- 16×16, 32×32, 64×64, 128×128, 256×256, 512×512, 1024×1024

Sidebar logo:
| Asset | Size | Format | Location |
|-------|------|--------|----------|
| Logo (full) | ~120×30 px | SVG or PNG | Sidebar header at normal width |
| Logo (icon only) | ~24×24 px | SVG or PNG | Sidebar header when collapsed/narrow |
| Logo (dark variant) | Same as above | SVG or PNG | If primary logo doesn't work on dark bg |

Other:
| Asset | Format | Location |
|-------|--------|----------|
| Favicon | 32×32 PNG or SVG | `index.html` `<link rel="icon">` |
| Installer banner (optional) | 493×58 px BMP | Windows MSI installer header |
| DMG background (optional) | 660×400 px PNG | macOS DMG window background |

**Files to update:**
- `src-tauri/icons/` — All icon files listed above
- `src/components/Sidebar.tsx` — Replace `<h1>Beachead</h1>` with `<img>` logo
- `index.html` — Update `<title>` and add favicon `<link>`
- `src-tauri/tauri.conf.json` — Verify icon paths in `bundle.icon` array

**Considerations:**
- Source artwork should be at least 1024×1024 for downscaling
- Use `cargo tauri icon <source.png>` to auto-generate all required sizes from a single source
- Sidebar logo should degrade gracefully at narrow widths (show icon-only below ~140px)
- Ensure sufficient contrast on both light and dark backgrounds

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

### Multi-Workspace Mounts

**Priority:** Medium  
**Effort:** Low  
**Affected area:** Persona form, kit generator, session manager

**Description:** sbx supports multiple workspace paths as positional args (`sbx run agent /path1 /path2`). Currently we only support one. Allow personas to specify additional workspaces (read-only or read-write).

**Implementation:**
- Add `additional_workspaces` field to Persona (array of {path, read_only} objects)
- Update persona form with dynamic list for extra workspaces
- Session manager passes extra paths as positional args to `sbx create`
- Append `:ro` suffix for read-only mounts per sbx docs

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
