//! API routes for Repo Sync operations.
//!
//! Provides endpoints for managing git remote synchronization, including
//! repository CRUD, sync operations, credential management, and status polling.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use chrono::Utc;
use uuid::Uuid;

use crate::db_ops;
use crate::error::OrchestratorError;
use crate::repo_credential_manager;
use crate::server::AppState;
use crate::types::{
    CommitInfo, DetectedRepo, EnableRepoRequest, ManagedRepoId, ManagedRepoResponse,
    RepoCredential, SetCredentialsRequest, SyncStatus, UpdateRepoRequest,
};

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

/// Query parameters for the DELETE /api/repo-sync/repos/{id} endpoint.
#[derive(Debug, Deserialize)]
pub struct DeleteRepoQuery {
    #[serde(default)]
    pub delete_mirror: bool,
}

/// Per-repo operation lock to prevent concurrent sync operations.
///
/// If a repo ID is present in this map, a sync operation is in progress for it.
/// Handlers try to acquire before starting an operation and release on completion.
/// Returns HTTP 409 if the repo already has an operation in progress.
pub static OPERATION_LOCKS: std::sync::LazyLock<DashMap<String, ()>> =
    std::sync::LazyLock::new(DashMap::new);

/// Guard that removes the repo ID from the operation lock map on drop.
/// Ensures the lock is always released, even on early returns or panics.
pub struct OperationGuard {
    repo_id: String,
}

