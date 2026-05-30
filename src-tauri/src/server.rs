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
use crate::mcp_container_manager::McpContainerManager;
use crate::persona_manager::PersonaManager;
use crate::policy_manager::PolicyManager;
use crate::port_allocator::PortAllocator;
use crate::pty_bridge::PtyBridge;
use crate::repo_sync_manager::RepoSyncManager;
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
    pub mcp_container_manager: Option<Arc<McpContainerManager>>,
    pub repo_sync_manager: Option<Arc<RepoSyncManager>>,
    pub db: Arc<Database>,
    pub sbx: Option<Arc<SbxCli>>,
    pub pty_bridge: Arc<PtyBridge>,
    pub kit_generator: Arc<KitGenerator>,
}

impl AppState {
    /// Helper to get credential_manager or return an error.
    pub fn require_credential_manager(&self) -> Result<&Arc<CredentialManager>, OrchestratorError> {
        self.credential_manager.as_ref().ok_or_else(|| {
            OrchestratorError::SbxError(
                "sbx CLI is not available. Install Docker Sandboxes to use this feature."
                    .to_string(),
            )
        })
    }

    /// Helper to get session_manager or return an error.
    pub fn require_session_manager(&self) -> Result<&Arc<SessionManager>, OrchestratorError> {
        self.session_manager.as_ref().ok_or_else(|| {
            OrchestratorError::SbxError(
                "sbx CLI is not available. Install Docker Sandboxes to use this feature."
                    .to_string(),
            )
        })
    }

    /// Helper to get policy_manager or return an error.
    pub fn require_policy_manager(&self) -> Result<&Arc<PolicyManager>, OrchestratorError> {
        self.policy_manager.as_ref().ok_or_else(|| {
            OrchestratorError::SbxError(
                "sbx CLI is not available. Install Docker Sandboxes to use this feature."
                    .to_string(),
            )
        })
    }

    /// Helper to get template_manager or return an error.
    pub fn require_template_manager(&self) -> Result<&Arc<TemplateManager>, OrchestratorError> {
        self.template_manager.as_ref().ok_or_else(|| {
            OrchestratorError::SbxError(
                "sbx CLI is not available. Install Docker Sandboxes to use this feature."
                    .to_string(),
            )
        })
    }

    /// Helper to get system_manager or return an error.
    pub fn require_system_manager(&self) -> Result<&Arc<SystemManager>, OrchestratorError> {
        self.system_manager.as_ref().ok_or_else(|| {
            OrchestratorError::SbxError(
                "sbx CLI is not available. Install Docker Sandboxes to use this feature."
                    .to_string(),
            )
        })
    }

    /// Helper to get repo_sync_manager or return an error.
    pub fn require_repo_sync_manager(&self) -> Result<&Arc<RepoSyncManager>, OrchestratorError> {
        self.repo_sync_manager.as_ref().ok_or_else(|| {
            OrchestratorError::Internal(
                "git CLI is not available. Install git to use Repo Sync.".to_string(),
            )
        })
    }
}

/// Health check endpoint
async fn health() -> &'static str {
    "ok"
}

/// Get the application data directory path.
fn dirs_data_path() -> PathBuf {
    // On Linux, respect XDG_DATA_HOME before falling back to dirs::data_dir().
    // On macOS this gives ~/Library/Application Support/beachead.
    // On Windows this gives %APPDATA%\beachead.
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| dirs_home().join(".local").join("share"));
        return base.join("beachead");
    }
    #[cfg(not(target_os = "linux"))]
    {
        dirs::data_dir()
            .unwrap_or_else(|| dirs_home().join(".local").join("share"))
            .join("beachead")
    }
}

/// Get the user's home directory.
fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

