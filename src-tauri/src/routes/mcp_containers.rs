use axum::{
    extract::{Query, State},
    routing::get,
    Json, Router,
};
use bollard::container::ListContainersOptions;
use bollard::Docker;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::OrchestratorError;
use crate::server::AppState;

/// Build the MCP containers routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new().route("/api/mcp-containers", get(list_mcp_containers))
}

// --- Query parameters ---

#[derive(Debug, Deserialize)]
struct ListContainersQuery {
    /// When true, include unmanaged Docker containers with image `beachead-memory-mcp:latest`.
    /// Defaults to false (only show DB-tracked containers).
    #[serde(default)]
    show_all: bool,
}

// --- Response types ---

/// Enriched MCP container response with persona name and live status confirmation.
/// Excludes bearer_token for security.
#[derive(Debug, Clone, Serialize)]
pub struct McpContainerListResponse {
    pub id: String,
    pub persona_id: String,
    pub persona_name: String,
    pub container_id: Option<String>,
    pub port: u16,
    pub volume_name: String,
    pub status: String,
    pub live_status_confirmed: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Internal struct for DB query results (container joined with persona name).
struct DbContainerRow {
    id: String,
    persona_id: String,
    persona_name: String,
    container_id: Option<String>,
    port: u16,
    volume_name: String,
    status: String,
    created_at: String,
    updated_at: String,
}

// --- Handlers ---

/// GET /api/mcp-containers — list MCP containers enriched with persona names and live Docker status.
///
/// Query parameters:
/// - `show_all` (bool, default false): when true, also includes unmanaged Docker containers
///   with image `beachead-memory-mcp:latest` that are not tracked in the database.
///
/// For each container with a `container_id`, queries Docker for live status.
/// If Docker is unreachable or the container has no docker ID, returns the DB status
/// with `live_status_confirmed: false`.
async fn list_mcp_containers(
    State(state): State<AppState>,
    Query(params): Query<ListContainersQuery>,
) -> Result<Json<Vec<McpContainerListResponse>>, OrchestratorError> {
    // 1. Query DB: join mcp_containers with personas to get persona_name
    let db_rows: Vec<DbContainerRow> = state.db.with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT mc.id, mc.persona_id, COALESCE(p.name, '') as persona_name,
                        mc.container_id, mc.port, mc.volume_name, mc.status,
                        mc.created_at, mc.updated_at
                 FROM mcp_containers mc
                 LEFT JOIN personas p ON mc.persona_id = p.id
                 ORDER BY mc.created_at DESC",
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(DbContainerRow {
                    id: row.get(0)?,
                    persona_id: row.get(1)?,
                    persona_name: row.get(2)?,
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

    // 2. Try to connect to Docker for live status enrichment
    let docker = Docker::connect_with_local_defaults().ok();

    // 3. Enrich each DB container with live Docker status
    let mut results: Vec<McpContainerListResponse> = Vec::with_capacity(db_rows.len());

    for row in &db_rows {
        let (live_status, confirmed) = get_live_status(&docker, row.container_id.as_deref()).await;

        results.push(McpContainerListResponse {
            id: row.id.clone(),
            persona_id: row.persona_id.clone(),
            persona_name: row.persona_name.clone(),
            container_id: row.container_id.clone(),
            port: row.port,
            volume_name: row.volume_name.clone(),
            status: live_status.unwrap_or_else(|| row.status.clone()),
            live_status_confirmed: confirmed,
            created_at: row.created_at.clone(),
            updated_at: row.updated_at.clone(),
        });
    }

    // 4. If show_all=true, find unmanaged Docker containers with the MCP image
    if params.show_all {
        if let Some(ref docker) = docker {
            let unmanaged = find_unmanaged_containers(docker, &db_rows).await;
            results.extend(unmanaged);
        }
    }

    Ok(Json(results))
}

/// Query Docker for the live status of a container by its Docker container ID.
///
/// Returns (Some(status_string), true) if Docker reports the status successfully.
/// Returns (None, false) if Docker is unreachable or the container_id is None.
async fn get_live_status(
    docker: &Option<Docker>,
    container_id: Option<&str>,
) -> (Option<String>, bool) {
    let docker = match docker {
        Some(d) => d,
        None => return (None, false),
    };

    let container_id = match container_id {
        Some(id) if !id.is_empty() => id,
        _ => return (None, false),
    };

    match docker.inspect_container(container_id, None).await {
        Ok(info) => {
            let status = info
                .state
                .as_ref()
                .and_then(|s| s.status)
                .map(|s| format!("{:?}", s).to_lowercase());

            match status {
                Some(s) => (Some(s), true),
                None => (None, false),
            }
        }
        Err(_) => (None, false),
    }
}

/// Find Docker containers with image `beachead-memory-mcp:latest` that are NOT tracked in the DB.
///
/// Returns them as `McpContainerListResponse` entries with placeholder values for DB-only fields.
async fn find_unmanaged_containers(
    docker: &Docker,
    db_rows: &[DbContainerRow],
) -> Vec<McpContainerListResponse> {
    let mut filters = HashMap::new();
    filters.insert("ancestor".to_string(), vec!["beachead-memory-mcp:latest".to_string()]);

    let options = ListContainersOptions {
        all: true,
        filters,
        ..Default::default()
    };

    let docker_containers = match docker.list_containers(Some(options)).await {
        Ok(containers) => containers,
        Err(_) => return Vec::new(),
    };

    // Collect all Docker container IDs that are already tracked in the DB
    let tracked_ids: std::collections::HashSet<&str> = db_rows
        .iter()
        .filter_map(|r| r.container_id.as_deref())
        .collect();

    let mut unmanaged = Vec::new();

    for container in docker_containers {
        let docker_id = match container.id.as_deref() {
            Some(id) => id.to_string(),
            None => continue,
        };

        // Skip containers already tracked in the DB
        if tracked_ids.contains(docker_id.as_str()) {
            continue;
        }

        // Extract status from Docker container summary
        let status = container
            .state
            .as_deref()
            .unwrap_or("unknown")
            .to_string();

        // Extract port from the container's port bindings (first host port found)
        let port = container
            .ports
            .as_ref()
            .and_then(|ports| {
                ports.iter().find_map(|p| p.public_port.map(|pp| pp as u16))
            })
            .unwrap_or(0);

        // Extract container name (Docker prefixes with '/')
        let name = container
            .names
            .as_ref()
            .and_then(|names| names.first())
            .map(|n| n.trim_start_matches('/').to_string())
            .unwrap_or_default();

        // Extract volume name from mounts
        let volume_name = container
            .mounts
            .as_ref()
            .and_then(|mounts| {
                mounts.iter().find_map(|m| m.name.clone())
            })
            .unwrap_or_default();

        let created_at = container
            .created
            .map(|ts| chrono::DateTime::from_timestamp(ts, 0)
                .map(|dt| dt.to_rfc3339())
                .unwrap_or_default())
            .unwrap_or_default();

        unmanaged.push(McpContainerListResponse {
            id: format!("unmanaged-{}", docker_id),
            persona_id: String::new(),
            persona_name: name,
            container_id: Some(docker_id),
            port,
            volume_name,
            status,
            live_status_confirmed: true,
            created_at,
            updated_at: String::new(),
        });
    }

    unmanaged
}
