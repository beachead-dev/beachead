use std::io::{Read, Write};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use tokio::sync::{broadcast, Mutex};

use crate::error::OrchestratorError;
use crate::types::SessionId;

/// Resize control message sent from frontend via WebSocket.
#[derive(serde::Deserialize)]
struct ResizeMessage {
    rows: u16,
    cols: u16,
}

/// Status of a PTY session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtySessionStatus {
    Running,
    Stopped,
}

/// Holds the PTY process state for a single session.
///
/// The reader task runs for the lifetime of the PTY process (started in `spawn()`).
/// It sends output to a broadcast channel that multiple WebSocket connections can
/// subscribe to. This allows detach/reattach without losing the reader.
pub struct PtySession {
    pub child: Mutex<Box<dyn Child + Send + Sync>>,
    pub writer: Mutex<Box<dyn Write + Send>>,
    pub master: Mutex<Box<dyn MasterPty + Send>>,
    pub status: Mutex<PtySessionStatus>,
    /// Broadcast sender for PTY output. WebSocket connections subscribe to this.
    pub output_tx: broadcast::Sender<Vec<u8>>,
}

/// Manages PTY sessions for terminal I/O relay.
/// Uses DashMap for concurrent access to sessions from multiple tasks.
pub struct PtyBridge {
    sessions: DashMap<SessionId, Arc<PtySession>>,
}

