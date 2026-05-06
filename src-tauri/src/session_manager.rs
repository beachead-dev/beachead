//! Session Manager: orchestrates the full session lifecycle.
//!
//! Coordinates kit generation, sandbox creation via sbx CLI, PTY spawning,
//! and session persistence in SQLite. Handles file uploads by routing to
//! workspace upload or `sbx cp` based on file path.

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;

use crate::db::Database;
use crate::db_ops;
use crate::error::OrchestratorError;
use crate::kit_generator::KitGenerator;
use crate::pty_bridge::PtyBridge;
use crate::sbx::{SbxCli, SbxRunArgs};
use crate::types::{
    Persona, PersonaId, Session, SessionId, SessionStatus, UploadMethod, UploadResult,
};
use crate::workspace_manager::WorkspaceManager;

/// Result of attempting to recover a single session on startup.
#[derive(Debug, Clone)]
pub enum RecoveryResult {
    /// Session was successfully recovered (PTY reattached).
    Recovered(SessionId),
    /// Session recovery failed.
    Failed {
        session_id: SessionId,
        reason: String,
    },
}

/// Credential error patterns detected in sbx run stderr output.
const CREDENTIAL_ERROR_PATTERNS: &[&str] = &[
    "unauthorized",
    "authentication",
    "credentials",
    "api key",
    "token",
];

/// The WebSocket server port used for terminal connections.
const WS_PORT: u16 = 9876;

/// Orchestrates the full session lifecycle: kit generation → sandbox creation
/// → PTY spawn → WebSocket relay. Tracks session state in SQLite.
pub struct SessionManager {
    db: Arc<Database>,
    sbx: Arc<SbxCli>,
    kit_generator: Arc<KitGenerator>,
    pty_bridge: Arc<PtyBridge>,
}

impl SessionManager {
    /// Create a new SessionManager with all required dependencies.
    pub fn new(
        db: Arc<Database>,
        sbx: Arc<SbxCli>,
        kit_generator: Arc<KitGenerator>,
        pty_bridge: Arc<PtyBridge>,
    ) -> Self {
        Self {
            db,
            sbx,
            kit_generator,
            pty_bridge,
        }
    }

    /// Start a new session for the given persona.
    ///
    /// Flow:
    /// 1. Get persona from DB (via persona_id)
    /// 2. Generate kit via KitGenerator::generate()
    /// 3. Build SbxRunArgs and call sbx.run()
    /// 4. If sbx.run() fails, check stderr for credential patterns → return MissingCredentials
    /// 5. Spawn PTY via PtyBridge::spawn() with `sbx exec -it <sandbox_id>`
    /// 6. Persist session in SQLite with status "running"
    /// 7. Return Session with ws_url like "ws://127.0.0.1:9876/api/sessions/{id}/terminal"
    ///
    /// Requirements: 3.1–3.4, 3.7, 3.11, 3.12
    pub async fn start(&self, persona_id: &PersonaId, name: Option<&str>) -> Result<Session, OrchestratorError> {
        // 1. Get persona from DB
        let persona = self.db.with_conn(|conn| db_ops::get_persona(conn, persona_id))?;

        // 2. Resolve agent identifier (needed for kit generation and sandbox creation)
        let agent = self.resolve_agent_identifier(&persona)?;

        // 3. Generate kit (includes agent-specific network domains)
        // TODO: Phase 2 — If persona.memory_enabled is true, look up the MCP container
        // for this persona to get port and bearer_token, then construct a McpConfig:
        //   let mcp_config = if persona.memory_enabled {
        //       // Query mcp_containers table for this persona_id
        //       // McpConfig { url: format!("http://host.docker.internal:{}", container.port),
        //       //             bearer_token: container.bearer_token, port: container.port }
        //       Some(mcp_config)
        //   } else { None };
        let kit_path = self.kit_generator.generate(&persona, None, Some(&agent))?;

        // 4. Create session record in "starting" state
        let session_id = SessionId::new();
        let now = Utc::now();
        let session = Session {
            id: session_id.clone(),
            persona_id: persona.id.clone(),
            sandbox_id: None,
            kit_path: Some(kit_path.clone()),
            status: SessionStatus::Starting,
            error_message: None,
            created_at: now,
            updated_at: now,
        };
        self.db.with_conn(|conn| db_ops::insert_session(conn, &session))?;

        // 5. Build args and call sbx create (creates sandbox without attaching)
        let run_args = SbxRunArgs {
            agent: agent.clone(),
            kit_paths: vec![kit_path.clone()],
            workspace: persona.workspace_path.clone(),
            name: name.map(|s| s.to_string()),
            template: None,
            agent_args: persona.agent_cli_args.clone(),
        };

        let sandbox_id = match self.sbx.create(&crate::sbx::SbxCreateArgs {
            agent: run_args.agent,
            kit_paths: run_args.kit_paths,
            workspace: run_args.workspace,
            name: run_args.name,
            template: run_args.template,
        }).await {
            Ok(id) => id,
            Err(OrchestratorError::SbxError(ref msg)) => {
                // Check stderr for credential error patterns
                if self.is_credential_error(msg) {
                    let guidance = format!(
                        "Missing credentials for agent. Please configure the required \
                         secrets via the Credentials page before starting a session. \
                         Error: {}",
                        msg
                    );
                    // Update session to failed
                    self.db.with_conn(|conn| {
                        db_ops::update_session_status(
                            conn,
                            &session_id,
                            &SessionStatus::Failed,
                            Some(&guidance),
                        )
                    })?;
                    return Err(OrchestratorError::MissingCredentials(guidance));
                }
                // Non-credential sbx error — mark session as failed
                self.db.with_conn(|conn| {
                    db_ops::update_session_status(
                        conn,
                        &session_id,
                        &SessionStatus::Failed,
                        Some(msg),
                    )
                })?;
                return Err(OrchestratorError::SbxError(msg.clone()));
            }
            Err(e) => {
                self.db.with_conn(|conn| {
                    db_ops::update_session_status(
                        conn,
                        &session_id,
                        &SessionStatus::Failed,
                        Some(&e.to_string()),
                    )
                })?;
                return Err(e);
            }
        };

        // Update session with sandbox_id
        self.db.with_conn(|conn| {
            db_ops::update_session_sandbox_id(conn, &session_id, &sandbox_id)
        })?;

        // 5. Spawn PTY via PtyBridge with `sbx run <sandbox_id>` to attach
        let sbx_path = self.sbx.path().to_string_lossy().to_string();
        let pty_bridge = self.pty_bridge.clone();
        let pty_session_id = session_id.clone();
        let sandbox_id_clone = sandbox_id.clone();
        tokio::task::spawn_blocking(move || {
            pty_bridge.spawn(
                pty_session_id,
                &sbx_path,
                &["run", &sandbox_id_clone],
            )
        })
        .await
        .map_err(|e| OrchestratorError::Internal(format!("PTY spawn task failed: {}", e)))??;

        // 6. Update session status to "running"
        self.db.with_conn(|conn| {
            db_ops::update_session_status(conn, &session_id, &SessionStatus::Running, None)
        })?;

        // 7. Return session with ws_url
        let session = self.db.with_conn(|conn| db_ops::get_session(conn, &session_id))?;
        Ok(session)
    }

