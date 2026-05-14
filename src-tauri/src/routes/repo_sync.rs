//! API routes for Repo Sync operations.
//!
//! Provides endpoints for managing git remote synchronization, including
//! repository CRUD, sync operations, credential management, and status polling.

use axum::{
    extract::State,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

use crate::error::OrchestratorError;
use crate::server::AppState;

/// Response for the lightweight status endpoint used by the sidebar badge.
#[derive(Debug, Serialize)]
pub struct RepoSyncStatusResponse {
    pub has_pending: bool,
}

/// Response for the mirrors directory endpoint.
#[derive(Debug, Serialize)]
pub struct MirrorsDirResponse {
    pub path: String,
}

/// Request body for updating the mirrors directory.
#[derive(Debug, Deserialize)]
pub struct UpdateMirrorsDirRequest {
    pub path: String,
}

/// Build the repo-sync routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/repo-sync/status", get(get_status))
        .route(
            "/api/repo-sync/settings/mirrors-dir",
            get(get_mirrors_dir).put(update_mirrors_dir),
        )
}

/// GET /api/repo-sync/status — lightweight endpoint for sidebar badge.
///
/// Returns whether any managed repo has pending commits in either direction
/// (workspace→mirror or remote→mirror). The sidebar polls this every 60 seconds
/// to show/hide the notification dot.
///
/// # Requirements: 16.2, 16.3, 16.7
async fn get_status(
    State(state): State<AppState>,
) -> Result<Json<RepoSyncStatusResponse>, OrchestratorError> {
    let mgr = state.require_repo_sync_manager()?;
    let has_pending = mgr.has_pending();
    Ok(Json(RepoSyncStatusResponse { has_pending }))
}

/// GET /api/repo-sync/settings/mirrors-dir — get current mirrors directory.
///
/// Returns the current configured mirrors directory path.
///
/// # Requirements: 14.1
async fn get_mirrors_dir(
    State(state): State<AppState>,
) -> Result<Json<MirrorsDirResponse>, OrchestratorError> {
    let mgr = state.require_repo_sync_manager()?;
    let path = mgr.get_mirrors_dir();
    Ok(Json(MirrorsDirResponse {
        path: path.to_string_lossy().to_string(),
    }))
}

/// PUT /api/repo-sync/settings/mirrors-dir — update mirrors directory.
///
/// Validates the new path (must be absolute, ≤4096 chars, writable or creatable),
/// creates the directory if needed, and updates all affected repo records.
///
/// # Requirements: 14.3, 14.4
async fn update_mirrors_dir(
    State(state): State<AppState>,
    Json(req): Json<UpdateMirrorsDirRequest>,
) -> Result<Json<MirrorsDirResponse>, OrchestratorError> {
    let mgr = state.require_repo_sync_manager()?;
    let updated_path = mgr.update_mirrors_dir(&req.path)?;
    Ok(Json(MirrorsDirResponse {
        path: updated_path.to_string_lossy().to_string(),
    }))
}
