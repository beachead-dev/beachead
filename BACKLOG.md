# Backlog

Deferred improvements and bug fixes for future implementation.

---

## PTY Exit → DB Status Sync

**Type:** Bug fix  
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

## Port Unpublish Missing Port Spec

**Type:** Bug fix  
**Priority:** Low  
**Affected area:** `src/pages/SessionsPage.tsx` (PortManagerView), `src-tauri/src/routes/sandboxes.rs`

**Problem:** The `handleUnpublish` function calls `api.del(/api/sandboxes/${sandboxId}/ports)` without specifying which port to unpublish. The backend DELETE endpoint needs the port spec to pass to `sbx ports <sandbox> --unpublish <spec>`.

**Solution:** Pass the port spec in the request body or as a query parameter. Update the frontend to send `{ port_spec: "${host_port}:${sandbox_port}/${protocol}" }` and the backend to extract and forward it.

---

## WebSocket Disconnect on Detach (Resource Optimization)

**Type:** Enhancement  
**Priority:** Low  
**Affected area:** `src/pages/SessionsPage.tsx` (TerminalView)

**Problem:** When a session is detached (tab closed), the terminal panel stays mounted (hidden) to preserve content. The WebSocket connection stays open, consuming a server connection slot and bandwidth for output the user isn't watching.

**Solution:** Disconnect the WebSocket when the terminal is hidden, reconnect when shown again. Terminal content is preserved in xterm.js's buffer regardless of WebSocket state. On reconnect, any output produced while disconnected is lost (acceptable since the user wasn't watching). Could also add a "scrollback sync" mechanism that replays missed output from the broadcast channel's buffer on reconnect.
