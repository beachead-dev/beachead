pub mod agents;
pub mod personas;
pub mod policies;
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
}
