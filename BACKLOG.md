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
