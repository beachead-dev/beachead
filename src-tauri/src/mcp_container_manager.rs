//! MCP Container Manager — manages Docker containers for per-persona memory MCP servers.
//!
//! Uses bollard to interact with the Docker Engine API for container lifecycle
//! management. Each persona gets a dedicated container running the
//! `ghcr.io/beachead-dev/beachead-memory-mcp:latest` image with its own volume and bearer token.

use std::collections::HashMap;
use std::sync::Arc;

use bollard::container::{
    Config, CreateContainerOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions,
};
use bollard::image::CreateImageOptions;
use bollard::models::{HostConfig, PortBinding};
use bollard::Docker;
use chrono::Utc;
use futures_util::TryStreamExt;
use rusqlite::params;
use rusqlite::OptionalExtension;

use crate::db::Database;
use crate::error::OrchestratorError;
use crate::port_allocator::PortAllocator;
use crate::token;
use crate::types::{McpContainerId, PersonaId};

/// Container port inside the MCP server image.
const CONTAINER_INTERNAL_PORT: u16 = 9100;

/// Docker image used for memory MCP containers.
const MCP_IMAGE: &str = "ghcr.io/beachead-dev/beachead-memory-mcp:latest";

/// Maximum restart attempts during health checks.
const MAX_HEALTH_RETRIES: u32 = 3;

/// Status values for MCP containers in the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerStatus {
    Created,
    Running,
    Stopped,
    Failed,
}

impl ContainerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
        }
    }
}

impl std::str::FromStr for ContainerStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "created" => Ok(Self::Created),
            "running" => Ok(Self::Running),
            "stopped" => Ok(Self::Stopped),
            "failed" => Ok(Self::Failed),
            other => Err(format!("unknown container status: '{}'", other)),
        }
    }
}

/// Represents an MCP container record from the database.
#[derive(Debug, Clone)]
pub struct McpContainer {
    pub id: McpContainerId,
    pub persona_id: PersonaId,
    pub shared_memory_id: Option<String>,
    pub container_id: Option<String>,
    pub port: u16,
    pub bearer_token: String,
    pub volume_name: String,
    pub status: ContainerStatus,
    pub created_at: String,
    pub updated_at: String,
}

/// Health status returned by health_check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Unhealthy,
    ContainerNotRunning,
}

/// Result of a health check for a single container.
#[derive(Debug, Clone)]
pub struct HealthCheckResult {
    pub mcp_container_id: McpContainerId,
    pub persona_id: PersonaId,
    pub status: HealthStatus,
    pub restarted: bool,
}

/// Manages Docker containers for MCP memory servers.
pub struct McpContainerManager {
    docker: Docker,
    db: Arc<Database>,
    port_allocator: Arc<PortAllocator>,
}