/// Start the Axum HTTP/WebSocket server bound to localhost only.
pub async fn start_server(_app_handle: tauri::AppHandle) -> Result<(), OrchestratorError> {
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
    let credential_manager = sbx
        .as_ref()
        .map(|s| Arc::new(CredentialManager::new(s.clone())));
    let policy_manager = sbx
        .as_ref()
        .map(|s| Arc::new(PolicyManager::new(s.clone())));
    let template_manager = sbx
        .as_ref()
        .map(|s| Arc::new(TemplateManager::new(s.clone())));
    let system_manager = sbx
        .as_ref()
        .map(|s| Arc::new(SystemManager::new(s.clone())));

    // Seed built-in agents
    if let Err(e) = agent_manager.seed_builtin_agents() {
        eprintln!("Warning: failed to seed built-in agents: {}", e);
    }

    // Export/Import manager
    let export_import_manager = Arc::new(ExportImportManager::new(db.clone()));

    // MCP Container Manager (requires Docker — optional)
    // Created before SessionManager so it can be passed as a dependency.
    let port_allocator = Arc::new(PortAllocator::new(db.clone(), 9100, 9199));
    let mcp_container_manager = match McpContainerManager::new(db.clone(), port_allocator) {
        Ok(mgr) => {
            let mgr = Arc::new(mgr);
            // Register globally for shutdown cleanup
            let _ = crate::MCP_MANAGER.set(mgr.clone());
            // Ensure the MCP image exists and start all existing containers on startup
            let mgr_clone = mgr.clone();
            tokio::spawn(async move {
                if let Err(e) = mgr_clone.ensure_image_available().await {
                    eprintln!("Warning: MCP image not available: {}", e);
                }
                if let Err(e) = mgr_clone.start_all().await {
                    eprintln!("Warning: failed to start MCP containers: {}", e);
                }
                if let Err(e) = mgr_clone.reconcile().await {
                    eprintln!("Warning: failed to reconcile MCP containers: {}", e);
                }
            });
            Some(mgr)
        }
        Err(e) => {
            eprintln!(
                "MCP Container Manager unavailable (Docker not accessible): {}",
                e
            );
            None
        }
    };

    // Initialize Repo Sync Manager — graceful degradation if git not found (Req 19.7, 22.3)
    let repo_sync_manager = match discover_git_binary().await {
        Some(git_path) => {
            println!("Git binary found at: {}", git_path);
            let git = Arc::new(crate::git_cli::GitCli::new(git_path));
            let mirrors_dir = RepoSyncManager::default_mirrors_dir();
            std::fs::create_dir_all(&mirrors_dir).ok();
            let manager = RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir);
            let _handle = manager.start_background_checker();
            Some(Arc::new(manager))
        }
        None => {
            eprintln!("Warning: git not found in PATH. Repo Sync features disabled.");
            None
        }
    };

    // Session manager depends on sbx and optionally on mcp_container_manager and repo_sync_manager
    let session_manager = sbx.as_ref().map(|s| {
        Arc::new(SessionManager::new(
            db.clone(),
            s.clone(),
            kit_generator.clone(),
            pty_bridge.clone(),
            mcp_container_manager.clone(),
            repo_sync_manager.clone(),
        ))
    });

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
        mcp_container_manager,
        repo_sync_manager,
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

/// Discover the git binary path at startup.
///
/// On Linux/macOS: runs `which git` to find it in PATH.
/// On Windows: checks `ProgramFiles\Git\cmd\git.exe` first, then falls back to PATH.
/// Returns `None` if git is not found (graceful degradation).
async fn discover_git_binary() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        // Check common Windows install location first
        if let Ok(program_files) = std::env::var("ProgramFiles") {
            let git_path = PathBuf::from(&program_files)
                .join("Git")
                .join("cmd")
                .join("git.exe");
            if git_path.exists() {
                return Some(git_path.to_string_lossy().to_string());
            }
        }
        // Fall back to PATH via `where git`
        let output = tokio::process::Command::new("where")
            .arg("git")
            .output()
            .await
            .ok()?;
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let first_line = stdout.lines().next()?.trim().to_string();
            if !first_line.is_empty() {
                return Some(first_line);
            }
        }
        None
    }

    #[cfg(not(target_os = "windows"))]
    {
        let output = tokio::process::Command::new("which")
            .arg("git")
            .output()
            .await
            .ok()?;
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
        None
    }
}
