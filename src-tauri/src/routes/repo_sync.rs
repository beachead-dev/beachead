//! API routes for Repo Sync operations.
//!
//! Provides endpoints for managing git remote synchronization, including
//! repository CRUD, sync operations, credential management, and status polling.

use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::error::OrchestratorError;
use crate::server::AppState;
use crate::types::ManagedRepoId;

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

/// Request body for the push-to-remote endpoint.
#[derive(Debug, Deserialize)]
pub struct PushToRemoteRequest {
    pub commit_shas: Vec<String>,
    pub squash: bool,
    pub squash_message: Option<String>,
}

/// Per-repo operation lock to prevent concurrent sync operations.
///
/// If a repo ID is present in this map, a sync operation is in progress for it.
/// Handlers try to acquire before starting an operation and release on completion.
/// Returns HTTP 409 if the repo already has an operation in progress.
static OPERATION_LOCKS: std::sync::LazyLock<DashMap<String, ()>> =
    std::sync::LazyLock::new(DashMap::new);

/// Guard that removes the repo ID from the operation lock map on drop.
/// Ensures the lock is always released, even on early returns or panics.
struct OperationGuard {
    repo_id: String,
}

impl OperationGuard {
    /// Try to acquire the operation lock for a repo.
    /// Returns `Ok(Self)` if acquired, or `Err(OrchestratorError::SyncInProgress)` if busy.
    fn try_acquire(repo_id: &str) -> Result<Self, OrchestratorError> {
        match OPERATION_LOCKS.entry(repo_id.to_string()) {
            dashmap::mapref::entry::Entry::Occupied(_) => {
                Err(OrchestratorError::SyncInProgress(format!(
                    "A sync operation is already in progress for repo '{}'",
                    repo_id
                )))
            }
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(());
                Ok(Self {
                    repo_id: repo_id.to_string(),
                })
            }
        }
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        OPERATION_LOCKS.remove(&self.repo_id);
    }
}

/// Build the repo-sync routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/repo-sync/status", get(get_status))
        .route(
            "/api/repo-sync/settings/mirrors-dir",
            get(get_mirrors_dir).put(update_mirrors_dir),
        )
        .route(
            "/api/repo-sync/repos/{id}/pull-from-agent",
            post(pull_from_agent),
        )
        .route(
            "/api/repo-sync/repos/{id}/push-to-remote",
            post(push_to_remote),
        )
        .route(
            "/api/repo-sync/repos/{id}/fetch-from-remote",
            post(fetch_from_remote),
        )
        .route(
            "/api/repo-sync/repos/{id}/push-to-agent",
            post(push_to_agent),
        )
        .route("/api/repo-sync/repos/{id}/commits", get(list_commits))
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
