use axum::{routing::get, Router};
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};

use crate::agent_manager::AgentManager;
use crate::credential_manager::CredentialManager;
use crate::db::Database;
use crate::error::OrchestratorError;
use crate::export_import_manager::ExportImportManager;
use crate::kit_generator::KitGenerator;
use crate::persona_manager::PersonaManager;
use crate::policy_manager::PolicyManager;
use crate::pty_bridge::PtyBridge;
use crate::sbx::SbxCli;
use crate::session_manager::SessionManager;
use crate::system_manager::SystemManager;
use crate::template_manager::TemplateManager;

/// Shared application state holding all manager instances.
///
/// Managers that depend on `SbxCli` are wrapped in `Option` because
/// sbx may not be installed on the host system. Route handlers must
/// check availability and return an appropriate error if None.
#[derive(Clone)]
pub struct AppState {
    pub persona_manager: Arc<PersonaManager>,
    pub agent_manager: Arc<AgentManager>,
    pub credential_manager: Option<Arc<CredentialManager>>,
    pub session_manager: Option<Arc<SessionManager>>,
    pub policy_manager: Option<Arc<PolicyManager>>,
    pub template_manager: Option<Arc<TemplateManager>>,
    pub system_manager: Option<Arc<SystemManager>>,
    pub export_import_manager: Arc<ExportImportManager>,
    pub db: Arc<Database>,
    pub sbx: Option<Arc<SbxCli>>,
    pub pty_bridge: Arc<PtyBridge>,
    pub kit_generator: Arc<KitGenerator>,
}

impl AppState {
    /// Helper to get credential_manager or return an error.
    pub fn require_credential_manager(&self) -> Result<&Arc<CredentialManager>, OrchestratorError> {
        self.credential_manager.as_ref().ok_or_else(|| {
            OrchestratorError::SbxError("sbx CLI is not available. Install Docker Sandboxes to use this feature.".to_string())
        })
    }

    /// Helper to get session_manager or return an error.
    pub fn require_session_manager(&self) -> Result<&Arc<SessionManager>, OrchestratorError> {
        self.session_manager.as_ref().ok_or_else(|| {
            OrchestratorError::SbxError("sbx CLI is not available. Install Docker Sandboxes to use this feature.".to_string())
        })
    }

    /// Helper to get policy_manager or return an error.
    pub fn require_policy_manager(&self) -> Result<&Arc<PolicyManager>, OrchestratorError> {
        self.policy_manager.as_ref().ok_or_else(|| {
            OrchestratorError::SbxError("sbx CLI is not available. Install Docker Sandboxes to use this feature.".to_string())
        })
    }

    /// Helper to get template_manager or return an error.
    pub fn require_template_manager(&self) -> Result<&Arc<TemplateManager>, OrchestratorError> {
        self.template_manager.as_ref().ok_or_else(|| {
            OrchestratorError::SbxError("sbx CLI is not available. Install Docker Sandboxes to use this feature.".to_string())
        })
    }

    /// Helper to get system_manager or return an error.
    pub fn require_system_manager(&self) -> Result<&Arc<SystemManager>, OrchestratorError> {
        self.system_manager.as_ref().ok_or_else(|| {
            OrchestratorError::SbxError("sbx CLI is not available. Install Docker Sandboxes to use this feature.".to_string())
        })
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
        .unwrap_or_else(|_| dirs_home().join(".local").join("share"));
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
    let data_path = dirs_data_path();
    std::fs::create_dir_all(&data_path).ok();

    let db_path = data_path.join("beachead.db");
    let db = Arc::new(crate::db::Database::open(&db_path)?);

    // Initialize SbxCli — may not be available if sbx is not installed
    let sbx: Option<Arc<SbxCli>> = match SbxCli::new() {
        Ok(cli) => Some(Arc::new(cli)),
        Err(e) => {
            eprintln!("sbx CLI not available: {}. Sandbox features disabled.", e);
            None
        }
    };

    // Core managers (always available)
    let persona_manager = Arc::new(PersonaManager::new(db.clone()));
    let agent_manager = Arc::new(AgentManager::new(db.clone(), sbx.clone()));
    let pty_bridge = Arc::new(PtyBridge::new());
    let kit_base_dir = data_path.join("kits");
    std::fs::create_dir_all(&kit_base_dir).ok();
    let kit_generator = Arc::new(KitGenerator::new(kit_base_dir));

    // sbx-dependent managers (None if sbx not available)
    let credential_manager = sbx.as_ref().map(|s| Arc::new(CredentialManager::new(s.clone())));
    let policy_manager = sbx.as_ref().map(|s| Arc::new(PolicyManager::new(s.clone())));
    let template_manager = sbx.as_ref().map(|s| Arc::new(TemplateManager::new(s.clone())));
    let system_manager = sbx.as_ref().map(|s| Arc::new(SystemManager::new(s.clone())));
    let session_manager = sbx.as_ref().map(|s| {
        Arc::new(SessionManager::new(
            db.clone(),
            s.clone(),
            kit_generator.clone(),
            pty_bridge.clone(),
        ))
    });

    // Seed built-in agents
    if let Err(e) = agent_manager.seed_builtin_agents() {
        eprintln!("Warning: failed to seed built-in agents: {}", e);
    }

    // Export/Import manager
    let export_import_manager = Arc::new(ExportImportManager::new(db.clone()));

    // Spawn session recovery as a non-blocking background task (Req 5.1–5.7)
    if let Some(ref sm) = session_manager {
        let sm_clone = sm.clone();
        tokio::spawn(async move {
            let results = sm_clone.recover_sessions().await;
            for result in &results {
                match result {
                    crate::session_manager::RecoveryResult::Recovered(id) => {
                        println!("Session recovery: recovered session {}", id);
                    }
                    crate::session_manager::RecoveryResult::Failed { session_id, reason } => {
                        eprintln!(
                            "Session recovery: failed to recover session {}: {}",
                            session_id, reason
                        );
                    }
                }
            }
            if results.is_empty() {
                println!("Session recovery: no active sessions to recover");
            }
        });
    }

    let state = AppState {
        persona_manager,
        agent_manager,
        credential_manager,
        session_manager,
        policy_manager,
        template_manager,
        system_manager,
        export_import_manager,
        db,
        sbx,
        pty_bridge,
        kit_generator,
    };

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
        .merge(crate::routes::build_router())
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
