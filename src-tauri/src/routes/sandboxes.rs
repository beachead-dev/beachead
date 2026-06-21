use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;
use tokio::time::timeout;

use crate::db_ops;
use crate::error::OrchestratorError;
use crate::sbx::{PortMapping, SbxRunArgs};
use crate::server::AppState;
use crate::types::{PersonaId, PublishPortRequest};

/// Enriched sandbox info with a `managed` flag indicating whether the sandbox
/// is associated with a Beachead session (i.e., its ID exists in the sessions table).
#[derive(Debug, Clone, Serialize)]
pub struct SandboxInfoEnriched {
    pub name: Option<String>,
    pub id: Option<String>,
    pub agent: Option<String>,
    pub status: Option<String>,
    pub managed: bool,
}

/// Response for sandbox action endpoints (stop).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxActionResponse {
    pub id: String,
    pub status: String,
}

/// Response for sandbox start endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxStartResponse {
    pub id: String,
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
        .route("/api/sandboxes/{id}/stop", post(stop_sandbox))
        .route("/api/sandboxes/{id}/start", post(start_sandbox))
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
    let sbx = state
        .sbx
        .as_ref()
        .ok_or_else(|| OrchestratorError::SbxError("sbx CLI is not available".to_string()))?;

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
            // sbx ls --json uses `name` as the sandbox identifier.
            // The sessions table stores the sandbox name as `sandbox_id`.
            // Check both `id` and `name` fields against managed_ids.
            let managed =
                s.id.as_ref()
                    .map(|id| managed_ids.contains(id))
                    .unwrap_or(false)
                    || s.name
                        .as_ref()
                        .map(|name| managed_ids.contains(name))
                        .unwrap_or(false);
            SandboxInfoEnriched {
                name: s.name.clone(),
                id: s.id.or(s.name),
                agent: s
                    .extra
                    .get("agent")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
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

/// POST /api/sandboxes/{id}/stop — stop a running sandbox.
///
/// Calls `sbx stop {id}` with a 30-second timeout.
/// Returns HTTP 200 with `{ id, status }` on success.
/// If the sandbox is already stopped, returns 200 with current status (idempotent).
/// Error cases: 503 (sbx unavailable), 404 (not found), 504 (timeout).
async fn stop_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SandboxActionResponse>, OrchestratorError> {
    let sbx = state
        .sbx
        .as_ref()
        .ok_or_else(|| OrchestratorError::SbxUnavailable("sbx CLI is not available".to_string()))?;

    // First, check current status to handle idempotent case
    let sandboxes = timeout(SBX_COMMAND_TIMEOUT, sbx.ls_json()).await;
    let sandboxes = match sandboxes {
        Ok(Ok(list)) => list,
        Ok(Err(e)) => return Err(e),
        Err(_) => {
            return Err(OrchestratorError::SbxTimeout(
                "sbx ls timed out while checking sandbox status".to_string(),
            ))
        }
    };

    let sandbox = sandboxes
        .iter()
        .find(|s| s.id.as_deref() == Some(&id) || s.name.as_deref() == Some(&id))
        .ok_or_else(|| OrchestratorError::NotFound(format!("Sandbox '{}' not found", id)))?;

    // If already stopped, return current status without calling stop
    if sandbox.status.as_deref() == Some("stopped") {
        return Ok(Json(SandboxActionResponse {
            id: id.clone(),
            status: "stopped".to_string(),
        }));
    }

    // Execute sbx stop with timeout
    let result = timeout(SBX_COMMAND_TIMEOUT, sbx.stop(&id)).await;

    match result {
        Ok(Ok(())) => {
            // After stopping, query status to confirm
            let status = match timeout(SBX_COMMAND_TIMEOUT, sbx.ls_json()).await {
                Ok(Ok(list)) => list
                    .iter()
                    .find(|s| s.id.as_deref() == Some(&id) || s.name.as_deref() == Some(&id))
                    .and_then(|s| s.status.clone())
                    .unwrap_or_else(|| "stopped".to_string()),
                _ => "stopped".to_string(),
            };

            Ok(Json(SandboxActionResponse { id, status }))
        }
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
            "sbx stop '{}' timed out after 30 seconds",
            id
        ))),
    }
}

