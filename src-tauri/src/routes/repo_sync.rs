//! API routes for Repo Sync operations.
//!
//! Provides endpoints for managing git remote synchronization, including
//! repository CRUD, sync operations, credential management, and status polling.

use axum::{
    extract::State,
    routing::get,
    Json, Router,
};
use serde::Serialize;

use crate::error::OrchestratorError;
use crate::server::AppState;

/// Response for the lightweight status endpoint used by the sidebar badge.
#[derive(Debug, Serialize)]
pub struct RepoSyncStatusResponse {
    pub has_pending: bool,
}

/// Build the repo-sync routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/repo-sync/status", get(get_status))
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
