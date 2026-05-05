use axum::{routing::get, Router};
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::error::OrchestratorError;

/// Shared application state holding all manager instances.
/// Managers will be added as they are implemented in subsequent tasks.
#[derive(Clone)]
pub struct AppState {
    // Placeholder — managers will be added in later tasks:
    // pub persona_manager: Arc<PersonaManager>,
    // pub agent_manager: Arc<AgentManager>,
    // pub session_manager: Arc<SessionManager>,
    // pub credential_manager: Arc<CredentialManager>,
    // pub policy_manager: Arc<PolicyManager>,
    // pub template_manager: Arc<TemplateManager>,
    // pub system_manager: Arc<SystemManager>,
    _placeholder: Arc<()>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            _placeholder: Arc::new(()),
        }
    }
}

/// Health check endpoint
async fn health() -> &'static str {
    "ok"
}

/// Start the Axum HTTP/WebSocket server bound to localhost only.
pub async fn start_server(
    _app_handle: tauri::AppHandle,
) -> Result<(), OrchestratorError> {
    let state = AppState::new();

    // CORS configured for localhost-only access
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            origin
                .to_str()
                .map(|s| s.starts_with("http://localhost") || s.starts_with("http://127.0.0.1"))
                .unwrap_or(false)
        }))
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any);

    let app = Router::new()
        .route("/api/health", get(health))
        // Additional routes will be added in task 16
        .layer(cors)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:9876")
        .await
        .map_err(|e| OrchestratorError::Internal(format!("Failed to bind server: {}", e)))?;

    println!("Axum server listening on http://127.0.0.1:9876");

    axum::serve(listener, app)
        .await
        .map_err(|e| OrchestratorError::Internal(format!("Server error: {}", e)))?;

    Ok(())
}