/// POST /api/sandboxes/{id}/start — start a new sandbox instance.
///
/// Looks up the session associated with the given sandbox ID to find the
/// persona's configured agent and workspace, then calls `sbx run` to create
/// a new running sandbox instance.
///
/// Returns HTTP 200 with `{ id }` containing the new sandbox ID.
/// Error cases: 503 (sbx unavailable), 404 (not found in sessions), 504 (timeout).
async fn start_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<SandboxStartResponse>, OrchestratorError> {
    let sbx = state
        .sbx
        .as_ref()
        .ok_or_else(|| OrchestratorError::SbxUnavailable("sbx CLI is not available".to_string()))?;

    // The frontend may pass either the stable UUID (from sbx ls --json `id` field)
    // or the sandbox name. The sessions table stores the sandbox name as `sandbox_id`.
    // Resolve the sandbox name from sbx ls if needed.
    let sandbox_name = {
        let sandboxes = timeout(SBX_COMMAND_TIMEOUT, sbx.ls_json())
            .await
            .map_err(|_| {
                OrchestratorError::SbxTimeout("sbx ls timed out".to_string())
            })??;
        sandboxes
            .iter()
            .find(|s| s.id.as_deref() == Some(&id) || s.name.as_deref() == Some(&id))
            .and_then(|s| s.name.clone())
    };

    // Try looking up the session by the provided id first, then by resolved name
    let persona_id: PersonaId = state.db.with_conn(|conn| {
        // Try direct match on sandbox_id (handles case where name was stored)
        let result = conn.query_row(
            "SELECT persona_id FROM sessions WHERE sandbox_id = ?1 ORDER BY created_at DESC LIMIT 1",
            [&id],
            |row| row.get::<_, String>(0),
        );
        if let Ok(pid) = result {
            return Ok(PersonaId(pid));
        }

        // Try matching by resolved sandbox name (handles UUID passed but name stored)
        if let Some(ref name) = sandbox_name {
            let result = conn.query_row(
                "SELECT persona_id FROM sessions WHERE sandbox_id = ?1 ORDER BY created_at DESC LIMIT 1",
                [name],
                |row| row.get::<_, String>(0),
            );
            if let Ok(pid) = result {
                return Ok(PersonaId(pid));
            }
        }

        Err(OrchestratorError::NotFound(format!(
            "Sandbox '{}' not found in sessions",
            id
        )))
    })?;

    // Get the persona to find agent_type_id and workspace_path
    let persona = state
        .db
        .with_conn(|conn| db_ops::get_persona(conn, &persona_id))?;

    // Get the agent type to find the sbx_agent identifier
    let agent_type = state
        .db
        .with_conn(|conn| db_ops::get_agent_type(conn, &persona.agent_type_id))?;
    let agent = agent_type.sbx_agent.or(agent_type.kit_ref).ok_or_else(|| {
        OrchestratorError::Internal(format!(
            "Agent type '{}' has no sbx_agent or kit_ref configured",
            agent_type.name
        ))
    })?;

    // Build run args and execute sbx run with timeout.
    // Use --name with the sandbox name to re-attach to the existing stopped sandbox.
    let run_args = SbxRunArgs {
        agent,
        kit_paths: vec![],
        workspace: persona.workspace_path,
        name: sandbox_name.clone(),
        template: None,
        agent_args: persona.agent_cli_args,
    };

    let result = timeout(SBX_COMMAND_TIMEOUT, sbx.run(&run_args)).await;

    match result {
        Ok(Ok(new_sandbox_id)) => Ok(Json(SandboxStartResponse { id: new_sandbox_id })),
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
            "sbx run for sandbox '{}' timed out after 30 seconds",
            id
        ))),
    }
}

/// DELETE /api/sandboxes/{id} — remove a sandbox permanently.
///
/// Calls `sbx rm {id}` with a 30-second timeout.
/// Returns HTTP 204 on success.
/// Error cases: 503 (sbx unavailable), 404 (not found), 504 (timeout).
async fn remove_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, OrchestratorError> {
    let sbx = state
        .sbx
        .as_ref()
        .ok_or_else(|| OrchestratorError::SbxUnavailable("sbx CLI is not available".to_string()))?;

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
    let sbx = state
        .sbx
        .as_ref()
        .ok_or_else(|| OrchestratorError::SbxError("sbx CLI is not available".to_string()))?;
    let ports = sbx.ports_list(&id).await?;
    Ok(Json(ports))
}

/// POST /api/sandboxes/{id}/ports — publish a port for a sandbox.
async fn publish_port(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<PublishPortRequest>,
) -> Result<(StatusCode, Json<PortMapping>), OrchestratorError> {
    let sbx = state
        .sbx
        .as_ref()
        .ok_or_else(|| OrchestratorError::SbxError("sbx CLI is not available".to_string()))?;
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
    let sbx = state
        .sbx
        .as_ref()
        .ok_or_else(|| OrchestratorError::SbxError("sbx CLI is not available".to_string()))?;
    sbx.ports_unpublish(&id, &req.port_spec).await?;
    Ok(StatusCode::NO_CONTENT)
}
