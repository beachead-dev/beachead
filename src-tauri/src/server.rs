use axum::{routing::get, Router};
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::error::OrchestratorError;
use crate::persona_manager::PersonaManager;

/// Shared application state holding all manager instances.
/// Managers will be added as they are implemented in subsequent tasks.
#[derive(Clone)]
pub struct AppState {
    pub persona_manager: Arc<PersonaManager>,
    // Additional managers will be added in later tasks:
    // pub agent_manager: Arc<AgentManager>,
    // pub session_manager: Arc<SessionManager>,
    // pub credential_manager: Arc<CredentialManager>,
    // pub policy_manager: Arc<PolicyManager>,
    // pub template_manager: Arc<TemplateManager>,
    // pub system_manager: Arc<SystemManager>,
}

impl AppState {
    pub fn new(persona_manager: Arc<PersonaManager>) -> Self {
        Self { persona_manager }
    }
}

/// Health check endpoint
async fn health() -> &'static str {
    "ok"
}

/// Get the application data directory path.
fn dirs_data_path() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs_home().join(".local").join("share")
        });
    base.join("beachead")
}

/// Get the user's home directory.
fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Start the Axum HTTP/WebSocket server bound to localhost only.
pub async fn start_server(
    _app_handle: tauri::AppHandle,
) -> Result<(), OrchestratorError> {
    let db_path = dirs_data_path().join("beachead.db");
    let db = Arc::new(crate::db::Database::open(&db_path)?);
    let persona_manager = Arc::new(PersonaManager::new(db));
    let state = AppState::new(persona_manager);

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
