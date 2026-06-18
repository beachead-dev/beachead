use axum::response::IntoResponse;
use axum::{routing::get, Router};
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::ServeDir;

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
    /// Per-launch API token required on all `/api/*` routes (except health).
    pub api_token: Arc<String>,
    /// Resolved frontend `dist/` directory, if found. Used to serve the
    /// token-injected `index.html`.
    pub frontend_dist: Option<Arc<PathBuf>>,
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

/// Well-known token accepted ONLY in debug builds (`cargo`/`tauri dev`).
///
/// The Vite dev server loads `index.html` itself, so the server-side token
/// injection never reaches the dev webview. In debug builds the server accepts
/// this fixed token, and the dev frontend supplies it via `VITE_API_TOKEN`
/// (see vite.config.ts). Release builds compile with `debug_assertions` off,
/// generate a random per-launch token, and never accept this value.
pub const DEV_API_TOKEN: &str = "dev-token-not-valid-in-release-builds";

/// Health check endpoint
async fn health() -> &'static str {
    "ok"
}

/// Constant-time byte-slice equality. Returns false for differing lengths.
/// Token lengths are fixed, so the length check leaks nothing useful.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Extract the API token from a request: `Authorization: Bearer <token>` header
/// (preferred) or `?token=<token>` query parameter (used by the WebSocket
/// terminal, since browsers can't set headers on WebSocket connections).
fn extract_request_token(headers: &axum::http::HeaderMap, query: Option<&str>) -> Option<String> {
    if let Some(auth) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(s) = auth.to_str() {
            if let Some(token) = s.strip_prefix("Bearer ") {
                return Some(token.to_string());
            }
        }
    }
    if let Some(q) = query {
        for pair in q.split('&') {
            if let Some(value) = pair.strip_prefix("token=") {
                // Token is URL-safe base64 (no percent-encoding needed).
                return Some(value.to_string());
            }
        }
    }
    None
}

/// Middleware that enforces the per-launch API token on all `/api/*` routes
/// except `/api/health`. Fails closed with a bare 401.
async fn require_api_token(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let provided = extract_request_token(req.headers(), req.uri().query());

    let authorized = match provided {
        Some(token) => constant_time_eq(token.as_bytes(), state.api_token.as_bytes()),
        None => false,
    };

    if authorized {
        next.run(req).await
    } else {
        (axum::http::StatusCode::UNAUTHORIZED, "Unauthorized").into_response()
    }
}

/// Serve `index.html` with the per-launch API token injected as a `<meta>` tag.
///
/// A `<meta>` tag is used rather than an inline `<script>` because the webview
/// CSP (`default-src 'self'`) forbids inline scripts. Static assets are served
/// directly by `ServeDir`; only the SPA entry point passes through here.
async fn serve_index_with_token(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> axum::response::Response {
    let Some(dist) = state.frontend_dist.as_ref() else {
        return (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "frontend not available",
        )
            .into_response();
    };

    let index_path = dist.join("index.html");
    let html = match std::fs::read_to_string(&index_path) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Failed to read index.html: {}", e);
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                "frontend not available",
            )
                .into_response();
        }
    };

    // Token is URL-safe base64 (chars [A-Za-z0-9_-]) so it is safe in an
    // attribute, but escape the quote defensively in case the source changes.
    let token_attr = state.api_token.replace('"', "&quot;");
    let meta = format!("<meta name=\"beachead-token\" content=\"{}\">", token_attr);
    // Inject before <meta charset (first element in <head>) to guarantee the
    // token is in the DOM before any scripts execute. Fall back to </head> or
    // file start if the expected structure isn't found.
    let injected = if let Some(pos) = html.find("<meta charset") {
        let (head, tail) = html.split_at(pos);
        format!("{head}{meta}\n    {tail}")
    } else if let Some(pos) = html.find("</head>") {
        let (head, tail) = html.split_at(pos);
        format!("{head}{meta}{tail}")
    } else {
        format!("{meta}{html}")
    };

    // Prevent the webview from caching index.html — the token changes every
    // launch, so a stale cached page would send the wrong token and get 401.
    (
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        axum::response::Html(injected),
    )
        .into_response()
}

