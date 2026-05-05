use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;

use crate::error::OrchestratorError;
use crate::sbx::{DiagnoseResult, SbxVersion};
use crate::server::AppState;
use crate::types::DependencyStatus;

/// Build the system routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/system/version", get(get_version))
        .route("/api/system/diagnose", get(diagnose))
        .route("/api/system/auth-status", get(auth_status))
        .route("/api/system/login", post(login))
        .route("/api/system/logout", post(logout))
        .route("/api/system/help/{topic}", get(help_topic))
        .route("/api/system/dependency-check", get(dependency_check))
}

/// GET /api/system/version — get the sbx CLI version.
async fn get_version(
    State(state): State<AppState>,
) -> Result<Json<SbxVersion>, OrchestratorError> {
    let mgr = state.require_system_manager()?;
    let version = mgr.get_version().await?;
    Ok(Json(version))
}

/// GET /api/system/diagnose — run system diagnostics.
async fn diagnose(
    State(state): State<AppState>,
) -> Result<Json<DiagnoseResult>, OrchestratorError> {
    let mgr = state.require_system_manager()?;
    let result = mgr.diagnose().await?;
    Ok(Json(result))
}

/// Auth status response.
#[derive(Serialize)]
struct AuthStatusResponse {
    authenticated: bool,
}

/// GET /api/system/auth-status — check Docker authentication status.
async fn auth_status(
    State(state): State<AppState>,
) -> Result<Json<AuthStatusResponse>, OrchestratorError> {
    let mgr = state.require_system_manager()?;
    let authenticated = mgr.check_auth_status().await?;
    Ok(Json(AuthStatusResponse { authenticated }))
}

/// POST /api/system/login — initiate Docker login.
async fn login(
    State(state): State<AppState>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_system_manager()?;
    mgr.login().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/system/logout — sign out of Docker.
async fn logout(
    State(state): State<AppState>,
) -> Result<StatusCode, OrchestratorError> {
    let mgr = state.require_system_manager()?;
    mgr.logout().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/system/help/{topic} — serve static help content.
async fn help_topic(
    Path(topic): Path<String>,
) -> Result<String, OrchestratorError> {
    let content = match topic.as_str() {
        "getting-started" => include_str!("../help/getting-started.md"),
        "personas" => include_str!("../help/personas.md"),
        "agents" => include_str!("../help/agents.md"),
        "credentials" => include_str!("../help/credentials.md"),
        "sessions" => include_str!("../help/sessions.md"),
        "policies" => include_str!("../help/policies.md"),
        "templates" => include_str!("../help/templates.md"),
        "system-settings" => include_str!("../help/system-settings.md"),
        "troubleshooting" => include_str!("../help/troubleshooting.md"),
        "glossary" => include_str!("../help/glossary.md"),
        _ => {
            return Err(OrchestratorError::NotFound(format!(
                "Help topic '{}' not found",
                topic
            )));
        }
    };
    Ok(content.to_string())
}

/// GET /api/system/dependency-check — check availability of dependencies.
async fn dependency_check(
    State(state): State<AppState>,
) -> Result<Json<DependencyStatus>, OrchestratorError> {
    let mgr = state.require_system_manager()?;
    let status = mgr.dependency_check().await?;
    Ok(Json(status))
}
