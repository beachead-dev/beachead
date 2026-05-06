use axum::{extract::State, routing::get, Json, Router};
use serde::Serialize;

use crate::error::OrchestratorError;
use crate::server::AppState;

/// Build the MCP containers routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/mcp-containers", get(list_mcp_containers))
}

// --- Response types ---

/// Serializable MCP container info for the API response.
/// Excludes bearer_token for security.
#[derive(Debug, Clone, Serialize)]
struct McpContainerResponse {
    pub id: String,
    pub persona_id: String,
    pub shared_memory_id: Option<String>,
    pub container_id: Option<String>,
    pub port: u16,
    pub volume_name: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

// --- Handlers ---

/// GET /api/mcp-containers — list all MCP containers from the database.
///
/// Returns container metadata without bearer tokens (security: tokens are internal-only).
async fn list_mcp_containers(
    State(state): State<AppState>,
) -> Result<Json<Vec<McpContainerResponse>>, OrchestratorError> {
    let containers = state.db.with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, persona_id, shared_memory_id, container_id, port, volume_name, status, created_at, updated_at
                 FROM mcp_containers",
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(McpContainerResponse {
                    id: row.get(0)?,
                    persona_id: row.get(1)?,
                    shared_memory_id: row.get(2)?,
                    container_id: row.get(3)?,
                    port: row.get::<_, i64>(4)? as u16,
                    volume_name: row.get(5)?,
                    status: row.get(6)?,
                    created_at: row.get(7)?,
                    updated_at: row.get(8)?,
                })
            })
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;

        let mut containers = Vec::new();
        for row in rows {
            containers.push(row.map_err(|e| OrchestratorError::Database(e.to_string()))?);
        }
        Ok(containers)
    })?;

    Ok(Json(containers))
}
