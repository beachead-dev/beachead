use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
    Json, Router,
};
use bollard::container::{
    ListContainersOptions, RemoveContainerOptions, StartContainerOptions, StopContainerOptions,
};
use bollard::Docker;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::error::OrchestratorError;
use crate::mcp_container_manager::ContainerStatus;
use crate::server::AppState;

/// Build the MCP containers routes sub-router.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/mcp-containers", get(list_mcp_containers))
        .route("/api/mcp-containers/{id}/start", post(start_container))
        .route("/api/mcp-containers/{id}/stop", post(stop_container))
        .route("/api/mcp-containers/{id}", delete(remove_container))
}

// --- Query parameters ---

#[derive(Debug, Deserialize)]
struct ListContainersQuery {
    /// When true, include unmanaged Docker containers with image `beachead-memory-mcp:latest`.
    /// Defaults to false (only show DB-tracked containers).
    #[serde(default)]
    show_all: bool,
}

#[derive(Debug, Deserialize)]
struct DeleteContainerQuery {
    /// When true, also delete the Docker volume `beachead-memory-{persona_id}`.
    /// Defaults to false.
    #[serde(default)]
    delete_volume: bool,
}

// --- Response types ---

/// Enriched MCP container response with persona name and live status confirmation.
/// Excludes bearer_token for security.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpContainerListResponse {
    pub id: String,
    pub persona_id: String,
    pub persona_name: String,
    pub container_id: Option<String>,
    pub image: String,
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
            image: "beachead-memory-mcp:latest".to_string(),
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