impl OperationGuard {
    /// Try to acquire the operation lock for a repo.
    /// Returns `Ok(Self)` if acquired, or `Err(OrchestratorError::SyncInProgress)` if busy.
    pub fn try_acquire(repo_id: &str) -> Result<Self, OrchestratorError> {
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
        .route("/api/repo-sync/repos", get(list_repos).post(enable_repo))
        .route(
            "/api/repo-sync/repos/{id}",
            put(update_repo).delete(delete_repo),
        )
        .route("/api/repo-sync/scan", post(scan_workspaces))
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
        .route(
            "/api/repo-sync/repos/{id}/credentials",
            put(set_credentials).delete(delete_credentials),
        )
}

// --- Status and Settings Endpoints ---

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

// --- CRUD Endpoints ---

/// GET /api/repo-sync/repos — list all managed repos with sync status.
///
/// Returns all managed repos enriched with sync status from the background checker,
/// persona name lookup, credential status, and mirror existence check.
///
/// # Requirements: 18.1, 18.14, 18.15
async fn list_repos(
    State(state): State<AppState>,
) -> Result<Json<Vec<ManagedRepoResponse>>, OrchestratorError> {
    let mgr = state.require_repo_sync_manager()?;

    // Get all managed repos from DB
    let repos = state
        .db
        .with_conn(|conn| db_ops::list_managed_repos(conn))?;

    // Get cached sync status from background checker
    let cached_status = mgr.get_cached_status();

    // Build enriched responses
    let mut responses = Vec::with_capacity(repos.len());
    for repo in repos {
        // Look up persona name
        let persona_name = state
            .db
            .with_conn(|conn| {
                db_ops::get_persona(conn, &repo.persona_id)
                    .map(|p| p.name)
            })
            .unwrap_or_else(|_| "Unknown".to_string());

        // Get sync status from cache (default to zeros if not yet computed)
        let sync_status = cached_status
            .get(&repo.id.0)
            .cloned()
            .unwrap_or(SyncStatus {
                workspace_ahead: 0,
                mirror_ahead: 0,
                remote_ahead: 0,
            });

        // Check credential status
        let credential_status =
            match repo_credential_manager::credentials_configured(&repo.id.0) {
                Ok(true) => "configured".to_string(),
                _ => "not_configured".to_string(),
            };

        // Check if mirror directory exists on disk
        let mirror_exists = PathBuf::from(&repo.mirror_path).exists();

        responses.push(ManagedRepoResponse {
            id: repo.id.0,
            persona_id: repo.persona_id.0,
            persona_name,
            workspace_path: repo.workspace_path,
            mirror_path: repo.mirror_path,
            remote_url: repo.remote_url,
            remote_provider: repo.remote_provider.map(|p| p.to_string()),
            branch_strategy: repo.branch_strategy.to_string(),
            branch_pattern: repo.branch_pattern,
            attribution_mode: repo.attribution_mode.to_string(),
            sync_mode: repo.sync_mode.to_string(),
            secret_scan_mode: repo.secret_scan_mode.to_string(),
            check_interval_seconds: repo.check_interval_seconds,
            sync_status,
            credential_status,
            mirror_exists,
            created_at: repo.created_at.to_rfc3339(),
            updated_at: repo.updated_at.to_rfc3339(),
        });
    }

    Ok(Json(responses))
}

/// POST /api/repo-sync/repos — enable repo sync for a workspace.
///
/// If `remote_url` is provided and the workspace has no remotes, calls
/// `enable_agent_created`. If the workspace has remotes, calls `enable`.
///
/// # Requirements: 18.2, 18.16
async fn enable_repo(
    State(state): State<AppState>,
    Json(req): Json<EnableRepoRequest>,
) -> Result<(StatusCode, Json<ManagedRepoResponse>), OrchestratorError> {
    let mgr = state.require_repo_sync_manager()?;

    let workspace_path = std::path::Path::new(&req.workspace_path);

    // Determine which enable path to use based on whether workspace has remotes
    let has_remotes = mgr
        .git
        .list_remote_names(workspace_path)
        .await
        .map(|names| !names.is_empty())
        .unwrap_or(false);

    let repo = if has_remotes {
        // Workspace has remotes — use the standard enable flow
        mgr.enable(&req.persona_id, workspace_path).await?
    } else {
        // No remotes — use the agent-created flow (link to remote or keep local)
        mgr.enable_agent_created(
            &req.persona_id,
            workspace_path,
            req.remote_url.as_deref(),
        )
        .await?
    };

    // Build the response
    let persona_name = state
        .db
        .with_conn(|conn| {
            db_ops::get_persona(conn, &repo.persona_id)
                .map(|p| p.name)
        })
        .unwrap_or_else(|_| "Unknown".to_string());

    let credential_status =
        match repo_credential_manager::credentials_configured(&repo.id.0) {
            Ok(true) => "configured".to_string(),
            _ => "not_configured".to_string(),
        };

    let mirror_exists = PathBuf::from(&repo.mirror_path).exists();

    let response = ManagedRepoResponse {
        id: repo.id.0,
        persona_id: repo.persona_id.0,
        persona_name,
        workspace_path: repo.workspace_path,
        mirror_path: repo.mirror_path,
        remote_url: repo.remote_url,
        remote_provider: repo.remote_provider.map(|p| p.to_string()),
        branch_strategy: repo.branch_strategy.to_string(),
        branch_pattern: repo.branch_pattern,
        attribution_mode: repo.attribution_mode.to_string(),
        sync_mode: repo.sync_mode.to_string(),
        secret_scan_mode: repo.secret_scan_mode.to_string(),
        check_interval_seconds: repo.check_interval_seconds,
        sync_status: SyncStatus {
            workspace_ahead: 0,
            mirror_ahead: 0,
            remote_ahead: 0,
        },
        credential_status,
        mirror_exists,
        created_at: repo.created_at.to_rfc3339(),
        updated_at: repo.updated_at.to_rfc3339(),
    };

    Ok((StatusCode::CREATED, Json(response)))
}

/// PUT /api/repo-sync/repos/{id} — update repo configuration.
///
/// Validates and applies configuration changes to a managed repo.
///
/// # Requirements: 18.4, 18.16
async fn update_repo(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateRepoRequest>,
) -> Result<Json<ManagedRepoResponse>, OrchestratorError> {
    let mgr = state.require_repo_sync_manager()?;
    let repo_id = ManagedRepoId(id);

    let repo = mgr.update_repo(&repo_id, &req).await?;

    // Build the response
    let persona_name = state
        .db
        .with_conn(|conn| {
            db_ops::get_persona(conn, &repo.persona_id)
                .map(|p| p.name)
        })
        .unwrap_or_else(|_| "Unknown".to_string());

    let cached_status = mgr.get_cached_status();
    let sync_status = cached_status
        .get(&repo.id.0)
        .cloned()
        .unwrap_or(SyncStatus {
            workspace_ahead: 0,
            mirror_ahead: 0,
            remote_ahead: 0,
        });

    let credential_status =
        match repo_credential_manager::credentials_configured(&repo.id.0) {
            Ok(true) => "configured".to_string(),
            _ => "not_configured".to_string(),
        };

    let mirror_exists = PathBuf::from(&repo.mirror_path).exists();

    let response = ManagedRepoResponse {
        id: repo.id.0,
        persona_id: repo.persona_id.0,
        persona_name,
        workspace_path: repo.workspace_path,
        mirror_path: repo.mirror_path,
        remote_url: repo.remote_url,
        remote_provider: repo.remote_provider.map(|p| p.to_string()),
        branch_strategy: repo.branch_strategy.to_string(),
        branch_pattern: repo.branch_pattern,
        attribution_mode: repo.attribution_mode.to_string(),
        sync_mode: repo.sync_mode.to_string(),
        secret_scan_mode: repo.secret_scan_mode.to_string(),
        check_interval_seconds: repo.check_interval_seconds,
        sync_status,
        credential_status,
        mirror_exists,
        created_at: repo.created_at.to_rfc3339(),
        updated_at: repo.updated_at.to_rfc3339(),
    };

    Ok(Json(response))
}

/// DELETE /api/repo-sync/repos/{id} — disable/remove repo sync.
///
/// Removes the managed repo record, deletes keyring credentials, and optionally
/// deletes the mirror directory from disk.
///
/// # Requirements: 18.10
async fn delete_repo(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<DeleteRepoQuery>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_repo_sync_manager()?;
    let repo_id = ManagedRepoId(id);

    mgr.delete_repo(&repo_id, params.delete_mirror).await?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/repo-sync/scan — scan all persona workspaces for untracked repos.
///
/// Detects git repositories in persona workspace directories that are not yet
/// tracked by Repo Sync.
///
/// # Requirements: 18.3
async fn scan_workspaces(
    State(state): State<AppState>,
) -> Result<Json<Vec<DetectedRepo>>, OrchestratorError> {
    let mgr = state.require_repo_sync_manager()?;
    let detected = mgr.scan_workspaces().await?;
    Ok(Json(detected))
}

// --- Sync Operation Endpoints ---

/// POST /api/repo-sync/repos/{id}/pull-from-agent — pull workspace commits into mirror.
///
/// Fetches new commits from the agent's workspace into the host-side mirror,
/// then merges them (fast-forward preferred, regular merge as fallback).
/// Returns the number of commits pulled.
///
/// Acquires a per-repo operation lock; returns HTTP 409 if another sync operation
/// is already in progress for this repo.
///
/// # Requirements: 18.5, 18.13, 18.17
async fn pull_from_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, OrchestratorError> {
    let _guard = OperationGuard::try_acquire(&id)?;
    let mgr = state.require_repo_sync_manager()?;
    let repo_id = ManagedRepoId(id);
    let result = mgr.pull_from_agent(&repo_id).await?;
    Ok(Json(serde_json::json!({ "commits": result.commits })))
}

/// POST /api/repo-sync/repos/{id}/push-to-remote — push mirror commits to remote.
///
/// Pushes selected commits from the mirror to the remote repository. Supports
/// squashing selected commits into a single commit. Runs secret scanning before
/// push. Requires credentials to be configured.
///
/// Acquires a per-repo operation lock; returns HTTP 409 if another sync operation
/// is already in progress for this repo.
///
/// # Requirements: 18.6, 18.13, 18.17, 18.18
async fn push_to_remote(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PushToRemoteRequest>,
) -> Result<Json<serde_json::Value>, OrchestratorError> {
    let _guard = OperationGuard::try_acquire(&id)?;
    let mgr = state.require_repo_sync_manager()?;
    let repo_id = ManagedRepoId(id);
    let result = mgr
        .push_to_remote(&repo_id, &req.commit_shas, req.squash, req.squash_message.as_deref())
        .await?;
    Ok(Json(serde_json::json!({
        "branch": result.branch,
        "commits": result.commits
    })))
}

/// POST /api/repo-sync/repos/{id}/fetch-from-remote — fetch remote commits into mirror.
///
/// Fetches new commits from the remote repository into the mirror using
/// configured credentials. Returns the number of new commits available.
///
/// Acquires a per-repo operation lock; returns HTTP 409 if another sync operation
/// is already in progress for this repo.
///
/// # Requirements: 18.7, 18.13, 18.17
async fn fetch_from_remote(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, OrchestratorError> {
    let _guard = OperationGuard::try_acquire(&id)?;
    let mgr = state.require_repo_sync_manager()?;
    let repo_id = ManagedRepoId(id);
    let result = mgr.fetch_from_remote(&repo_id).await?;
    Ok(Json(serde_json::json!({ "commits": result.commits })))
}

/// POST /api/repo-sync/repos/{id}/push-to-agent — push mirror commits to workspace.
///
/// Pushes commits from the mirror into the agent's workspace using local file
/// paths only (no network access). Checks for dirty working tree first.
///
/// Acquires a per-repo operation lock; returns HTTP 409 if another sync operation
/// is already in progress for this repo.
///
/// # Requirements: 18.8, 18.13, 18.17
async fn push_to_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, OrchestratorError> {
    let _guard = OperationGuard::try_acquire(&id)?;
    let mgr = state.require_repo_sync_manager()?;
    let repo_id = ManagedRepoId(id);
    let result = mgr.push_to_agent(&repo_id).await?;
    Ok(Json(serde_json::json!({ "commits": result.commits })))
}

/// GET /api/repo-sync/repos/{id}/commits — list unpushed commits in mirror.
///
/// Returns the list of commits in the mirror that have not yet been pushed
/// to the remote, including message, author, timestamp, and file change stats.
///
/// # Requirements: 18.9
async fn list_commits(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<CommitInfo>>, OrchestratorError> {
    let mgr = state.require_repo_sync_manager()?;
    let repo_id = ManagedRepoId(id);
    let commits = mgr.list_commits(&repo_id).await?;
    Ok(Json(commits))
}


// --- Credential Endpoints ---

/// PUT /api/repo-sync/repos/{id}/credentials — store credentials in keyring.
///
/// Accepts a `SetCredentialsRequest` JSON body with username, secret, and credential_type.
/// Validates that username and secret are non-empty. Stores credentials in the OS keyring
/// via `repo_credential_manager::store_credentials()`, then upserts a `RepoCredential`
/// record in the database (deletes existing record first if present, then inserts new).
///
/// Returns 200 OK with `{ "status": "configured" }`.
///
/// # Requirements: 18.11, 13.5, 13.6, 13.9
async fn set_credentials(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<SetCredentialsRequest>,
) -> Result<Json<serde_json::Value>, OrchestratorError> {
    // Validate non-empty username and secret
    if req.username.trim().is_empty() {
        return Err(OrchestratorError::Validation(
            "username must not be empty".to_string(),
        ));
    }
    if req.secret.trim().is_empty() {
        return Err(OrchestratorError::Validation(
            "secret must not be empty".to_string(),
        ));
    }

    // Verify the repo exists
    let repo_id = ManagedRepoId(id.clone());
    state
        .db
        .with_conn(|conn| {
            db_ops::get_managed_repo(conn, &repo_id)
        })?;

    // Store credentials in the OS keyring
    repo_credential_manager::store_credentials(&id, req.username, req.secret)?;

    // Upsert the RepoCredential record in the DB:
    // Delete existing record (if any), then insert new one.
    let keyring_service = format!("beachead-repo-sync-{}", id);
    let now = Utc::now();
    let cred = RepoCredential {
        id: Uuid::new_v4().to_string(),
        repo_id: ManagedRepoId(id.clone()),
        keyring_service_name: keyring_service,
        credential_type: req.credential_type,
        created_at: now,
        updated_at: now,
    };

    state.db.with_conn(|conn| {
        // Delete old credential record if it exists
        let _ = db_ops::delete_repo_credential(conn, &ManagedRepoId(id.clone()));
        // Insert new credential record
        db_ops::insert_repo_credential(conn, &cred)
    })?;

    Ok(Json(serde_json::json!({ "status": "configured" })))
}

/// DELETE /api/repo-sync/repos/{id}/credentials — remove credentials from keyring.
///
/// Deletes credentials from the OS keyring via `repo_credential_manager::delete_credentials()`
/// and removes the `RepoCredential` record from the database.
///
/// Returns 204 No Content.
///
/// # Requirements: 18.12, 13.5, 13.6, 13.9
async fn delete_credentials(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, OrchestratorError> {
    let repo_id = ManagedRepoId(id.clone());

    // Delete from OS keyring (idempotent — ignores "not found")
    repo_credential_manager::delete_credentials(&id)?;

    // Delete from database
    state.db.with_conn(|conn| {
        db_ops::delete_repo_credential(conn, &repo_id)
    })?;

    Ok(StatusCode::NO_CONTENT)
}
