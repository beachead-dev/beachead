use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};

use crate::error::OrchestratorError;
use crate::server::AppState;
use crate::types::{SecretStatus, SetSecretRequest};

/// Build the secret routes sub-router.
///
/// SECURITY: Request bodies containing secret values are never logged.
/// The CredentialManager uses zeroize to clear secrets from memory.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/secrets", get(list_secrets))
        .route(
            "/api/secrets/{service}",
            post(set_secret).delete(remove_secret),
        )
        .route("/api/secrets/{service}/oauth", post(set_secret_oauth))
}

/// GET /api/secrets — list all configured secrets (names and status only).
async fn list_secrets(
    State(state): State<AppState>,
) -> Result<Json<Vec<SecretStatus>>, OrchestratorError> {
    let mgr = state.require_credential_manager()?;
    let secrets = mgr.list_secrets().await?;
    Ok(Json(secrets))
}

/// POST /api/secrets/{service} — set a secret value for a service.
///
/// SECURITY: The request body contains a secret value. It is passed directly
/// to CredentialManager which zeroizes it after use. Never log this body.
async fn set_secret(
    State(state): State<AppState>,
    Path(service): Path<String>,
    Json(req): Json<SetSecretRequest>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_credential_manager()?;
    mgr.set_secret(&service, req.value).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/secrets/{service} — remove a secret for a service.
async fn remove_secret(
    State(state): State<AppState>,
    Path(service): Path<String>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_credential_manager()?;
    mgr.remove_secret(&service).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/secrets/{service}/oauth — initiate OAuth flow for a service.
async fn set_secret_oauth(
    State(state): State<AppState>,
    Path(service): Path<String>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_credential_manager()?;
    mgr.set_secret_oauth(&service).await?;
    Ok(StatusCode::NO_CONTENT)
}