impl McpContainerManager {
    /// Create a new McpContainerManager using the local Docker socket.
    pub fn new(db: Arc<Database>, port_allocator: Arc<PortAllocator>) -> Result<Self, OrchestratorError> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| OrchestratorError::DockerError(format!("Failed to connect to Docker: {}", e)))?;

        Ok(Self {
            docker,
            db,
            port_allocator,
        })
    }

    /// Ensure the MCP Docker image is present locally, pulling from the registry if needed.
    pub async fn ensure_image_available(&self) -> Result<(), OrchestratorError> {
        match self.docker.inspect_image(MCP_IMAGE).await {
            Ok(_) => return Ok(()),
            Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => {}
            Err(e) => {
                return Err(OrchestratorError::DockerError(
                    format!("Failed to inspect image '{}': {}", MCP_IMAGE, e),
                ));
            }
        }

        self.docker
            .create_image(
                Some(CreateImageOptions {
                    from_image: MCP_IMAGE,
                    ..Default::default()
                }),
                None,
                None,
            )
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| OrchestratorError::DockerError(
                format!("Failed to pull MCP image '{}': {}", MCP_IMAGE, e),
            ))?;

        Ok(())
    }

    /// Ensure the MCP container for a persona is running.
    ///
    /// Handles all cases:
    /// - Container exists and is running → returns its config (no-op)
    /// - Container exists but is stopped/failed → restarts it
    /// - No container exists → creates and starts one
    ///
    /// Called by SessionManager at session start time.
    pub async fn ensure_container_running(&self, persona_id: &PersonaId) -> Result<McpContainer, OrchestratorError> {
        let existing = self.find_by_persona_id(persona_id)?;

        match existing {
            Some(container) if container.status == ContainerStatus::Running => {
                // Already running — verify with Docker that it's actually alive
                if let Some(ref docker_id) = container.container_id {
                    match self.docker.inspect_container(docker_id, None).await {
                        Ok(info) => {
                            let is_running = info
                                .state
                                .as_ref()
                                .and_then(|s| s.running)
                                .unwrap_or(false);
                            if is_running {
                                return Ok(container);
                            }
                            // Docker says it's not running — restart it
                        }
                        Err(_) => {
                            // Can't inspect — container may have been removed externally
                            // Fall through to recreate
                            return self.recreate_container(container).await;
                        }
                    }
                } else {
                    // No docker_id recorded — need to recreate
                    return self.recreate_container(container).await;
                }

                // Container exists in DB as "running" but Docker says otherwise — restart
                self.restart_existing_container(container).await
            }
            Some(container) if container.status == ContainerStatus::Stopped
                || container.status == ContainerStatus::Created => {
                // Stopped or just created — start it
                self.restart_existing_container(container).await
            }
            Some(container) if container.status == ContainerStatus::Failed => {
                // Failed — try to restart, recreate if that fails
                match self.restart_existing_container(container.clone()).await {
                    Ok(c) => Ok(c),
                    Err(_) => self.recreate_container(container).await,
                }
            }
            Some(container) => {
                // Unknown status — try restart
                self.restart_existing_container(container).await
            }
            None => {
                // No container exists — create one
                self.create_container(persona_id.clone()).await
            }
        }
    }

    /// Restart an existing container that has a Docker container ID.
    async fn restart_existing_container(&self, container: McpContainer) -> Result<McpContainer, OrchestratorError> {
        if let Some(ref docker_id) = container.container_id {
            // Try to start it (works for stopped containers)
            match self.docker.start_container(docker_id, None::<StartContainerOptions<String>>).await {
                Ok(_) => {
                    self.update_status(&container.id, ContainerStatus::Running)?;
                    let mut updated = container;
                    updated.status = ContainerStatus::Running;
                    return Ok(updated);
                }
                Err(e) => {
                    eprintln!(
                        "Failed to start existing container {} for persona {}: {}",
                        docker_id, container.persona_id.0, e
                    );
                    // Fall through to recreate
                }
            }
        }

        self.recreate_container(container).await
    }

    /// Remove an existing container record and create a fresh one.
    async fn recreate_container(&self, container: McpContainer) -> Result<McpContainer, OrchestratorError> {
        let persona_id = container.persona_id.clone();

        // Clean up the old container from Docker (best-effort)
        if let Some(ref docker_id) = container.container_id {
            let _ = self.docker.stop_container(docker_id, Some(StopContainerOptions { t: 5 })).await;
            let _ = self.docker.remove_container(docker_id, Some(RemoveContainerOptions {
                force: true,
                ..Default::default()
            })).await;
        }

        // Release the old port
        self.port_allocator.release(container.port)?;

        // Delete the old DB record
        self.db.with_conn(|conn| {
            conn.execute(
                "DELETE FROM mcp_containers WHERE id = ?1",
                params![container.id.0],
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })?;

        // Create a fresh container
        self.create_container(persona_id).await
    }

    /// Create a new MCP container for the given persona.
    ///
    /// Allocates a port, generates a bearer token, creates the Docker container
    /// with the appropriate volume mount and port binding, and persists the
    /// record in the `mcp_containers` table.
    pub async fn create_container(&self, persona_id: PersonaId) -> Result<McpContainer, OrchestratorError> {
        let mcp_id = McpContainerId::new();
        let bearer_token = token::generate_bearer_token();
        let volume_name = format!("beachead-memory-{}", persona_id.0);
        let now = Utc::now().to_rfc3339();

        // Insert the mcp_containers record first (port_allocations has a FK to it)
        // Use port 0 as placeholder until allocation succeeds.
        self.db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO mcp_containers (id, persona_id, port, bearer_token, volume_name, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    mcp_id.0,
                    persona_id.0,
                    0i64,
                    bearer_token,
                    volume_name,
                    ContainerStatus::Created.as_str(),
                    now,
                    now,
                ],
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })?;

        // Allocate a port (inserts into port_allocations which references mcp_containers)
        let port = match self.port_allocator.allocate(&mcp_id.0) {
            Ok(p) => p,
            Err(e) => {
                // Clean up the mcp_containers row on failure
                let _ = self.db.with_conn(|conn| {
                    conn.execute("DELETE FROM mcp_containers WHERE id = ?1", params![mcp_id.0])
                        .map_err(|e| OrchestratorError::Database(e.to_string()))?;
                    Ok(())
                });
                return Err(e);
            }
        };

        // Update the record with the actual allocated port
        self.db.with_conn(|conn| {
            conn.execute(
                "UPDATE mcp_containers SET port = ?1, updated_at = ?2 WHERE id = ?3",
                params![port as i64, Utc::now().to_rfc3339(), mcp_id.0],
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })?;

        // Create the Docker container
        let container_name = format!("beachead-mcp-{}", persona_id.0);
        let port_str = format!("{}/tcp", CONTAINER_INTERNAL_PORT);

        let mut port_bindings = HashMap::new();
        port_bindings.insert(
            port_str.clone(),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                host_port: Some(port.to_string()),
            }]),
        );

        let host_config = HostConfig {
            port_bindings: Some(port_bindings),
            binds: Some(vec![format!("{}:/data/memory", volume_name)]),
            ..Default::default()
        };

        let mut exposed_ports = HashMap::new();
        exposed_ports.insert(port_str, HashMap::new());

        let config = Config {
            image: Some(MCP_IMAGE.to_string()),
            env: Some(vec![
                format!("BEACHEAD_PORT={}", CONTAINER_INTERNAL_PORT),
                format!("BEACHEAD_BEARER_TOKEN={}", bearer_token),
                "BEACHEAD_DATA_DIR=/data/memory".to_string(),
            ]),
            host_config: Some(host_config),
            exposed_ports: Some(exposed_ports),
            ..Default::default()
        };

        let options = CreateContainerOptions {
            name: container_name,
            platform: None,
        };

        let response = self
            .docker
            .create_container(Some(options), config)
            .await
            .map_err(|e| OrchestratorError::DockerError(format!("Failed to create container: {}", e)))?;

        let docker_container_id = response.id;

        // Update the record with the Docker container ID
        self.db.with_conn(|conn| {
            conn.execute(
                "UPDATE mcp_containers SET container_id = ?1, updated_at = ?2 WHERE id = ?3",
                params![docker_container_id, Utc::now().to_rfc3339(), mcp_id.0],
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })?;

        // Start the container
        self.docker
            .start_container(&docker_container_id, None::<StartContainerOptions<String>>)
            .await
            .map_err(|e| OrchestratorError::DockerError(format!("Failed to start container: {}", e)))?;

        // Update status to running
        self.update_status(&mcp_id, ContainerStatus::Running)?;

        Ok(McpContainer {
            id: mcp_id,
            persona_id,
            shared_memory_id: None,
            container_id: Some(docker_container_id),
            port,
            bearer_token,
            volume_name,
            status: ContainerStatus::Running,
            created_at: now.clone(),
            updated_at: now,
        })
    }

    /// Start all MCP containers that are in the database.
    /// Called on orchestrator startup.
    pub async fn start_all(&self) -> Result<(), OrchestratorError> {
        let containers = self.list_containers()?;

        for container in containers {
            if let Some(ref docker_id) = container.container_id {
                match self
                    .docker
                    .start_container(docker_id, None::<StartContainerOptions<String>>)
                    .await
                {
                    Ok(_) => {
                        self.update_status(&container.id, ContainerStatus::Running)?;
                    }
                    Err(e) => {
                        eprintln!(
                            "Failed to start MCP container {} (persona {}): {}",
                            container.id.0, container.persona_id.0, e
                        );
                        self.update_status(&container.id, ContainerStatus::Failed)?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Reconcile containers: create missing containers for personas with memory enabled.
    ///
    /// Handles the case where a persona has memory_enabled=true but no container
    /// exists (e.g., due to a previous creation failure).
    /// Called on orchestrator startup after start_all.
    pub async fn reconcile(&self) -> Result<(), OrchestratorError> {
        // Find personas with memory_enabled that have no mcp_container
        let missing: Vec<PersonaId> = self.db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT p.id FROM personas p
                     WHERE p.memory_enabled = 1
                     AND NOT EXISTS (SELECT 1 FROM mcp_containers mc WHERE mc.persona_id = p.id)",
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))?;

            let ids = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| OrchestratorError::Database(e.to_string()))?
                .filter_map(|r| r.ok())
                .map(PersonaId)
                .collect();

            Ok(ids)
        })?;

        for persona_id in missing {
            eprintln!(
                "Reconciling: creating missing MCP container for persona {}",
                persona_id.0
            );
            if let Err(e) = self.create_container(persona_id.clone()).await {
                eprintln!(
                    "Failed to reconcile MCP container for persona {}: {}",
                    persona_id.0, e
                );
            }
        }

        Ok(())
    }

    /// Stop all running MCP containers.
    /// Called on orchestrator shutdown.
    pub async fn stop_all(&self) -> Result<(), OrchestratorError> {
        let containers = self.list_containers()?;

        for container in containers {
            if container.status == ContainerStatus::Running {
                if let Some(ref docker_id) = container.container_id {
                    match self
                        .docker
                        .stop_container(docker_id, Some(StopContainerOptions { t: 10 }))
                        .await
                    {
                        Ok(_) => {
                            self.update_status(&container.id, ContainerStatus::Stopped)?;
                        }
                        Err(e) => {
                            eprintln!(
                                "Failed to stop MCP container {} (persona {}): {}",
                                container.id.0, container.persona_id.0, e
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Check the health of a specific container by hitting its /health endpoint.
    pub async fn health_check(&self, container_id: &str) -> Result<HealthStatus, OrchestratorError> {
        // Find the container record by docker container_id
        let container = self.find_by_docker_id(container_id)?;

        match container {
            Some(c) => {
                if c.status != ContainerStatus::Running {
                    return Ok(HealthStatus::ContainerNotRunning);
                }
                self.check_health_endpoint(c.port).await
            }
            None => Err(OrchestratorError::NotFound(format!(
                "Container {} not found",
                container_id
            ))),
        }
    }

    /// Run health checks on all running containers.
    /// Restarts unhealthy containers up to MAX_HEALTH_RETRIES times.
    pub async fn run_health_checks(&self) -> Result<Vec<HealthCheckResult>, OrchestratorError> {
        let containers = self.list_containers()?;
        let mut results = Vec::new();

        for container in containers {
            if container.status != ContainerStatus::Running {
                continue;
            }

            let health = self.check_health_endpoint(container.port).await?;

            if health == HealthStatus::Healthy {
                results.push(HealthCheckResult {
                    mcp_container_id: container.id,
                    persona_id: container.persona_id,
                    status: HealthStatus::Healthy,
                    restarted: false,
                });
                continue;
            }

            // Unhealthy — attempt restart with retries
            let mut restarted = false;
            if let Some(ref docker_id) = container.container_id {
                for attempt in 1..=MAX_HEALTH_RETRIES {
                    eprintln!(
                        "Health check failed for container {} (persona {}), restart attempt {}/{}",
                        container.id.0, container.persona_id.0, attempt, MAX_HEALTH_RETRIES
                    );

                    // Stop then start
                    let _ = self
                        .docker
                        .stop_container(docker_id, Some(StopContainerOptions { t: 5 }))
                        .await;

                    if let Err(e) = self
                        .docker
                        .start_container(docker_id, None::<StartContainerOptions<String>>)
                        .await
                    {
                        eprintln!("Restart attempt {} failed: {}", attempt, e);
                        continue;
                    }

                    // Brief pause to let the container initialize
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                    // Re-check health
                    if let Ok(HealthStatus::Healthy) = self.check_health_endpoint(container.port).await {
                        restarted = true;
                        self.update_status(&container.id, ContainerStatus::Running)?;
                        break;
                    }
                }

                if !restarted {
                    self.update_status(&container.id, ContainerStatus::Failed)?;
                }
            }

            results.push(HealthCheckResult {
                mcp_container_id: container.id,
                persona_id: container.persona_id,
                status: if restarted {
                    HealthStatus::Healthy
                } else {
                    HealthStatus::Unhealthy
                },
                restarted,
            });
        }

        Ok(results)
    }

    /// Remove a container for the given persona.
    /// Stops the container, removes it from Docker, releases the port,
    /// and deletes the database record.
    pub async fn remove_container(&self, persona_id: PersonaId) -> Result<(), OrchestratorError> {
        let container = self.find_by_persona_id(&persona_id)?;

        let container = match container {
            Some(c) => c,
            None => {
                return Err(OrchestratorError::NotFound(format!(
                    "No MCP container found for persona {}",
                    persona_id.0
                )));
            }
        };

        // Stop the container if running
        if let Some(ref docker_id) = container.container_id {
            let _ = self
                .docker
                .stop_container(docker_id, Some(StopContainerOptions { t: 10 }))
                .await;

            // Remove the container
            self.docker
                .remove_container(
                    docker_id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await
                .map_err(|e| {
                    OrchestratorError::DockerError(format!("Failed to remove container: {}", e))
                })?;
        }

        // Release the port
        self.port_allocator.release(container.port)?;

        // Delete the database record
        self.db.with_conn(|conn| {
            conn.execute(
                "DELETE FROM mcp_containers WHERE id = ?1",
                params![container.id.0],
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })?;

        Ok(())
    }

    // --- Private helpers ---

    /// Check the health endpoint of a container at the given port.
    async fn check_health_endpoint(&self, port: u16) -> Result<HealthStatus, OrchestratorError> {
        let url = format!("http://127.0.0.1:{}/health", port);

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| OrchestratorError::Internal(format!("HTTP client error: {}", e)))?;

        match client.get(&url).send().await {
            Ok(resp) if resp.status().is_success() => Ok(HealthStatus::Healthy),
            Ok(_) => Ok(HealthStatus::Unhealthy),
            Err(_) => Ok(HealthStatus::Unhealthy),
        }
    }

    /// Update the status of a container in the database.
    fn update_status(
        &self,
        mcp_id: &McpContainerId,
        status: ContainerStatus,
    ) -> Result<(), OrchestratorError> {
        let now = Utc::now().to_rfc3339();
        self.db.with_conn(|conn| {
            conn.execute(
                "UPDATE mcp_containers SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![status.as_str(), now, mcp_id.0],
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })
    }

    /// List all MCP containers from the database.
    fn list_containers(&self) -> Result<Vec<McpContainer>, OrchestratorError> {
        self.db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, persona_id, shared_memory_id, container_id, port, bearer_token, volume_name, status, created_at, updated_at
                     FROM mcp_containers",
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))?;

            let rows = stmt
                .query_map([], |row| {
                    Ok(McpContainer {
                        id: McpContainerId(row.get::<_, String>(0)?),
                        persona_id: PersonaId(row.get::<_, String>(1)?),
                        shared_memory_id: row.get::<_, Option<String>>(2)?,
                        container_id: row.get::<_, Option<String>>(3)?,
                        port: row.get::<_, i64>(4)? as u16,
                        bearer_token: row.get::<_, String>(5)?,
                        volume_name: row.get::<_, String>(6)?,
                        status: row.get::<_, String>(7)?.parse::<ContainerStatus>().unwrap_or(ContainerStatus::Failed),
                        created_at: row.get::<_, String>(8)?,
                        updated_at: row.get::<_, String>(9)?,
                    })
                })
                .map_err(|e| OrchestratorError::Database(e.to_string()))?;

            let mut containers = Vec::new();
            for row in rows {
                containers
                    .push(row.map_err(|e| OrchestratorError::Database(e.to_string()))?);
            }
            Ok(containers)
        })
    }

    /// Find a container by its Docker container ID.
    fn find_by_docker_id(&self, docker_id: &str) -> Result<Option<McpContainer>, OrchestratorError> {
        self.db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, persona_id, shared_memory_id, container_id, port, bearer_token, volume_name, status, created_at, updated_at
                     FROM mcp_containers WHERE container_id = ?1",
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))?;

            let result = stmt
                .query_row(params![docker_id], |row| {
                    Ok(McpContainer {
                        id: McpContainerId(row.get::<_, String>(0)?),
                        persona_id: PersonaId(row.get::<_, String>(1)?),
                        shared_memory_id: row.get::<_, Option<String>>(2)?,
                        container_id: row.get::<_, Option<String>>(3)?,
                        port: row.get::<_, i64>(4)? as u16,
                        bearer_token: row.get::<_, String>(5)?,
                        volume_name: row.get::<_, String>(6)?,
                        status: row.get::<_, String>(7)?.parse::<ContainerStatus>().unwrap_or(ContainerStatus::Failed),
                        created_at: row.get::<_, String>(8)?,
                        updated_at: row.get::<_, String>(9)?,
                    })
                })
                .optional()
                .map_err(|e| OrchestratorError::Database(e.to_string()))?;

            Ok(result)
        })
    }

    /// Find a running container for the given persona.
    ///
    /// Returns the container record if it exists and has status "running".
    /// Used by SessionManager to get connection details at session start.
    pub fn find_running_container(&self, persona_id: &PersonaId) -> Result<Option<McpContainer>, OrchestratorError> {
        let container = self.find_by_persona_id(persona_id)?;
        match container {
            Some(c) if c.status == ContainerStatus::Running => Ok(Some(c)),
            _ => Ok(None),
        }
    }

    /// Find a container by persona ID.
    fn find_by_persona_id(&self, persona_id: &PersonaId) -> Result<Option<McpContainer>, OrchestratorError> {
        self.db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, persona_id, shared_memory_id, container_id, port, bearer_token, volume_name, status, created_at, updated_at
                     FROM mcp_containers WHERE persona_id = ?1",
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))?;

            let result = stmt
                .query_row(params![persona_id.0], |row| {
                    Ok(McpContainer {
                        id: McpContainerId(row.get::<_, String>(0)?),
                        persona_id: PersonaId(row.get::<_, String>(1)?),
                        shared_memory_id: row.get::<_, Option<String>>(2)?,
                        container_id: row.get::<_, Option<String>>(3)?,
                        port: row.get::<_, i64>(4)? as u16,
                        bearer_token: row.get::<_, String>(5)?,
                        volume_name: row.get::<_, String>(6)?,
                        status: row.get::<_, String>(7)?.parse::<ContainerStatus>().unwrap_or(ContainerStatus::Failed),
                        created_at: row.get::<_, String>(8)?,
                        updated_at: row.get::<_, String>(9)?,
                    })
                })
                .optional()
                .map_err(|e| OrchestratorError::Database(e.to_string()))?;

            Ok(result)
        })
    }
}