/// Find all Docker containers that are NOT tracked in the DB.
///
/// When show_all is enabled, returns all Docker containers (regardless of image)
/// that are not already tracked in the Beachead database.
/// Returns them as `McpContainerListResponse` entries with placeholder values for DB-only fields.
async fn find_unmanaged_containers(
    docker: &Docker,
    db_rows: &[DbContainerRow],
) -> Vec<McpContainerListResponse> {
    let options = ListContainersOptions::<String> {
        all: true,
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

        // Extract image name
        let image = container
            .image
            .as_deref()
            .unwrap_or("unknown")
            .to_string();

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
            image,
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

/// POST /api/mcp-containers/{id}/start — start a stopped MCP container.
///
/// Looks up the container by its database primary key `id` (not the Docker container_id).
/// - If not found in DB, returns 404.
/// - If already running (confirmed via Docker), returns 200 with current record (idempotent).
/// - Otherwise, starts the Docker container via bollard, updates DB status to "running",
///   and returns 200 with the updated container record.
/// - On Docker failure, updates DB status to "failed" and returns 500.
async fn start_container(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<McpContainerListResponse>, OrchestratorError> {
    // 1. Look up the container in the DB by its primary key
    let db_row: DbContainerRow = state.db.with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT mc.id, mc.persona_id, COALESCE(p.name, '') as persona_name,
                        mc.container_id, mc.port, mc.volume_name, mc.status,
                        mc.created_at, mc.updated_at
                 FROM mcp_containers mc
                 LEFT JOIN personas p ON mc.persona_id = p.id
                 WHERE mc.id = ?1",
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;

        stmt.query_row(rusqlite::params![id], |row| {
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
        .optional()
        .map_err(|e| OrchestratorError::Database(e.to_string()))
    })?
    .ok_or_else(|| OrchestratorError::NotFound(format!("Container '{}' not found", id)))?;

    // 2. Connect to Docker
    let docker = Docker::connect_with_local_defaults()
        .map_err(|e| OrchestratorError::DockerError(format!("Failed to connect to Docker: {}", e)))?;

    // 3. Check if already running via Docker inspect
    if let Some(ref docker_id) = db_row.container_id {
        if !docker_id.is_empty() {
            if let Ok(info) = docker.inspect_container(docker_id, None).await {
                let is_running = info
                    .state
                    .as_ref()
                    .and_then(|s| s.running)
                    .unwrap_or(false);

                if is_running {
                    // Already running — return current record (idempotent)
                    return Ok(Json(McpContainerListResponse {
                        id: db_row.id,
                        persona_id: db_row.persona_id,
                        persona_name: db_row.persona_name,
                        container_id: db_row.container_id,
                        image: "beachead-memory-mcp:latest".to_string(),
                        port: db_row.port,
                        volume_name: db_row.volume_name,
                        status: "running".to_string(),
                        live_status_confirmed: true,
                        created_at: db_row.created_at,
                        updated_at: db_row.updated_at,
                    }));
                }
            }
        }
    }

    // 4. Start the container via bollard
    let docker_id = db_row.container_id.as_deref().ok_or_else(|| {
        OrchestratorError::DockerError("Container has no Docker container ID".to_string())
    })?;

    if let Err(e) = docker
        .start_container(docker_id, None::<StartContainerOptions<String>>)
        .await
    {
        // Docker failure — update status to "failed" and return 500
        let now = chrono::Utc::now().to_rfc3339();
        let _ = state.db.with_conn(|conn| {
            conn.execute(
                "UPDATE mcp_containers SET status = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![ContainerStatus::Failed.as_str(), now, db_row.id],
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))
        });

        return Err(OrchestratorError::DockerError(format!(
            "Failed to start container '{}': {}",
            id, e
        )));
    }

    // 5. Update DB status to "running"
    let now = chrono::Utc::now().to_rfc3339();
    state.db.with_conn(|conn| {
        conn.execute(
            "UPDATE mcp_containers SET status = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![ContainerStatus::Running.as_str(), now, db_row.id],
        )
        .map_err(|e| OrchestratorError::Database(e.to_string()))
    })?;

    // 6. Return the updated container record
    Ok(Json(McpContainerListResponse {
        id: db_row.id,
        persona_id: db_row.persona_id,
        persona_name: db_row.persona_name,
        container_id: db_row.container_id,
        image: "beachead-memory-mcp:latest".to_string(),
        port: db_row.port,
        volume_name: db_row.volume_name,
        status: "running".to_string(),
        live_status_confirmed: true,
        created_at: db_row.created_at,
        updated_at: now,
    }))
}

/// POST /api/mcp-containers/{id}/stop — stop a running MCP container.
///
/// Looks up the container by its database primary key `id` (not the Docker container_id).
/// - If not found in DB, returns 404.
/// - If already stopped (DB status is "stopped"), returns 200 with current record (idempotent).
/// - Otherwise, stops the Docker container via bollard with a 10-second timeout,
///   updates DB status to "stopped", and returns 200 with the updated container record.
/// - On Docker failure, updates DB status to "failed" and returns error.
async fn stop_container(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<McpContainerListResponse>, OrchestratorError> {
    // 1. Look up the container in the DB by its primary key
    let db_row: DbContainerRow = state.db.with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT mc.id, mc.persona_id, COALESCE(p.name, '') as persona_name,
                        mc.container_id, mc.port, mc.volume_name, mc.status,
                        mc.created_at, mc.updated_at
                 FROM mcp_containers mc
                 LEFT JOIN personas p ON mc.persona_id = p.id
                 WHERE mc.id = ?1",
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;

        stmt.query_row(rusqlite::params![id], |row| {
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
        .optional()
        .map_err(|e| OrchestratorError::Database(e.to_string()))
    })?
    .ok_or_else(|| OrchestratorError::NotFound(format!("Container '{}' not found", id)))?;

    // 2. If already stopped, return current record (idempotent)
    if db_row.status == "stopped" {
        return Ok(Json(McpContainerListResponse {
            id: db_row.id,
            persona_id: db_row.persona_id,
            persona_name: db_row.persona_name,
            container_id: db_row.container_id,
            image: "beachead-memory-mcp:latest".to_string(),
            port: db_row.port,
            volume_name: db_row.volume_name,
            status: "stopped".to_string(),
            live_status_confirmed: false,
            created_at: db_row.created_at,
            updated_at: db_row.updated_at,
        }));
    }

    // 3. Connect to Docker
    let docker = Docker::connect_with_local_defaults()
        .map_err(|e| OrchestratorError::DockerError(format!("Failed to connect to Docker: {}", e)))?;

    // 4. Stop the container via bollard with 10-second timeout
    let docker_id = db_row.container_id.as_deref().ok_or_else(|| {
        OrchestratorError::DockerError("Container has no Docker container ID".to_string())
    })?;

    if let Err(e) = docker
        .stop_container(docker_id, Some(StopContainerOptions { t: 10 }))
        .await
    {
        // Docker failure — update status to "failed" and return error
        let now = chrono::Utc::now().to_rfc3339();
        let _ = state.db.with_conn(|conn| {
            conn.execute(
                "UPDATE mcp_containers SET status = ?1, updated_at = ?2 WHERE id = ?3",
                rusqlite::params![ContainerStatus::Failed.as_str(), now, db_row.id],
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))
        });

        return Err(OrchestratorError::DockerError(format!(
            "Failed to stop container '{}': {}",
            id, e
        )));
    }

    // 5. Update DB status to "stopped"
    let now = chrono::Utc::now().to_rfc3339();
    state.db.with_conn(|conn| {
        conn.execute(
            "UPDATE mcp_containers SET status = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![ContainerStatus::Stopped.as_str(), now, db_row.id],
        )
        .map_err(|e| OrchestratorError::Database(e.to_string()))
    })?;

    // 6. Return the updated container record
    Ok(Json(McpContainerListResponse {
        id: db_row.id,
        persona_id: db_row.persona_id,
        persona_name: db_row.persona_name,
        container_id: db_row.container_id,
        image: "beachead-memory-mcp:latest".to_string(),
        port: db_row.port,
        volume_name: db_row.volume_name,
        status: "stopped".to_string(),
        live_status_confirmed: true,
        created_at: db_row.created_at,
        updated_at: now,
    }))
}

/// DELETE /api/mcp-containers/{id} — remove an MCP container.
///
/// Looks up the container by its database primary key `id`.
/// - If not found in DB, returns 404.
/// - Stops the Docker container (best-effort, errors ignored).
/// - Removes the Docker container with force (best-effort, errors ignored).
/// - If `delete_volume=true`, deletes the Docker volume `beachead-memory-{persona_id}`.
/// - Releases the allocated port and deletes the DB record.
/// - Returns HTTP 204 on success.
///
/// Per requirement 8.9: port release and DB record deletion MUST happen even if
/// Docker operations fail (best-effort cleanup).
async fn remove_container(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(params): Query<DeleteContainerQuery>,
) -> Result<StatusCode, OrchestratorError> {
    // Handle unmanaged containers (not in DB — remove directly via Docker)
    if id.starts_with("unmanaged-") {
        let docker_id = id.strip_prefix("unmanaged-").unwrap_or(&id);

        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| OrchestratorError::DockerError(format!("Failed to connect to Docker: {}", e)))?;

        // Stop (best-effort)
        let _ = docker
            .stop_container(docker_id, Some(StopContainerOptions { t: 10 }))
            .await;

        // Remove with force (best-effort)
        let _ = docker
            .remove_container(
                docker_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;

        return Ok(StatusCode::NO_CONTENT);
    }

    // 1. Look up the container in the DB by its primary key
    let db_row: DbContainerRow = state.db.with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT mc.id, mc.persona_id, COALESCE(p.name, '') as persona_name,
                        mc.container_id, mc.port, mc.volume_name, mc.status,
                        mc.created_at, mc.updated_at
                 FROM mcp_containers mc
                 LEFT JOIN personas p ON mc.persona_id = p.id
                 WHERE mc.id = ?1",
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;

        stmt.query_row(rusqlite::params![id], |row| {
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
        .optional()
        .map_err(|e| OrchestratorError::Database(e.to_string()))
    })?
    .ok_or_else(|| OrchestratorError::NotFound(format!("Container '{}' not found", id)))?;

    // 2. Connect to Docker (best-effort — if unavailable, skip Docker operations)
    let docker = Docker::connect_with_local_defaults().ok();

    // 3. Docker operations (best-effort — errors are logged but do not prevent cleanup)
    if let Some(ref docker) = docker {
        if let Some(ref docker_id) = db_row.container_id {
            if !docker_id.is_empty() {
                // 3a. Stop container (best-effort, ignore errors)
                let _ = docker
                    .stop_container(docker_id, Some(StopContainerOptions { t: 10 }))
                    .await;

                // 3b. Remove container with force (best-effort, ignore errors)
                let _ = docker
                    .remove_container(
                        docker_id,
                        Some(RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }),
                    )
                    .await;
            }
        }

        // 3c. If delete_volume=true, delete the Docker volume
        if params.delete_volume {
            let volume_name = format!("beachead-memory-{}", db_row.persona_id);
            let _ = docker.remove_volume(&volume_name, None).await;
        }
    }

    // 4. Release the allocated port (MUST happen even if Docker ops failed)
    state.db.with_conn(|conn| {
        conn.execute(
            "DELETE FROM port_allocations WHERE port = ?1",
            rusqlite::params![db_row.port as i64],
        )
        .map_err(|e| OrchestratorError::Database(e.to_string()))?;
        Ok(())
    })?;

    // 5. Delete the DB record (MUST happen even if Docker ops failed)
    state.db.with_conn(|conn| {
        conn.execute(
            "DELETE FROM mcp_containers WHERE id = ?1",
            rusqlite::params![db_row.id],
        )
        .map_err(|e| OrchestratorError::Database(e.to_string()))?;
        Ok(())
    })?;

    // 6. Return HTTP 204 No Content
    Ok(StatusCode::NO_CONTENT)
}
