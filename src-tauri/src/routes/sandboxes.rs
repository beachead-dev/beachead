use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::Deserialize;

use crate::error::OrchestratorError;
use crate::sbx::{PortMapping, SandboxInfo};
use crate::server::AppState;
use crate::types::PublishPortRequest;

/// Build the sandbox routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/sandboxes", get(list_sandboxes))
        .route(
            "/api/sandboxes/{id}/ports",
            get(list_ports).post(publish_port).delete(unpublish_port),
        )
}

/// GET /api/sandboxes — list all sandboxes via sbx ls.
async fn list_sandboxes(
    State(state): State<AppState>,
) -> Result<Json<Vec<SandboxInfo>>, OrchestratorError> {
    let sbx = state.sbx.as_ref().ok_or_else(|| {
        OrchestratorError::SbxError("sbx CLI is not available".to_string())
    })?;
    let sandboxes = sbx.ls_json().await?;
    Ok(Json(sandboxes))
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
