pub mod personas;

use axum::Router;

use crate::server::AppState;

/// Build the complete API router with all route modules.
pub fn build_router() -> Router<AppState> {
    Router::new().merge(personas::router())
}