/// Exact-origin allowlist for CORS.
///
/// Returns true only for the dev webview origin (Vite) and the production
/// server origin. Uses exact string matching — never prefix matching — so
/// look-alike hostnames such as `http://localhost.attacker.com` are rejected.
fn is_allowed_origin(origin: &str) -> bool {
    const ALLOWED: &[&str] = &[
        // Vite dev server (npm run dev)
        "http://localhost:5173",
        "http://127.0.0.1:5173",
        // Production: webview is served from the Axum server itself
        "http://localhost:9876",
        "http://127.0.0.1:9876",
    ];
    ALLOWED.contains(&origin)
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
        base.join("beachead")
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
///
/// If `ready_signal` is provided, sends `()` once the listener is bound
/// so the caller knows it's safe to open the webview.
pub async fn start_server(
    app_handle: tauri::AppHandle,
    ready_signal: Option<std::sync::mpsc::Sender<()>>,
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

    // Resolve frontend dist/ directory before building state (the token-injected
    // index handler needs it).
    // Search order:
    //   1. Dev layout: CARGO_MANIFEST_DIR/../dist (works during `cargo run`)
    //   2. Tauri resource dir: bundled dist/ in the installed .deb/.app/.msi
    //   3. Exe sibling: <binary_dir>/dist (manual deployment)
    let dist_path = {
        let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist");
        if dev.join("index.html").exists() {
            Some(dev)
        } else {
            // Try Tauri resource directory (where `bundle.resources` are placed).
            // Tauri encodes `../dist` as `_up_/dist` inside the resource dir.
            use tauri::Manager;
            let resource_base = app_handle.path().resource_dir().ok();
            let resource_dist = resource_base.as_ref().and_then(|r| {
                // Try direct `dist/` first, then Tauri's `_up_/dist/` encoding
                let direct = r.join("dist");
                if direct.join("index.html").exists() {
                    return Some(direct);
                }
                let up_encoded = r.join("_up_").join("dist");
                if up_encoded.join("index.html").exists() {
                    return Some(up_encoded);
                }
                None
            });
            if resource_dist.is_some() {
                resource_dist
            } else {
                if let Some(ref rb) = resource_base {
                    eprintln!(
                        "dist/ not found at resource_dir {:?} — trying exe sibling",
                        rb
                    );
                }
                // Fallback: exe sibling
                std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(|d| d.join("dist")))
                    .filter(|p| p.join("index.html").exists())
            }
        }
    };

    if dist_path.is_none() {
        eprintln!("Warning: dist/ directory not found — frontend will not be served (token injection disabled)");
    } else {
        println!(
            "Frontend dist/ resolved at: {:?}",
            dist_path.as_ref().unwrap()
        );
    }

    // Per-launch API token. Release builds use a random 256-bit token generated
    // fresh each launch. Debug builds use a fixed well-known token so the Vite
    // dev server (which serves its own index.html) can authenticate via
    // VITE_API_TOKEN without server-side injection.
    let api_token = if cfg!(debug_assertions) {
        DEV_API_TOKEN.to_string()
    } else {
        crate::token::generate_bearer_token()
    };

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
        api_token: Arc::new(api_token),
        frontend_dist: dist_path.clone().map(Arc::new),
    };

    // Exact-origin allowlist. In a released build the webview is served from
    // http://127.0.0.1:9876 (see tauri.conf.json `frontendDist`), so production
    // requests are same-origin and never exercise CORS. These entries cover the
    // dev webview (Vite on :5173) and the production origin for completeness.
    // A `starts_with` match is intentionally avoided — it would accept hostile
    // origins like `http://localhost.attacker.com`.
    let cors = CorsLayer::new()
        .allow_origin(AllowOrigin::predicate(|origin, _| {
            origin.to_str().map(is_allowed_origin).unwrap_or(false)
        }))
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any);

    // All /api/* routes (except /api/health) require the API token. The health
    // route is registered separately so it stays unauthenticated for readiness
    // probes.
    let api_routes = crate::routes::build_router().layer(axum::middleware::from_fn_with_state(
        state.clone(),
        require_api_token,
    ));

    let mut app = Router::new()
        .route("/api/health", get(health))
        .merge(api_routes);

    // Serve frontend static files from dist/ so the webview can load via HTTP.
    // This avoids the tauri:// protocol which fails on some WebKitGTK versions.
    // The SPA entry point (index.html) is ALWAYS served via the injection handler
    // (which adds the per-launch token as a <meta> tag). Static assets (JS, CSS,
    // images) go through ServeDir directly. The fallback also routes to the
    // injection handler for SPA client-side routes.
    if let Some(dist) = dist_path {
        let index_fallback = get(serve_index_with_token).with_state(state.clone());
        let serve_dir = ServeDir::new(&dist)
            .append_index_html_on_directories(false)
            .fallback(index_fallback);
        app = app
            .route("/", get(serve_index_with_token))
            .route("/index.html", get(serve_index_with_token))
            .fallback_service(serve_dir);
    } else {
        eprintln!("Warning: dist/ directory not found — frontend will not be served");
    }

    // Apply CORS and shared state once, after all routes are registered.
    let app = app.layer(cors).with_state(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:9876")
        .await
        .map_err(|e| OrchestratorError::Internal(format!("Failed to bind server: {}", e)))?;

    println!("Axum server listening on http://127.0.0.1:9876");

    // Signal that the server is ready to accept connections
    if let Some(tx) = ready_signal {
        let _ = tx.send(());
    }

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

#[cfg(test)]
mod tests {
    use super::{constant_time_eq, extract_request_token, is_allowed_origin};
    use axum::http::{header::AUTHORIZATION, HeaderMap, HeaderValue};

    #[test]
    fn test_allows_dev_and_production_origins() {
        assert!(is_allowed_origin("http://localhost:5173"));
        assert!(is_allowed_origin("http://127.0.0.1:5173"));
        assert!(is_allowed_origin("http://localhost:9876"));
        assert!(is_allowed_origin("http://127.0.0.1:9876"));
    }

    #[test]
    fn test_rejects_lookalike_hostnames() {
        // The previous `starts_with` predicate accepted all of these.
        assert!(!is_allowed_origin("http://localhost.attacker.com"));
        assert!(!is_allowed_origin("http://127.0.0.1.attacker.com"));
        assert!(!is_allowed_origin("http://localhost:5173.attacker.com"));
        assert!(!is_allowed_origin("http://localhostX:9876"));
    }

    #[test]
    fn test_rejects_other_origins() {
        assert!(!is_allowed_origin("https://example.com"));
        assert!(!is_allowed_origin("http://localhost:3000"));
        assert!(!is_allowed_origin("tauri://localhost"));
        assert!(!is_allowed_origin(""));
    }

    #[test]
    fn test_constant_time_eq() {
        assert!(constant_time_eq(b"abc123", b"abc123"));
        assert!(!constant_time_eq(b"abc123", b"abc124"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
        assert!(!constant_time_eq(b"", b"x"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn test_extract_token_from_header() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer my-token"));
        assert_eq!(
            extract_request_token(&headers, None),
            Some("my-token".to_string())
        );
    }

    #[test]
    fn test_extract_token_from_query() {
        let headers = HeaderMap::new();
        assert_eq!(
            extract_request_token(&headers, Some("foo=bar&token=abc123&x=y")),
            Some("abc123".to_string())
        );
        assert_eq!(
            extract_request_token(&headers, Some("token=only")),
            Some("only".to_string())
        );
    }

    #[test]
    fn test_extract_token_header_preferred_over_query() {
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer from-header"),
        );
        assert_eq!(
            extract_request_token(&headers, Some("token=from-query")),
            Some("from-header".to_string())
        );
    }

    #[test]
    fn test_extract_token_absent() {
        let headers = HeaderMap::new();
        assert_eq!(extract_request_token(&headers, None), None);
        assert_eq!(extract_request_token(&headers, Some("foo=bar")), None);
    }

    #[test]
    fn test_extract_token_ignores_non_bearer_auth() {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert_eq!(extract_request_token(&headers, None), None);
    }
}
