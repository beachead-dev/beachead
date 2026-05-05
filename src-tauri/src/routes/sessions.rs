use axum::{
    extract::{ws::WebSocketUpgrade, Multipart, Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};

use crate::error::OrchestratorError;
use crate::server::AppState;
use crate::session_manager::SessionManager;
use crate::types::{
    CreateSessionRequest, CreateSessionResponse, Session, SessionId, SessionStatus,
    UploadResult,
};

/// Build the session routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/sessions", post(create_session).get(list_sessions))
        .route("/api/sessions/{id}", get(get_session).delete(remove_session))
        .route("/api/sessions/{id}/stop", post(stop_session))
        .route("/api/sessions/{id}/resume", post(resume_session))
        .route("/api/sessions/{id}/upload", post(upload_file))
        .route("/api/sessions/{id}/terminal", get(ws_terminal))
}

/// POST /api/sessions — start a new session for a persona.
async fn create_session(
    State(state): State<AppState>,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<CreateSessionResponse>), OrchestratorError> {
    let mgr = state.require_session_manager()?;
    let session = mgr.start(&req.persona_id).await?;
    let ws_url = SessionManager::ws_url(&session.id);
    let response = CreateSessionResponse {
        session_id: session.id,
        ws_url,
    };
    Ok((StatusCode::CREATED, Json(response)))
}

/// GET /api/sessions — list all sessions.
async fn list_sessions(
    State(state): State<AppState>,
) -> Result<Json<Vec<Session>>, OrchestratorError> {
    let mgr = state.require_session_manager()?;
    let sessions = mgr.list()?;
    Ok(Json(sessions))
}

/// GET /api/sessions/{id} — get a session by ID.
async fn get_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Session>, OrchestratorError> {
    let mgr = state.require_session_manager()?;
    let sessions = mgr.list()?;
    let session = sessions
        .into_iter()
        .find(|s| s.id.0 == id)
        .ok_or_else(|| OrchestratorError::NotFound(format!("Session '{}' not found", id)))?;
    Ok(Json(session))
}

/// POST /api/sessions/{id}/stop — stop a running session.
async fn stop_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_session_manager()?;
    mgr.stop(&SessionId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/sessions/{id}/resume — resume a stopped session.
async fn resume_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_session_manager()?;
    mgr.resume(&SessionId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/sessions/{id} — remove a session completely.
async fn remove_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_session_manager()?;
    mgr.remove(&SessionId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/sessions/{id}/upload — upload a file to a session's sandbox.
async fn upload_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> Result<Json<UploadResult>, OrchestratorError> {
    let mgr = state.require_session_manager()?;
    let session_id = SessionId(id);

    // Extract the first file field from the multipart form
    let field = multipart
        .next_field()
        .await
        .map_err(|e| OrchestratorError::Validation(format!("Invalid multipart data: {}", e)))?
        .ok_or_else(|| OrchestratorError::Validation("No file field in upload".to_string()))?;

    let filename = field.file_name().unwrap_or("upload").to_string();

    let content = field
        .bytes()
        .await
        .map_err(|e| OrchestratorError::Validation(format!("Failed to read file data: {}", e)))?;

    let result = mgr
        .upload_file(&session_id, &filename, &content, None)
        .await?;

    Ok(Json(result))
}

/// GET /api/sessions/{id}/terminal — WebSocket upgrade for terminal access.
///
/// SECURITY: Validates that the session exists and is running before upgrading.
async fn ws_terminal(
    State(state): State<AppState>,
    Path(id): Path<String>,
    ws: WebSocketUpgrade,
) -> Result<impl IntoResponse, OrchestratorError> {
    let mgr = state.require_session_manager()?;
    let session_id = SessionId(id);

    // Validate session exists and is running before WebSocket upgrade
    let sessions = mgr.list()?;
    let session = sessions
        .into_iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| {
            OrchestratorError::NotFound(format!("Session '{}' not found", session_id))
        })?;

    if session.status != SessionStatus::Running {
        return Err(OrchestratorError::Validation(format!(
            "Session '{}' is not running (status: {})",
            session_id, session.status
        )));
    }

    let pty_bridge = state.pty_bridge.clone();
    Ok(ws.on_upgrade(move |socket| async move {
        if let Err(e) = pty_bridge.attach_ws(&session_id, socket).await {
            eprintln!("WebSocket attach error for session {}: {}", session_id, e);
        }
    }))
}
