use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::timeout;

use crate::error::OrchestratorError;
use crate::sbx::PortMapping;
use crate::server::AppState;
use crate::types::PublishPortRequest;

/// Enriched sandbox info with a `managed` flag indicating whether the sandbox
/// is associated with a Beachead session (i.e., its ID exists in the sessions table).
#[derive(Debug, Clone, Serialize)]
pub struct SandboxInfoEnriched {
    pub name: Option<String>,
    pub id: Option<String>,
    pub status: Option<String>,
    pub managed: bool,
}

/// Query parameters for `GET /api/sandboxes`.
#[derive(Debug, Deserialize)]
struct ListSandboxesQuery {
    /// When true, return all sandboxes. When false (default), only return managed sandboxes.
    #[serde(default)]
    show_all: bool,
}

/// Build the sandbox routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/sandboxes", get(list_sandboxes))
        .route("/api/sandboxes/{id}", delete(remove_sandbox))
        .route(
            "/api/sandboxes/{id}/ports",
            get(list_ports).post(publish_port).delete(unpublish_port),
        )
}

/// GET /api/sandboxes — list sandboxes with managed filtering.
///
/// By default (`show_all=false`), only returns sandboxes whose ID matches a
/// `sandbox_id` in the sessions table. When `show_all=true`, returns all
/// sandboxes with the `managed` flag set appropriately.
async fn list_sandboxes(
    State(state): State<AppState>,
    Query(query): Query<ListSandboxesQuery>,
) -> Result<Json<Vec<SandboxInfoEnriched>>, OrchestratorError> {
    let sbx = state.sbx.as_ref().ok_or_else(|| {
        OrchestratorError::SbxError("sbx CLI is not available".to_string())
    })?;

    let sandboxes = sbx.ls_json().await?;

    // Query sessions table for distinct sandbox_id values to determine managed sandboxes
    let managed_ids: HashSet<String> = state.db.with_conn(|conn| {
        let mut stmt = conn
            .prepare("SELECT DISTINCT sandbox_id FROM sessions WHERE sandbox_id IS NOT NULL")
            .map_err(|e| OrchestratorError::Database(format!("Failed to query sessions: {}", e)))?;
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| OrchestratorError::Database(format!("Failed to query sessions: {}", e)))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(ids)
    })?;

    // Enrich sandboxes with managed flag
    let enriched: Vec<SandboxInfoEnriched> = sandboxes
        .into_iter()
        .map(|s| {
            let managed = s
                .id
                .as_ref()
                .map(|id| managed_ids.contains(id))
                .unwrap_or(false);
            SandboxInfoEnriched {
                name: s.name,
                id: s.id,
                status: s.status,
                managed,
            }
        })
        .filter(|s| query.show_all || s.managed)
        .collect();

    Ok(Json(enriched))
}

/// Timeout duration for sbx CLI commands (30 seconds per requirements).
const SBX_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// DELETE /api/sandboxes/{id} — remove a sandbox permanently.
///
/// Calls `sbx rm {id}` with a 30-second timeout.
/// Returns HTTP 204 on success.
/// Error cases: 503 (sbx unavailable), 404 (not found), 504 (timeout).
async fn remove_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, OrchestratorError> {
    let sbx = state.sbx.as_ref().ok_or_else(|| {
        OrchestratorError::SbxUnavailable("sbx CLI is not available".to_string())
    })?;

    let result = timeout(SBX_COMMAND_TIMEOUT, sbx.rm(&id)).await;

    match result {
        Ok(Ok(())) => Ok(StatusCode::NO_CONTENT),
        Ok(Err(e)) => {
            let err_msg = e.to_string();
            if err_msg.contains("not found") || err_msg.contains("No such") {
                Err(OrchestratorError::NotFound(format!(
                    "Sandbox '{}' not found",
                    id
                )))
            } else {
                Err(e)
            }
        }
        Err(_) => Err(OrchestratorError::SbxTimeout(format!(
            "sbx rm '{}' timed out after 30 seconds",
            id
        ))),
    }
}

/// GET /api/sandboxes/{id}/ports — list published ports for a sandbox.
async fn list_ports(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<PortMapping>>, OrchestratorError> {
    let sbx = state.sbx.as_ref().ok_or_else(|| {
        OrchestratorError::SbxError("sbx CLI is not available".to_string())
    })?;
    let ports = sbx.ports_list(&id).await?;
    Ok(Json(ports))
}

/// POST /api/sandboxes/{id}/ports — publish a port for a sandbox.
async fn publish_port(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PublishPortRequest>,
) -> Result<(StatusCode, Json<PortMapping>), OrchestratorError> {
    let sbx = state.sbx.as_ref().ok_or_else(|| {
        OrchestratorError::SbxError("sbx CLI is not available".to_string())
    })?;
    let mapping = sbx.ports_publish(&id, &req.port_spec).await?;
    Ok((StatusCode::CREATED, Json(mapping)))
}

/// Request body for unpublishing a port.
#[derive(Debug, Deserialize)]
struct UnpublishPortRequest {
    port_spec: String,
}

/// DELETE /api/sandboxes/{id}/ports — unpublish a port for a sandbox.
async fn unpublish_port(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UnpublishPortRequest>,
) -> Result<StatusCode, OrchestratorError> {
    let sbx = state.sbx.as_ref().ok_or_else(|| {
        OrchestratorError::SbxError("sbx CLI is not available".to_string())
    })?;
    sbx.ports_unpublish(&id, &req.port_spec).await?;
    Ok(StatusCode::NO_CONTENT)
}
