pub mod agents;
pub mod export_import;
pub mod mcp_containers;
pub mod personas;
pub mod policies;
pub mod repo_sync;
pub mod sandboxes;
pub mod secrets;
pub mod sessions;
pub mod system;
pub mod templates;

use axum::Router;

use crate::server::AppState;

/// Build the complete API router with all route modules.
pub fn build_router() -> Router<AppState> {
    Router::new()
        .merge(personas::router())
        .merge(agents::router())
        .merge(secrets::router())
        .merge(sessions::router())
        .merge(sandboxes::router())
        .merge(policies::router())
        .merge(templates::router())
        .merge(system::router())
        .merge(export_import::router())
        .merge(mcp_containers::router())
        .merge(repo_sync::router())
}