    /// Get the WebSocket URL for a session's terminal.
    pub fn ws_url(session_id: &SessionId) -> String {
        format!(
            "ws://127.0.0.1:{}/api/sessions/{}/terminal",
            WS_PORT, session_id.0
        )
    }

    /// Stop a running session.
    ///
    /// Invokes `sbx stop` and updates session status to "stopped" in SQLite.
    ///
    /// Requirements: 3.5
    pub async fn stop(&self, session_id: &SessionId) -> Result<(), OrchestratorError> {
        let session = self.db.with_conn(|conn| db_ops::get_session(conn, session_id))?;

        if let Some(ref raw_sandbox_id) = session.sandbox_id {
            // Kill PTY first (ignore errors if already stopped)
            let pty_bridge = self.pty_bridge.clone();
            let sid = session_id.clone();
            let _ = tokio::task::spawn_blocking(move || pty_bridge.kill(&sid)).await;

            // Stop the sandbox (use extracted name in case of legacy garbage)
            let sandbox_name = crate::sbx::extract_sandbox_name(raw_sandbox_id);
            if !sandbox_name.is_empty() {
                if let Err(e) = self.sbx.stop(&sandbox_name).await {
                    eprintln!("Warning: sbx stop failed for '{}': {}", sandbox_name, e);
                }
            }
        }

        // Update session status
        self.db.with_conn(|conn| {
            db_ops::update_session_status(conn, session_id, &SessionStatus::Stopped, None)
        })?;

        Ok(())
    }

    /// Resume a stopped session.
    ///
    /// Reattaches to the sandbox by spawning a new PTY with `sbx run <sandbox_name>`.
    /// The sandbox must be in stopped state.
    pub async fn resume(&self, session_id: &SessionId) -> Result<(), OrchestratorError> {
        let session = self.db.with_conn(|conn| db_ops::get_session(conn, session_id))?;

        if session.status != SessionStatus::Stopped {
            return Err(OrchestratorError::Validation(format!(
                "Cannot resume session in '{}' state, must be 'stopped'",
                session.status
            )));
        }

        let sandbox_id = session.sandbox_id.as_ref().ok_or_else(|| {
            OrchestratorError::Internal("Session has no sandbox_id to resume".to_string())
        })?;

        // Spawn PTY with `sbx run <sandbox_name>` to reattach
        let sbx_path = self.sbx.path().to_string_lossy().to_string();
        let pty_bridge = self.pty_bridge.clone();
        let pty_session_id = session_id.clone();
        let sandbox_id_clone = sandbox_id.clone();
        tokio::task::spawn_blocking(move || {
            pty_bridge.spawn(
                pty_session_id,
                &sbx_path,
                &["run", &sandbox_id_clone],
            )
        })
        .await
        .map_err(|e| OrchestratorError::Internal(format!("PTY spawn task failed: {}", e)))??;

        // Update session status to running
        self.db.with_conn(|conn| {
            db_ops::update_session_status(conn, session_id, &SessionStatus::Running, None)
        })?;

        Ok(())
    }

    /// Remove a session completely.
    ///
    /// Invokes `sbx rm`, cleans up the kit directory, and updates session status
    /// to "removed" in SQLite.
    ///
    /// Requirements: 3.6, 3.10
    pub async fn remove(&self, session_id: &SessionId) -> Result<(), OrchestratorError> {
        let session = self.db.with_conn(|conn| db_ops::get_session(conn, session_id))?;

        // Kill PTY if still active (ignore errors)
        let pty_bridge = self.pty_bridge.clone();
        let sid = session_id.clone();
        let _ = tokio::task::spawn_blocking(move || pty_bridge.kill(&sid)).await;

        // Remove the sandbox (best-effort — if it fails, still clean up the session record)
        if let Some(ref raw_sandbox_id) = session.sandbox_id {
            let sandbox_name = crate::sbx::extract_sandbox_name(raw_sandbox_id);
            if !sandbox_name.is_empty() {
                if let Err(e) = self.sbx.rm(&sandbox_name).await {
                    eprintln!("Warning: sbx rm failed for '{}': {}", sandbox_name, e);
                    // Continue with session cleanup even if sandbox removal fails
                }
            }
        }

        // Clean up kit directory
        if let Some(ref kit_path) = session.kit_path {
            self.kit_generator.cleanup(kit_path)?;
        }

        // Update session status
        self.db.with_conn(|conn| {
            db_ops::update_session_status(conn, session_id, &SessionStatus::Removed, None)
        })?;

        Ok(())
    }