impl PtyBridge {
    /// Create a new PtyBridge with no active sessions.
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }

    /// Spawn a new PTY process for the given session.
    ///
    /// Starts the PTY reader task immediately. Output is buffered in a broadcast
    /// channel (capacity 1024) until a WebSocket subscribes via `attach_ws()`.
    pub fn spawn(
        &self,
        session_id: SessionId,
        command: &str,
        args: &[&str],
    ) -> Result<(), OrchestratorError> {
        if self.sessions.contains_key(&session_id) {
            return Err(OrchestratorError::PtyError(format!(
                "Session {} already has an active PTY",
                session_id
            )));
        }

        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| OrchestratorError::PtyError(format!("Failed to open PTY: {}", e)))?;

        let mut cmd = CommandBuilder::new(command);
        cmd.args(args);

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| OrchestratorError::PtyError(format!("Failed to spawn command: {}", e)))?;

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| OrchestratorError::PtyError(format!("Failed to clone reader: {}", e)))?;

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| OrchestratorError::PtyError(format!("Failed to take writer: {}", e)))?;

        // Broadcast channel for PTY output (capacity 1024 messages)
        let (output_tx, _) = broadcast::channel::<Vec<u8>>(1024);

        let session = Arc::new(PtySession {
            child: Mutex::new(child),
            writer: Mutex::new(writer),
            master: Mutex::new(pair.master),
            status: Mutex::new(PtySessionStatus::Running),
            output_tx: output_tx.clone(),
        });

        // Start the PTY reader task — runs for the lifetime of the PTY process.
        // Reads from PTY stdout and broadcasts to all subscribers.
        let reader_session = session.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            let mut reader = reader;
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF — process exited
                    Ok(n) => {
                        let data = buf[..n].to_vec();
                        // Send to broadcast — if no subscribers, data is dropped (ok)
                        let _ = reader_session.output_tx.send(data);
                    }
                    Err(_) => break,
                }
            }
            // Mark session as stopped when PTY process exits
            let mut status = reader_session.status.blocking_lock();
            *status = PtySessionStatus::Stopped;
        });

        self.sessions.insert(session_id, session);
        Ok(())
    }

    /// Write data to the PTY stdin for the given session.
    pub fn write(&self, session_id: &SessionId, data: &[u8]) -> Result<(), OrchestratorError> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| {
                OrchestratorError::PtyError(format!("No PTY session for {}", session_id))
            })?;

        let session = session.clone();
        let mut writer = session.writer.blocking_lock();
        writer
            .write_all(data)
            .map_err(|e| OrchestratorError::PtyError(format!("Failed to write to PTY: {}", e)))?;
        Ok(())
    }

    /// Resize the PTY for the given session.
    pub fn resize(&self, session_id: &SessionId, rows: u16, cols: u16) -> Result<(), OrchestratorError> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| {
                OrchestratorError::PtyError(format!("No PTY session for {}", session_id))
            })?;

        let session = session.clone();
        let master = session.master.blocking_lock();
        master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| OrchestratorError::PtyError(format!("Failed to resize PTY: {}", e)))?;
        Ok(())
    }

    /// Attach a WebSocket to the PTY session for bidirectional I/O relay.
    ///
    /// Subscribes to the PTY output broadcast channel and relays to the WebSocket.
    /// Multiple WebSocket connections can attach to the same session simultaneously.
    /// When the WebSocket disconnects, the subscription is dropped but the PTY
    /// reader keeps running — allowing reattach without data loss.
    pub async fn attach_ws(
        &self,
        session_id: &SessionId,
        ws: WebSocket,
    ) -> Result<(), OrchestratorError> {
        let session = self
            .sessions
            .get(session_id)
            .ok_or_else(|| {
                OrchestratorError::PtyError(format!("No PTY session for {}", session_id))
            })?
            .clone();

        let (mut ws_sender, mut ws_receiver) = ws.split();

        // Subscribe to the PTY output broadcast
        let mut output_rx = session.output_tx.subscribe();

        // Task 1: PTY output (broadcast) -> WebSocket
        let ws_forward_handle = tokio::spawn(async move {
            loop {
                match output_rx.recv().await {
                    Ok(data) => {
                        let text = String::from_utf8_lossy(&data).into_owned();
                        if ws_sender.send(Message::Text(text.into())).await.is_err() {
                            break; // WebSocket closed
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        // Output was produced faster than we could relay — some was lost
                        eprintln!("PTY output lagged, skipped {} messages", skipped);
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        // PTY reader exited (process ended)
                        break;
                    }
                }
            }
            let _ = ws_sender.close().await;
        });

        // Task 2: WebSocket -> PTY stdin + resize
        let writer_session = session.clone();
        let resize_session = session.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = ws_receiver.next().await {
                match msg {
                    Message::Binary(data) => {
                        let ws = writer_session.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            let mut writer = ws.writer.blocking_lock();
                            let _ = writer.write_all(&data);
                        })
                        .await;
                    }
                    Message::Text(text) => {
                        if text.starts_with('\x01') {
                            let json_str = &text[1..];
                            if let Ok(resize) = serde_json::from_str::<ResizeMessage>(json_str) {
                                let rs = resize_session.clone();
                                let _ = tokio::task::spawn_blocking(move || {
                                    let master = rs.master.blocking_lock();
                                    let _ = master.resize(PtySize {
                                        rows: resize.rows,
                                        cols: resize.cols,
                                        pixel_width: 0,
                                        pixel_height: 0,
                                    });
                                })
                                .await;
                            }
                        } else {
                            let ws = writer_session.clone();
                            let _ = tokio::task::spawn_blocking(move || {
                                let mut writer = ws.writer.blocking_lock();
                                let _ = writer.write_all(text.as_bytes());
                            })
                            .await;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            // WebSocket closed — abort the forward task
            ws_forward_handle.abort();
        });

        Ok(())
    }

    /// Kill the PTY process for the given session and clean up.
    pub fn kill(&self, session_id: &SessionId) -> Result<(), OrchestratorError> {
        let (_, session) = self.sessions.remove(session_id).ok_or_else(|| {
            OrchestratorError::PtyError(format!("No PTY session for {}", session_id))
        })?;

        // Kill the child process
        let mut child = session.child.blocking_lock();
        child
            .kill()
            .map_err(|e| OrchestratorError::PtyError(format!("Failed to kill PTY process: {}", e)))?;

        // Mark as stopped
        let mut status = session.status.blocking_lock();
        *status = PtySessionStatus::Stopped;

        Ok(())
    }

    /// Check if a session exists and return its status.
    pub fn session_status(
        &self,
        session_id: &SessionId,
    ) -> Option<PtySessionStatus> {
        self.sessions.get(session_id).map(|s| {
            s.status.blocking_lock().clone()
        })
    }

    /// Get the number of active sessions.
    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_spawn_and_kill_lifecycle() {
        let bridge = PtyBridge::new();
        let session_id = SessionId("test-session-1".to_string());

        bridge
            .spawn(session_id.clone(), "echo", &["hello"])
            .expect("spawn should succeed");

        assert_eq!(bridge.session_count(), 1);
        std::thread::sleep(std::time::Duration::from_millis(200));

        bridge.kill(&session_id).expect("kill should succeed");
        assert_eq!(bridge.session_count(), 0);
    }

    #[test]
    fn test_spawn_duplicate_session_fails() {
        let bridge = PtyBridge::new();
        let session_id = SessionId("test-session-dup".to_string());

        bridge
            .spawn(session_id.clone(), "sleep", &["10"])
            .expect("first spawn should succeed");

        let result = bridge.spawn(session_id.clone(), "sleep", &["10"]);
        assert!(result.is_err());

        bridge.kill(&session_id).expect("kill should succeed");
    }

    #[test]
    fn test_kill_nonexistent_session_fails() {
        let bridge = PtyBridge::new();
        let session_id = SessionId("nonexistent".to_string());

        let result = bridge.kill(&session_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_write_to_session() {
        let bridge = PtyBridge::new();
        let session_id = SessionId("test-write".to_string());

        #[cfg(unix)]
        let (cmd, args): (&str, &[&str]) = ("cat", &[]);
        #[cfg(windows)]
        let (cmd, args): (&str, &[&str]) = ("cmd", &["/c", "more"]);

        bridge
            .spawn(session_id.clone(), cmd, args)
            .expect("spawn should succeed");

        let result = bridge.write(&session_id, b"hello\n");
        assert!(result.is_ok());

        bridge.kill(&session_id).expect("kill should succeed");
    }

    #[test]
    fn test_write_to_nonexistent_session_fails() {
        let bridge = PtyBridge::new();
        let session_id = SessionId("no-such-session".to_string());

        let result = bridge.write(&session_id, b"data");
        assert!(result.is_err());
    }

    #[test]
    fn test_session_status() {
        let bridge = PtyBridge::new();
        let session_id = SessionId("test-status".to_string());

        assert!(bridge.session_status(&session_id).is_none());

        bridge
            .spawn(session_id.clone(), "sleep", &["10"])
            .expect("spawn should succeed");

        assert_eq!(
            bridge.session_status(&session_id),
            Some(PtySessionStatus::Running)
        );

        bridge.kill(&session_id).expect("kill should succeed");
        assert!(bridge.session_status(&session_id).is_none());
    }
}
