//! API routes for Repo Sync operations.
//!
//! Provides endpoints for managing git remote synchronization, including
//! repository CRUD, sync operations, credential management, and status polling.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path as StdPath;

use crate::db_ops;
use crate::error::OrchestratorError;
use crate::server::AppState;
use crate::types::{
    DetectedRepo, EnableRepoRequest, ManagedRepoId, ManagedRepoResponse, SyncStatus,
    UpdateRepoRequest,
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

/// Query parameters for `DELETE /api/repo-sync/repos/{id}`.
#[derive(Debug, Deserialize)]
struct DeleteRepoQuery {
    /// When true, also delete the mirror directory on disk.
    /// Defaults to false.
    #[serde(default)]
    delete_mirror: bool,
}