    /// List all sessions with their current status.
    ///
    /// Requirements: 4.6
    pub fn list(&self) -> Result<Vec<Session>, OrchestratorError> {
        self.db.with_conn(|conn| db_ops::list_sessions(conn))
    }

    /// Recover previously active sessions on startup.
    ///
    /// Queries SQLite for sessions with status "running" or "starting", then for each:
    /// 1. If no sandbox_id → mark as stopped (nothing to reattach to)
    /// 2. Try to spawn a PTY with `sbx exec -it <sandbox_id>`
    /// 3. If spawn succeeds → session is recovered (status stays "running")
    /// 4. If spawn fails → check `sbx ls --json` for the sandbox
    /// 5. If sandbox is stopped/missing → update session to "stopped", attempt `sbx rm`, log result
    ///
    /// Requirements: 5.1–5.7
    pub async fn recover_sessions(&self) -> Vec<RecoveryResult> {
        let active_sessions = match self.db.with_conn(|conn| db_ops::list_active_sessions(conn)) {
            Ok(sessions) => sessions,
            Err(e) => {
                eprintln!("Failed to query active sessions for recovery: {}", e);
                return vec![];
            }
        };

        let mut results = Vec::new();

        for session in active_sessions {
            let result = self.recover_single_session(&session).await;
            results.push(result);
        }

        // Also clean up stopped sessions whose sandboxes no longer exist
        self.cleanup_orphaned_stopped_sessions().await;

        results
    }

