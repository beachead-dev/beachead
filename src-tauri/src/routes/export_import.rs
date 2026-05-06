use axum::{
    extract::State,
    http::{header, StatusCode},
    response::IntoResponse,
    routing::post,
    Json, Router,
};
use base64::Engine;
use serde::Deserialize;

use crate::error::OrchestratorError;
use crate::export_import_manager::{ConflictResolutions, ImportPreview, ImportSummary};
use crate::server::AppState;

/// Build the export/import routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/export", post(export_config))
        .route("/api/import/preview", post(import_preview))
        .route("/api/import", post(import_config))
}

// --- Request types ---

#[derive(Debug, Deserialize)]
struct ExportRequest {
    password: String,
}

#[derive(Debug, Deserialize)]
struct ImportPreviewRequest {
    /// Base64-encoded encrypted export data.
    data: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct ImportRequest {
    /// Base64-encoded encrypted export data.
    data: String,
    password: String,
    resolutions: ConflictResolutions,
}

// --- Handlers ---

/// POST /api/export — export all configuration as encrypted bytes.
///
/// Returns the encrypted data as application/octet-stream.
async fn export_config(
    State(state): State<AppState>,
    Json(req): Json<ExportRequest>,
) -> Result<impl IntoResponse, OrchestratorError> {
    if req.password.is_empty() {
        return Err(OrchestratorError::Validation(
            "Password must not be empty".to_string(),
        ));
    }

    let encrypted = state.export_import_manager.export(&req.password)?;

    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/octet-stream")],
        encrypted,
    ))
}

/// POST /api/import/preview — decrypt and preview import data without applying changes.
///
/// Accepts base64-encoded encrypted data + password, returns ImportPreview as JSON.
async fn import_preview(
    State(state): State<AppState>,
    Json(req): Json<ImportPreviewRequest>,
) -> Result<Json<ImportPreview>, OrchestratorError> {
    if req.password.is_empty() {
        return Err(OrchestratorError::Validation(
            "Password must not be empty".to_string(),
        ));
    }

    let data = base64::engine::general_purpose::STANDARD
        .decode(&req.data)
        .map_err(|e| OrchestratorError::Validation(format!("Invalid base64 data: {}", e)))?;

    let preview = state.export_import_manager.preview_import(&data, &req.password)?;

    Ok(Json(preview))
}

/// POST /api/import — import configuration data with conflict resolutions.
///
/// Accepts base64-encoded encrypted data, password, and resolutions.
/// Returns ImportSummary as JSON.
async fn import_config(
    State(state): State<AppState>,
    Json(req): Json<ImportRequest>,
) -> Result<Json<ImportSummary>, OrchestratorError> {
    if req.password.is_empty() {
        return Err(OrchestratorError::Validation(
            "Password must not be empty".to_string(),
        ));
    }

    let data = base64::engine::general_purpose::STANDARD
        .decode(&req.data)
        .map_err(|e| OrchestratorError::Validation(format!("Invalid base64 data: {}", e)))?;

    let summary = state
        .export_import_manager
        .import(&data, &req.password, &req.resolutions)?;

    Ok(Json(summary))
}
