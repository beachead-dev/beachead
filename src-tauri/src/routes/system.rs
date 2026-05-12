use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post, put},
    Json, Router,
};
use serde::{Deserialize, Serialize};

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
        .route("/api/system/settings/{key}", get(get_setting))
        .route("/api/system/settings/{key}", put(set_setting))
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

/// Resolve a help topic identifier to its static markdown content.
///
/// Valid topics: "getting-started", "agents", "credentials", "policies",
/// "templates", "personas", "sessions", "docker", "system-settings", "troubleshooting", "glossary"
///
/// Returns the embedded markdown content for valid topics, or `OrchestratorError::NotFound`
/// for unrecognized topic identifiers.
pub fn resolve_help_topic(topic: &str) -> Result<&'static str, OrchestratorError> {
    match topic {
        "getting-started" => Ok(include_str!("../help/getting-started.md")),
        "personas" => Ok(include_str!("../help/personas.md")),
        "agents" => Ok(include_str!("../help/agents.md")),
        "credentials" => Ok(include_str!("../help/credentials.md")),
        "sessions" => Ok(include_str!("../help/sessions.md")),
        "policies" => Ok(include_str!("../help/policies.md")),
        "docker" => Ok(include_str!("../help/docker.md")),
        "templates" => Ok(include_str!("../help/templates.md")),
        "shared-memory" => Ok(include_str!("../help/shared-memory.md")),
        "system-settings" => Ok(include_str!("../help/system-settings.md")),
        "troubleshooting" => Ok(include_str!("../help/troubleshooting.md")),
        "glossary" => Ok(include_str!("../help/glossary.md")),
        _ => Err(OrchestratorError::NotFound(format!(
            "Help topic '{}' not found",
            topic
        ))),
    }
}

/// GET /api/system/help/{topic} — serve static help content.
async fn help_topic(
    Path(topic): Path<String>,
) -> Result<String, OrchestratorError> {
    let content = resolve_help_topic(&topic)?;
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

// --- User Settings ---

#[derive(Debug, Serialize)]
struct SettingResponse {
    key: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct SetSettingRequest {
    value: String,
}

/// GET /api/system/settings/{key} — get a user setting by key.
async fn get_setting(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> Result<Json<SettingResponse>, OrchestratorError> {
    let value = state.db.with_conn(|conn| {
        let result: Option<String> = conn
            .query_row(
                "SELECT value FROM user_settings WHERE key = ?1",
                rusqlite::params![key],
                |row| row.get(0),
            )
            .ok();
        Ok(result)
    })?;

    match value {
        Some(v) => Ok(Json(SettingResponse { key, value: v })),
        None => Err(OrchestratorError::NotFound(format!(
            "Setting '{}' not found",
            key
        ))),
    }
}

/// PUT /api/system/settings/{key} — set a user setting.
async fn set_setting(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(req): Json<SetSettingRequest>,
) -> Result<Json<SettingResponse>, OrchestratorError> {
    state.db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO user_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, req.value],
        )
        .map_err(|e| OrchestratorError::Database(e.to_string()))?;
        Ok(())
    })?;

    Ok(Json(SettingResponse {
        key,
        value: req.value,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// All valid help topic identifiers supported by the system.
    const VALID_TOPICS: &[&str] = &[
        "getting-started",
        "agents",
        "credentials",
        "policies",
        "docker",
        "templates",
        "personas",
        "sessions",
        "shared-memory",
        "system-settings",
        "troubleshooting",
        "glossary",
    ];

    /// Strategy that generates valid help topic identifiers.
    fn valid_topic_strategy() -> impl Strategy<Value = String> {
        prop::sample::select(VALID_TOPICS).prop_map(|s| s.to_string())
    }

    /// Strategy that generates invalid help topic identifiers.
    /// These are arbitrary strings that do NOT match any valid topic.
    fn invalid_topic_strategy() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_\\-]{1,50}".prop_filter(
            "must not be a valid topic",
            |s| !VALID_TOPICS.contains(&s.as_str()),
        )
    }

    proptest! {
        /// **Validates: Requirements 14.1, 14.2, 14.3, 14.4**
        ///
        /// Property 23: Help content topic resolution
        /// Valid topics always return non-empty markdown content.
        #[test]
        fn valid_topics_return_nonempty_markdown(topic in valid_topic_strategy()) {
            let result = resolve_help_topic(&topic);
            prop_assert!(result.is_ok(), "Expected Ok for valid topic '{}', got {:?}", topic, result);
            let content = result.unwrap();
            prop_assert!(!content.is_empty(), "Expected non-empty content for topic '{}'", topic);
            // Help content should be markdown — verify it contains at least one heading or text
            prop_assert!(content.len() > 10, "Content for '{}' is suspiciously short: {} bytes", topic, content.len());
        }

        /// **Validates: Requirements 14.1, 14.2, 14.3, 14.4**
        ///
        /// Property 23: Help content topic resolution
        /// Invalid topics always return a NotFound error.
        #[test]
        fn invalid_topics_return_not_found(topic in invalid_topic_strategy()) {
            let result = resolve_help_topic(&topic);
            prop_assert!(result.is_err(), "Expected Err for invalid topic '{}', got Ok", topic);
            match result.unwrap_err() {
                OrchestratorError::NotFound(msg) => {
                    prop_assert!(msg.contains(&topic), "Error message should contain the topic name");
                }
                other => {
                    prop_assert!(false, "Expected NotFound error, got {:?}", other);
                }
            }
        }
    }
}