    /// Remove stopped session records whose sandboxes no longer exist in sbx ls.
    async fn cleanup_orphaned_stopped_sessions(&self) {
        // First, get the list of all existing sandboxes. If this fails, skip cleanup entirely.
        let existing_sandboxes = match self.sbx.ls_json().await {
            Ok(sandboxes) => sandboxes,
            Err(_) => return, // Can't verify — don't delete anything
        };

        let stopped_sessions = match self.db.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, persona_id, sandbox_id, kit_path, status, error_message, created_at, updated_at \
                 FROM sessions WHERE status = 'stopped'"
            )?;
            let sessions = stmt.query_map([], |row| {
                let kit_path_str: Option<String> = row.get(3)?;
                let status_str: String = row.get(4)?;
                let created_str: String = row.get(6)?;
                let updated_str: String = row.get(7)?;
                Ok(Session {
                    id: SessionId(row.get(0)?),
                    persona_id: PersonaId(row.get(1)?),
                    sandbox_id: row.get(2)?,
                    kit_path: kit_path_str.map(std::path::PathBuf::from),
                    status: status_str.parse().unwrap_or(SessionStatus::Stopped),
                    error_message: row.get(5)?,
                    created_at: chrono::DateTime::parse_from_rfc3339(&created_str)
                        .unwrap().with_timezone(&chrono::Utc),
                    updated_at: chrono::DateTime::parse_from_rfc3339(&updated_str)
                        .unwrap().with_timezone(&chrono::Utc),
                })
            })?.collect::<Result<Vec<_>, _>>()?;
            Ok(sessions)
        }) {
            Ok(sessions) => sessions,
            Err(_) => return,
        };

        for session in stopped_sessions {
            if let Some(ref sandbox_id) = session.sandbox_id {
                let sandbox_name = crate::sbx::extract_sandbox_name(sandbox_id);
                if sandbox_name.is_empty() {
                    // No valid sandbox name — remove orphan
                    let _ = self.db.with_conn(|conn| {
                        conn.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![session.id.0])
                            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
                        Ok(())
                    });
                    continue;
                }
                // Check if ANY sandbox matches by name or id
                let found = existing_sandboxes.iter().any(|sb| {
                    sb.name.as_deref() == Some(&sandbox_name)
                        || sb.id.as_deref() == Some(&sandbox_name)
                        // Also try matching the raw sandbox_id in case extract_sandbox_name altered it
                        || sb.name.as_deref() == Some(sandbox_id.as_str())
                        || sb.id.as_deref() == Some(sandbox_id.as_str())
                });
                if !found {
                    // Sandbox truly gone — delete session record
                    let _ = self.db.with_conn(|conn| {
                        conn.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![session.id.0])
                            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
                        Ok(())
                    });
                }
            } else {
                // No sandbox_id at all — remove orphan
                let _ = self.db.with_conn(|conn| {
                    conn.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![session.id.0])
                        .map_err(|e| OrchestratorError::Database(e.to_string()))?;
                    Ok(())
                });
            }
        }
    }

    /// Attempt to recover a single session.
    async fn recover_single_session(&self, session: &Session) -> RecoveryResult {
        let session_id = &session.id;

        // If no sandbox_id, we can't reattach
        let sandbox_id = match &session.sandbox_id {
            Some(id) => id.clone(),
            None => {
                let reason = "No sandbox_id recorded".to_string();
                // No sandbox to verify — delete the orphaned session record
                let _ = self.db.with_conn(|conn| {
                    conn.execute("DELETE FROM sessions WHERE id = ?1", rusqlite::params![session_id.0])
                        .map_err(|e| OrchestratorError::Database(e.to_string()))?;
                    Ok(())
                });
                return RecoveryResult::Failed {
                    session_id: session_id.clone(),
                    reason,
                };
            }
        };

        // First check if the sandbox is still running via `sbx ls --json`
        let sandbox_status = match self.check_sandbox_status(&sandbox_id).await {
            Ok(status) => status,
            Err(_) => {
                // Can't query sbx ls — don't make destructive decisions
                let reason = format!(
                    "Could not verify sandbox {} status (sbx ls failed); leaving session as-is",
                    sandbox_id
                );
                let _ = self.db.with_conn(|conn| {
                    db_ops::update_session_status(
                        conn,
                        session_id,
                        &SessionStatus::Stopped,
                        Some(&reason),
                    )
                });
                return RecoveryResult::Failed {
                    session_id: session_id.clone(),
                    reason,
                };
            }
        };

        match sandbox_status.as_deref() {
            Some("running") => {
                // Sandbox is running — attempt PTY reattachment
                let sbx_path = self.sbx.path().to_string_lossy().to_string();
                let pty_bridge = self.pty_bridge.clone();
                let pty_session_id = session_id.clone();
                let sandbox_id_for_pty = sandbox_id.clone();

                let spawn_result = tokio::task::spawn_blocking(move || {
                    pty_bridge.spawn(
                        pty_session_id,
                        &sbx_path,
                        &["exec", "-it", &sandbox_id_for_pty],
                    )
                })
                .await;

                let pty_ok = match spawn_result {
                    Ok(Ok(())) => true,
                    Ok(Err(_)) => false,
                    Err(_) => false,
                };

                if pty_ok {
                    // Ensure status is "running"
                    let _ = self.db.with_conn(|conn| {
                        db_ops::update_session_status(
                            conn,
                            session_id,
                            &SessionStatus::Running,
                            None,
                        )
                    });
                    RecoveryResult::Recovered(session_id.clone())
                } else {
                    let reason = format!(
                        "Sandbox {} is running but PTY reattachment failed",
                        sandbox_id
                    );
                    let _ = self.db.with_conn(|conn| {
                        db_ops::update_session_status(
                            conn,
                            session_id,
                            &SessionStatus::Failed,
                            Some(&reason),
                        )
                    });
                    RecoveryResult::Failed {
                        session_id: session_id.clone(),
                        reason,
                    }
                }
            }
            _ => {
                if sandbox_status.is_none() {
                    // Sandbox is truly missing — remove the session record entirely
                    let _ = self.db.with_conn(|conn| {
                        conn.execute(
                            "DELETE FROM sessions WHERE id = ?1",
                            rusqlite::params![session_id.0],
                        ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
                        Ok(())
                    });

                    // Attempt sbx rm cleanup (best-effort)
                    let _ = self.sbx.rm(&sandbox_id).await;

                    let reason = format!(
                        "Sandbox {} not found; session removed",
                        sandbox_id
                    );
                    RecoveryResult::Failed {
                        session_id: session_id.clone(),
                        reason,
                    }
                } else {
                    // Sandbox exists but is stopped — mark session as stopped (resumable)
                    let _ = self.db.with_conn(|conn| {
                        db_ops::update_session_status(
                            conn,
                            session_id,
                            &SessionStatus::Stopped,
                            Some("Sandbox stopped during recovery"),
                        )
                    });

                    let reason = format!(
                        "Sandbox {} is stopped; session marked as stopped",
                        sandbox_id
                    );
                    RecoveryResult::Failed {
                        session_id: session_id.clone(),
                        reason,
                    }
                }
            }
        }
    }

    /// Check the status of a sandbox via `sbx ls --json`.
    /// Returns:
    /// - Ok(Some(status)) if sandbox found
    /// - Ok(None) if sandbox definitively not in the list
    /// - Err if we couldn't query sbx ls (unreliable — don't act on this)
    async fn check_sandbox_status(&self, sandbox_id: &str) -> Result<Option<String>, OrchestratorError> {
        let sandboxes = self.sbx.ls_json().await?;
        for sb in sandboxes {
            if sb.id.as_deref() == Some(sandbox_id)
                || sb.name.as_deref() == Some(sandbox_id)
            {
                return Ok(sb.status);
            }
        }
        Ok(None) // sandbox not found in listing
    }

    /// Upload a file to a session's workspace or sandbox.
    ///
    /// Routes the file based on its source path:
    /// - If the file path is inside the persona's workspace → upload to workspace
    /// - Otherwise → use `sbx cp` to copy into the sandbox
    ///
    /// Requirements: 4.8, 4.9, 4.10, 4.11
    pub async fn upload_file(
        &self,
        session_id: &SessionId,
        filename: &str,
        content: &[u8],
        source_path: Option<&Path>,
    ) -> Result<UploadResult, OrchestratorError> {
        let session = self.db.with_conn(|conn| db_ops::get_session(conn, session_id))?;

        if session.status != SessionStatus::Running {
            return Err(OrchestratorError::Validation(
                "Cannot upload files to a session that is not running".to_string(),
            ));
        }

        let sandbox_id = session.sandbox_id.as_ref().ok_or_else(|| {
            OrchestratorError::Internal("Session has no sandbox_id".to_string())
        })?;

        // Get the persona to determine workspace path
        let persona = self
            .db
            .with_conn(|conn| db_ops::get_persona(conn, &session.persona_id))?;

        // Determine routing: workspace upload or sbx cp
        let use_workspace = source_path
            .map(|p| WorkspaceManager::is_path_inside_workspace(p, &persona.workspace_path))
            .unwrap_or(true); // Default to workspace upload if no source path given

        if use_workspace {
            // Upload to workspace uploads directory
            let _uploaded_path = WorkspaceManager::upload_to_workspace(
                &persona.workspace_path,
                filename,
                content,
            )?;
            // The sandbox path is relative to the workspace mount
            let sandbox_path = format!("/workspace/.beachead/uploads/{}", filename);
            Ok(UploadResult {
                sandbox_path,
                method: UploadMethod::Workspace,
            })
        } else {
            // Write to a temp file and use sbx cp
            let temp_dir = std::env::temp_dir();
            let temp_file = temp_dir.join(filename);
            std::fs::write(&temp_file, content).map_err(|e| {
                OrchestratorError::Internal(format!("Failed to write temp file: {}", e))
            })?;

            let src = temp_file.to_string_lossy().to_string();
            let dst = format!("{}:/workspace/{}", sandbox_id, filename);
            let result = self.sbx.cp(&src, &dst).await;

            // Clean up temp file
            let _ = std::fs::remove_file(&temp_file);

            result?;

            Ok(UploadResult {
                sandbox_path: format!("/workspace/{}", filename),
                method: UploadMethod::SbxCp,
            })
        }
    }

    /// Check if an error message contains credential-related patterns.
    fn is_credential_error(&self, message: &str) -> bool {
        let lower = message.to_lowercase();
        CREDENTIAL_ERROR_PATTERNS
            .iter()
            .any(|pattern| lower.contains(pattern))
    }

    /// Resolve the agent identifier for sbx run from the persona's agent type.
    fn resolve_agent_identifier(&self, persona: &Persona) -> Result<String, OrchestratorError> {
        let agent_type = self
            .db
            .with_conn(|conn| db_ops::get_agent_type(conn, &persona.agent_type_id))?;

        // Use sbx_agent if available (built-in agents), otherwise use kit_ref or name
        agent_type
            .sbx_agent
            .or(agent_type.kit_ref)
            .ok_or_else(|| {
                OrchestratorError::Validation(format!(
                    "Agent type '{}' has no sbx_agent or kit_ref configured",
                    agent_type.name
                ))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::db_ops;
    use crate::kit_generator::KitGenerator;
    use crate::pty_bridge::PtyBridge;
    use crate::sbx::SbxCli;
    use crate::types::{
        AgentMetadata, AgentType, AgentTypeId, AuthMethod, Persona, PersonaId, SessionStatus,
    };
    use std::fs;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use tempfile::TempDir;

    /// Helper to create a mock sbx script that succeeds and outputs a sandbox ID.
    fn create_mock_sbx_success(dir: &Path) -> PathBuf {
        let script_path = dir.join("sbx");
        let mut file = fs::File::create(&script_path).unwrap();
        writeln!(
            file,
            r#"#!/bin/sh
case "$1" in
    create)
        echo "sandbox-abc123"
        exit 0
        ;;
    run)
        # For PTY spawn (attaching to existing sandbox), just run cat to keep process alive
        exec cat
        ;;
    stop|rm)
        exit 0
        ;;
    cp)
        exit 0
        ;;
    *)
        echo "unknown command: $1" >&2
        exit 1
        ;;
esac
"#
        )
        .unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1));
        script_path
    }

    /// Helper to create a mock sbx script that fails with a credential error.
    fn create_mock_sbx_credential_error(dir: &Path) -> PathBuf {
        let script_path = dir.join("sbx");
        let mut file = fs::File::create(&script_path).unwrap();
        writeln!(
            file,
            r#"#!/bin/sh
case "$1" in
    create)
        echo "Error: unauthorized - invalid API key or credentials not configured" >&2
        exit 1
        ;;
    *)
        exit 0
        ;;
