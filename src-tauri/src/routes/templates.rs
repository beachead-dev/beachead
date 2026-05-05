use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::Deserialize;

use crate::error::OrchestratorError;
use crate::sbx::TemplateInfo;
use crate::server::AppState;

/// Build the template routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/templates", get(list_templates))
        .route("/api/templates/{sandbox_id}", axum::routing::post(save_template))
        .route("/api/templates/load", axum::routing::post(load_template))
        .route("/api/templates/{tag}", axum::routing::delete(remove_template))
}

/// GET /api/templates — list all saved templates.
async fn list_templates(
    State(state): State<AppState>,
) -> Result<Json<Vec<TemplateInfo>>, OrchestratorError> {
    let mgr = state.require_template_manager()?;
    let templates = mgr.list().await?;
    Ok(Json(templates))
}

/// Request body for saving a template.
#[derive(Debug, Deserialize)]
struct SaveTemplateRequest {
    tag: String,
}

/// POST /api/templates/{sandbox_id} — save a sandbox as a template.
async fn save_template(
    State(state): State<AppState>,
    Path(sandbox_id): Path<String>,
    Json(req): Json<SaveTemplateRequest>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_template_manager()?;
    mgr.save(&sandbox_id, &req.tag, None).await?;
    Ok(StatusCode::CREATED)
}

/// Request body for loading a template from a tar file.
#[derive(Debug, Deserialize)]
struct LoadTemplateRequest {
    tar_path: String,
}

/// POST /api/templates/load — load a template from a tar file.
async fn load_template(
    State(state): State<AppState>,
    Json(req): Json<LoadTemplateRequest>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_template_manager()?;
    let path = std::path::Path::new(&req.tar_path);
    mgr.load(path).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/templates/{tag} — remove a template by tag.
async fn remove_template(
    State(state): State<AppState>,
    Path(tag): Path<String>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_template_manager()?;
    mgr.remove(&tag).await?;
    Ok(StatusCode::NO_CONTENT)
}