esac
"#
        )
        .unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1));
        script_path
    }

    /// Helper to create a mock sbx script that fails with a non-credential error.
    fn create_mock_sbx_generic_error(dir: &Path) -> PathBuf {
        let script_path = dir.join("sbx");
        let mut file = fs::File::create(&script_path).unwrap();
        writeln!(
            file,
            r#"#!/bin/sh
case "$1" in
    create)
        echo "Error: network timeout connecting to sandbox runtime" >&2
        exit 1
        ;;
    *)
        exit 0
        ;;
esac
"#
        )
        .unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1));
        script_path
    }

    /// Helper to set up a test environment with DB, persona, and agent type.
    fn setup_test_env(
        sbx_path: PathBuf,
    ) -> (Arc<Database>, Arc<SbxCli>, Arc<KitGenerator>, Arc<PtyBridge>, TempDir, TempDir) {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let sbx = Arc::new(SbxCli::with_path(sbx_path));

        let kit_dir = TempDir::new().unwrap();
        let kit_generator = Arc::new(KitGenerator::new(kit_dir.path().to_path_buf()));
        let pty_bridge = Arc::new(PtyBridge::new());

        let workspace_dir = TempDir::new().unwrap();

        (db, sbx, kit_generator, pty_bridge, kit_dir, workspace_dir)
    }

    /// Helper to insert a test agent type and persona into the DB.
    fn insert_test_persona(db: &Database, workspace_path: &Path) -> PersonaId {
        let agent_type_id = AgentTypeId("agent-1".to_string());
        let persona_id = PersonaId("persona-1".to_string());
        let now = Utc::now();

        db.with_conn(|conn| {
            let agent_type = AgentType {
                id: agent_type_id.clone(),
                name: "claude".to_string(),
                sbx_agent: Some("claude".to_string()),
                kit_ref: None,
                is_builtin: true,
                metadata: AgentMetadata {
                    required_secrets: vec!["anthropic".to_string()],
                    auth_methods: vec![AuthMethod::ApiKey],
                    description: "Claude Code agent".to_string(),
                    supports_interactive_auth: false,
                },
                created_at: now,
                updated_at: now,
            };
            db_ops::insert_agent_type(conn, &agent_type)?;

            let persona = Persona {
                id: persona_id.clone(),
                name: "test-persona".to_string(),
                agent_type_id,
                workspace_path: workspace_path.to_path_buf(),
                memory_enabled: false,
                agent_cli_args: vec![],
                mcp_servers: vec![],
                created_at: now,
                updated_at: now,
            };
            db_ops::insert_persona(conn, &persona)?;
            Ok(())
        })
        .unwrap();

        persona_id
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_start_session_success() {
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_success(sbx_dir.path());
        let (db, sbx, kit_generator, pty_bridge, _kit_dir, workspace_dir) =
            setup_test_env(sbx_path);

        let persona_id = insert_test_persona(&db, workspace_dir.path());

        let manager = SessionManager::new(
            db.clone(),
            sbx,
            kit_generator,
            pty_bridge.clone(),
        );

        let session = manager.start(&persona_id, None).await.unwrap();

        assert_eq!(session.status, SessionStatus::Running);
        assert_eq!(session.persona_id, persona_id);
        assert!(session.sandbox_id.is_some());
        assert_eq!(session.sandbox_id.as_deref(), Some("sandbox-abc123"));
        assert!(session.kit_path.is_some());

        // Verify ws_url format
        let ws_url = SessionManager::ws_url(&session.id);
        assert!(ws_url.starts_with("ws://127.0.0.1:9876/api/sessions/"));
        assert!(ws_url.ends_with("/terminal"));

        // Clean up PTY
        let pb = pty_bridge.clone();
        let sid = session.id.clone();
        let _ = tokio::task::spawn_blocking(move || pb.kill(&sid)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_start_session_credential_error() {
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_credential_error(sbx_dir.path());
        let (db, sbx, kit_generator, pty_bridge, _kit_dir, workspace_dir) =
            setup_test_env(sbx_path);

        let persona_id = insert_test_persona(&db, workspace_dir.path());

        let manager = SessionManager::new(db.clone(), sbx, kit_generator, pty_bridge);

        let result = manager.start(&persona_id, None).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            OrchestratorError::MissingCredentials(msg) => {
                assert!(msg.contains("credentials"));
            }
            other => panic!("Expected MissingCredentials error, got: {:?}", other),
        }

        // Verify session was persisted as failed
        let sessions = db.with_conn(|conn| db_ops::list_sessions(conn)).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Failed);
        assert!(sessions[0].error_message.is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_start_session_generic_error() {
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_generic_error(sbx_dir.path());
        let (db, sbx, kit_generator, pty_bridge, _kit_dir, workspace_dir) =
            setup_test_env(sbx_path);

        let persona_id = insert_test_persona(&db, workspace_dir.path());

        let manager = SessionManager::new(db.clone(), sbx, kit_generator, pty_bridge);

        let result = manager.start(&persona_id, None).await;
        assert!(result.is_err());

        match result.unwrap_err() {
            OrchestratorError::SbxError(_) => {}
            other => panic!("Expected SbxError, got: {:?}", other),
        }

        // Verify session was persisted as failed
        let sessions = db.with_conn(|conn| db_ops::list_sessions(conn)).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, SessionStatus::Failed);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_stop_session() {
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_success(sbx_dir.path());
        let (db, sbx, kit_generator, pty_bridge, _kit_dir, workspace_dir) =
            setup_test_env(sbx_path);

        let persona_id = insert_test_persona(&db, workspace_dir.path());

        let manager = SessionManager::new(
            db.clone(),
            sbx,
            kit_generator,
            pty_bridge.clone(),
        );

        // Start a session first
        let session = manager.start(&persona_id, None).await.unwrap();
        assert_eq!(session.status, SessionStatus::Running);

        // Stop it
        manager.stop(&session.id).await.unwrap();

        // Verify status updated
        let updated = db
            .with_conn(|conn| db_ops::get_session(conn, &session.id))
            .unwrap();
        assert_eq!(updated.status, SessionStatus::Stopped);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_remove_session() {
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_success(sbx_dir.path());
        let (db, sbx, kit_generator, pty_bridge, _kit_dir, workspace_dir) =
            setup_test_env(sbx_path);

        let persona_id = insert_test_persona(&db, workspace_dir.path());

        let manager = SessionManager::new(
            db.clone(),
            sbx,
            kit_generator,
            pty_bridge.clone(),
        );

        // Start a session first
        let session = manager.start(&persona_id, None).await.unwrap();
        let kit_path = session.kit_path.clone().unwrap();
        assert!(kit_path.exists());

        // Remove it
        manager.remove(&session.id).await.unwrap();

        // Verify status updated
        let updated = db
            .with_conn(|conn| db_ops::get_session(conn, &session.id))
            .unwrap();
        assert_eq!(updated.status, SessionStatus::Removed);

        // Verify kit directory was cleaned up
        assert!(!kit_path.exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_list_sessions() {
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_success(sbx_dir.path());
        let (db, sbx, kit_generator, pty_bridge, _kit_dir, workspace_dir) =
            setup_test_env(sbx_path);

        let persona_id = insert_test_persona(&db, workspace_dir.path());

        let manager = SessionManager::new(
            db.clone(),
            sbx,
            kit_generator,
            pty_bridge.clone(),
        );

        // Initially empty
        let sessions = manager.list().unwrap();
        assert!(sessions.is_empty());

        // Start a session
        let session = manager.start(&persona_id, None).await.unwrap();

        // Should have one session
        let sessions = manager.list().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, session.id);
        assert_eq!(sessions[0].status, SessionStatus::Running);

        // Clean up
        let pb = pty_bridge.clone();
        let sid = session.id.clone();
        let _ = tokio::task::spawn_blocking(move || pb.kill(&sid)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_upload_file_workspace_route() {
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_success(sbx_dir.path());
        let (db, sbx, kit_generator, pty_bridge, _kit_dir, workspace_dir) =
            setup_test_env(sbx_path);

        let persona_id = insert_test_persona(&db, workspace_dir.path());

        let manager = SessionManager::new(
            db.clone(),
            sbx,
            kit_generator,
            pty_bridge.clone(),
        );

        // Start a session
        let session = manager.start(&persona_id, None).await.unwrap();

        // Upload a file (source inside workspace)
        let source = workspace_dir.path().join("myfile.txt");
        fs::write(&source, "hello").unwrap();

        let result = manager
            .upload_file(&session.id, "myfile.txt", b"hello", Some(&source))
            .await
            .unwrap();

        assert_eq!(result.sandbox_path, "/workspace/.beachead/uploads/myfile.txt");
        assert!(matches!(result.method, UploadMethod::Workspace));

        // Clean up
        let pb = pty_bridge.clone();
        let sid = session.id.clone();
        let _ = tokio::task::spawn_blocking(move || pb.kill(&sid)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_upload_file_sbx_cp_route() {
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_success(sbx_dir.path());
        let (db, sbx, kit_generator, pty_bridge, _kit_dir, workspace_dir) =
            setup_test_env(sbx_path);

        let persona_id = insert_test_persona(&db, workspace_dir.path());

        let manager = SessionManager::new(
            db.clone(),
            sbx,
            kit_generator,
            pty_bridge.clone(),
        );

        // Start a session
        let session = manager.start(&persona_id, None).await.unwrap();

        // Upload a file from outside workspace (should use sbx cp)
        let other_dir = TempDir::new().unwrap();
        let source = other_dir.path().join("external.txt");
        fs::write(&source, "external data").unwrap();

        let result = manager
            .upload_file(&session.id, "external.txt", b"external data", Some(&source))
            .await
            .unwrap();

        assert_eq!(result.sandbox_path, "/workspace/external.txt");
        assert!(matches!(result.method, UploadMethod::SbxCp));

        // Clean up
        let pb = pty_bridge.clone();
        let sid = session.id.clone();
        let _ = tokio::task::spawn_blocking(move || pb.kill(&sid)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_upload_file_not_running_rejected() {
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_success(sbx_dir.path());
        let (db, sbx, kit_generator, pty_bridge, _kit_dir, workspace_dir) =
            setup_test_env(sbx_path);

        let persona_id = insert_test_persona(&db, workspace_dir.path());

        let manager = SessionManager::new(
            db.clone(),
            sbx,
            kit_generator,
            pty_bridge.clone(),
        );

        // Start and stop a session
        let session = manager.start(&persona_id, None).await.unwrap();
        manager.stop(&session.id).await.unwrap();

        // Try to upload — should fail
        let result = manager
            .upload_file(&session.id, "file.txt", b"data", None)
            .await;
        assert!(result.is_err());
        match result.unwrap_err() {
            OrchestratorError::Validation(msg) => {
                assert!(msg.contains("not running"));
            }
            other => panic!("Expected Validation error, got: {:?}", other),
        }
    }

    #[test]
    fn test_is_credential_error_detection() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_success(sbx_dir.path());
        let sbx = Arc::new(SbxCli::with_path(sbx_path));
        let kit_dir = TempDir::new().unwrap();
        let kit_generator = Arc::new(KitGenerator::new(kit_dir.path().to_path_buf()));
        let pty_bridge = Arc::new(PtyBridge::new());

        let manager = SessionManager::new(db, sbx, kit_generator, pty_bridge);

        // Should detect credential errors
        assert!(manager.is_credential_error("Error: unauthorized access"));
        assert!(manager.is_credential_error("authentication failed"));
        assert!(manager.is_credential_error("Missing credentials for service"));
        assert!(manager.is_credential_error("Invalid API key provided"));
        assert!(manager.is_credential_error("Token expired or invalid"));

        // Should NOT detect non-credential errors
        assert!(!manager.is_credential_error("network timeout"));
        assert!(!manager.is_credential_error("disk full"));
        assert!(!manager.is_credential_error("sandbox not found"));
    }

    #[test]
    fn test_ws_url_format() {
        let session_id = SessionId("test-session-123".to_string());
        let url = SessionManager::ws_url(&session_id);
        assert_eq!(
            url,
            "ws://127.0.0.1:9876/api/sessions/test-session-123/terminal"
        );
    }

    /// Helper to create a mock sbx script for recovery tests.
    /// - `exec` succeeds (spawns cat) to simulate successful reattachment
    /// - `ls` returns JSON with the given sandbox info
    /// - `rm` succeeds
    fn create_mock_sbx_recovery_success(dir: &Path) -> PathBuf {
        let script_path = dir.join("sbx");
        let mut file = fs::File::create(&script_path).unwrap();
        writeln!(
            file,
            r#"#!/bin/sh
case "$1" in
    exec)
        exec cat
        ;;
    ls)
        echo '[{{"id":"sandbox-recover-1","name":"sandbox-recover-1","status":"running"}}]'
        exit 0
        ;;
    rm)
        exit 0
        ;;
    *)
        exit 0
        ;;
esac
"#
        )
        .unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1));
        script_path
    }

    /// Helper to create a mock sbx script where exec fails (sandbox stopped).
    fn create_mock_sbx_recovery_stopped(dir: &Path) -> PathBuf {
        let script_path = dir.join("sbx");
        let mut file = fs::File::create(&script_path).unwrap();
        writeln!(
            file,
            r#"#!/bin/sh
case "$1" in
    exec)
        echo "Error: sandbox not running" >&2
        exit 1
        ;;
    ls)
        echo '[{{"id":"sandbox-stopped-1","name":"sandbox-stopped-1","status":"stopped"}}]'
        exit 0
        ;;
    rm)
        exit 0
        ;;
    *)
        exit 0
        ;;
esac
"#
        )
        .unwrap();
        file.sync_all().unwrap();
        drop(file);
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(1));
        script_path
    }

    /// Helper to insert a session directly into the DB with a given status and sandbox_id.
    fn insert_session_directly(
        db: &Database,
        session_id: &str,
        persona_id: &PersonaId,
        sandbox_id: Option<&str>,
        status: SessionStatus,
    ) {
        let now = Utc::now();
        let session = Session {
            id: SessionId(session_id.to_string()),
            persona_id: persona_id.clone(),
            sandbox_id: sandbox_id.map(|s| s.to_string()),
            kit_path: None,
            status,
            error_message: None,
            created_at: now,
            updated_at: now,
        };
        db.with_conn(|conn| db_ops::insert_session(conn, &session))
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_recover_sessions_success() {
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_recovery_success(sbx_dir.path());
        let (db, sbx, kit_generator, pty_bridge, _kit_dir, workspace_dir) =
            setup_test_env(sbx_path);

        let persona_id = insert_test_persona(&db, workspace_dir.path());

        // Insert a "running" session directly into DB
        insert_session_directly(
            &db,
            "session-recover-1",
            &persona_id,
            Some("sandbox-recover-1"),
            SessionStatus::Running,
        );

        let manager = SessionManager::new(
            db.clone(),
            sbx,
            kit_generator,
            pty_bridge.clone(),
        );

        let results = manager.recover_sessions().await;
        assert_eq!(results.len(), 1);
        assert!(matches!(&results[0], RecoveryResult::Recovered(id) if id.0 == "session-recover-1"));

        // Verify session is still running in DB
        let session = db
            .with_conn(|conn| db_ops::get_session(conn, &SessionId("session-recover-1".to_string())))
            .unwrap();
        assert_eq!(session.status, SessionStatus::Running);

        // Clean up PTY
        let pb = pty_bridge.clone();
        let sid = SessionId("session-recover-1".to_string());
        let _ = tokio::task::spawn_blocking(move || pb.kill(&sid)).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_recover_sessions_sandbox_stopped() {
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_recovery_stopped(sbx_dir.path());
        let (db, sbx, kit_generator, pty_bridge, _kit_dir, workspace_dir) =
            setup_test_env(sbx_path);

        let persona_id = insert_test_persona(&db, workspace_dir.path());

        // Insert a "running" session with a sandbox that is actually stopped
        insert_session_directly(
            &db,
            "session-stopped-1",
            &persona_id,
            Some("sandbox-stopped-1"),
            SessionStatus::Running,
        );

        let manager = SessionManager::new(
            db.clone(),
            sbx,
            kit_generator,
            pty_bridge.clone(),
        );

        let results = manager.recover_sessions().await;
        assert_eq!(results.len(), 1);
        assert!(matches!(&results[0], RecoveryResult::Failed { session_id, .. } if session_id.0 == "session-stopped-1"));

        // Verify session was marked as stopped in DB
        let session = db
            .with_conn(|conn| db_ops::get_session(conn, &SessionId("session-stopped-1".to_string())))
            .unwrap();
        assert_eq!(session.status, SessionStatus::Stopped);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_recover_sessions_no_sandbox_id() {
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_success(sbx_dir.path());
        let (db, sbx, kit_generator, pty_bridge, _kit_dir, workspace_dir) =
            setup_test_env(sbx_path);

        let persona_id = insert_test_persona(&db, workspace_dir.path());

        // Insert a "starting" session with no sandbox_id
        insert_session_directly(
            &db,
            "session-no-sbx",
            &persona_id,
            None,
            SessionStatus::Starting,
        );

        let manager = SessionManager::new(
            db.clone(),
            sbx,
            kit_generator,
            pty_bridge.clone(),
        );

        let results = manager.recover_sessions().await;
        assert_eq!(results.len(), 1);
        assert!(matches!(&results[0], RecoveryResult::Failed { session_id, reason }
            if session_id.0 == "session-no-sbx" && reason.contains("No sandbox_id")));

        // Session with no sandbox_id should be cleaned up (deleted from DB)
        let session_result = db
            .with_conn(|conn| db_ops::get_session(conn, &SessionId("session-no-sbx".to_string())));
        assert!(session_result.is_err(), "Session with no sandbox_id should be deleted");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_recover_sessions_empty() {
        let sbx_dir = TempDir::new().unwrap();
        let sbx_path = create_mock_sbx_success(sbx_dir.path());
        let (db, sbx, kit_generator, pty_bridge, _kit_dir, _workspace_dir) =
            setup_test_env(sbx_path);

        let manager = SessionManager::new(db.clone(), sbx, kit_generator, pty_bridge);

        // No active sessions — should return empty
        let results = manager.recover_sessions().await;
        assert!(results.is_empty());
    }
}
