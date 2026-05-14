//! Unit tests for MCP container action endpoints (task 2.5).
//!
//! Tests the database logic layer for container list, start, stop, and remove operations.
//! Since bollard's Docker client is not trait-based and is called directly inside handlers,
//! these tests verify:
//! - Persona name join in list queries
//! - Live status enrichment fallback (Docker unavailable → DB status with confirmed=false)
//! - show_all query parameter behavior
//! - Idempotent start/stop behavior (DB status checks)
//! - Best-effort cleanup on delete (port release + DB record deletion)
//! - Volume deletion flag handling
//! - 404 responses for non-existent containers
//!
//! **Validates: Requirements 8.1–8.9**

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use http_body_util::BodyExt;
    use rusqlite::params;
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::db::Database;
    use crate::error::OrchestratorError;
    use crate::routes::mcp_containers::{router, McpContainerListResponse};
    use crate::server::AppState;

    /// Create a minimal AppState with an in-memory database for testing.
    /// All optional managers are None since we only need the DB and routes.
    fn test_app_state() -> AppState {
        let db = Arc::new(Database::open_in_memory().expect("Failed to open in-memory db"));
        let pty_bridge = Arc::new(crate::pty_bridge::PtyBridge::new());
        let kit_generator = Arc::new(crate::kit_generator::KitGenerator::new(
            std::path::PathBuf::from("/tmp/test-kits"),
        ));
        let persona_manager = Arc::new(crate::persona_manager::PersonaManager::new(db.clone()));
        let agent_manager = Arc::new(crate::agent_manager::AgentManager::new(db.clone(), None));
        let export_import_manager =
            Arc::new(crate::export_import_manager::ExportImportManager::new(db.clone()));

        AppState {
            persona_manager,
            agent_manager,
            credential_manager: None,
            session_manager: None,
            policy_manager: None,
            template_manager: None,
            system_manager: None,
            export_import_manager,
            mcp_container_manager: None,
            db,
            sbx: None,
            pty_bridge,
            kit_generator,
            repo_sync_manager: None,
        }
    }

    /// Build the test router with the MCP containers routes.
    fn test_router(state: &AppState) -> Router {
        router().with_state(state.clone())
    }

    /// Insert prerequisite data: an agent_type and a persona.
    fn seed_persona(db: &Database, persona_id: &str, persona_name: &str) {
        db.with_conn(|conn| {
            // Ensure agent_type exists (ignore if already inserted)
            conn.execute(
                "INSERT OR IGNORE INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('at1', 'test-agent', 'test', 0, '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES (?1, ?2, 'at1', '/tmp/workspace', 1, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                params![persona_id, persona_name],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            Ok(())
        })
        .unwrap();
    }

    /// Insert an MCP container record into the database.
    fn seed_container(
        db: &Database,
        id: &str,
        persona_id: &str,
        container_id: Option<&str>,
        port: u16,
        status: &str,
    ) {
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO mcp_containers (id, persona_id, container_id, port, bearer_token, volume_name, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 'test-token', ?5, ?6, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                params![
                    id,
                    persona_id,
                    container_id,
                    port as i64,
                    format!("beachead-memory-{}", persona_id),
                    status,
                ],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            Ok(())
        })
        .unwrap();
    }

    /// Insert a port allocation record.
    fn seed_port_allocation(db: &Database, port: u16, mcp_container_id: &str) {
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO port_allocations (port, mcp_container_id, allocated_at)
                 VALUES (?1, ?2, '2024-01-01T00:00:00Z')",
                params![port as i64, mcp_container_id],
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })
        .unwrap();
    }

    // -----------------------------------------------------------------------
    // Test: list containers with persona name join
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_list_containers_with_persona_name() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Alice Agent");
        seed_persona(&state.db, "persona-2", "Bob Builder");
        seed_container(&state.db, "mc-1", "persona-1", None, 9100, "running");
        seed_container(&state.db, "mc-2", "persona-2", None, 9101, "stopped");

        let app = test_router(&state);
        let req = Request::builder()
            .uri("/api/mcp-containers")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let containers: Vec<McpContainerListResponse> =
            serde_json::from_slice(&body).unwrap();

        assert_eq!(containers.len(), 2);

        // Containers are ordered by created_at DESC, but both have same timestamp
        // so check both are present with correct persona names
        let names: Vec<&str> = containers.iter().map(|c| c.persona_name.as_str()).collect();
        assert!(names.contains(&"Alice Agent"));
        assert!(names.contains(&"Bob Builder"));

        // Verify persona_id is also present
        let alice = containers.iter().find(|c| c.persona_name == "Alice Agent").unwrap();
        assert_eq!(alice.persona_id, "persona-1");
        assert_eq!(alice.port, 9100);
    }

    // -----------------------------------------------------------------------
    // Test: list containers live status (Docker unavailable → fallback to DB)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_list_containers_live_status() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        // Container with a docker container_id — but Docker is unavailable in test
        seed_container(
            &state.db,
            "mc-1",
            "persona-1",
            Some("docker-abc123"),
            9100,
            "running",
        );

        let app = test_router(&state);
        let req = Request::builder()
            .uri("/api/mcp-containers")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let containers: Vec<McpContainerListResponse> =
            serde_json::from_slice(&body).unwrap();

        assert_eq!(containers.len(), 1);
        let c = &containers[0];

        // Docker is unavailable in test environment, so:
        // - status falls back to DB value
        // - live_status_confirmed is false
        assert_eq!(c.status, "running"); // DB status used as fallback
        assert!(!c.live_status_confirmed);
        assert_eq!(c.container_id, Some("docker-abc123".to_string()));
    }

    // -----------------------------------------------------------------------
    // Test: list containers with show_all (unmanaged container detection)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_list_containers_show_all() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Managed Persona");
        seed_container(&state.db, "mc-1", "persona-1", None, 9100, "running");

        let app = test_router(&state);

        // Without show_all — only DB-tracked containers
        let req = Request::builder()
            .uri("/api/mcp-containers")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let containers: Vec<McpContainerListResponse> =
            serde_json::from_slice(&body).unwrap();

        assert_eq!(containers.len(), 1);
        assert_eq!(containers[0].persona_name, "Managed Persona");

        // With show_all=true — includes unmanaged Docker containers (if Docker is available)
        let app2 = test_router(&state);
        let req2 = Request::builder()
            .uri("/api/mcp-containers?show_all=true")
            .body(Body::empty())
            .unwrap();

        let response2 = app2.oneshot(req2).await.unwrap();
        assert_eq!(response2.status(), StatusCode::OK);

        let body2 = response2.into_body().collect().await.unwrap().to_bytes();
        let containers2: Vec<McpContainerListResponse> =
            serde_json::from_slice(&body2).unwrap();

        // Must include at least the DB-tracked container
        assert!(
            containers2.len() >= 1,
            "show_all should return at least the DB-tracked container"
        );

        // The DB-tracked container must be present
        let managed = containers2
            .iter()
            .find(|c| c.id == "mc-1")
            .expect("DB-tracked container should be in the list");
        assert_eq!(managed.persona_name, "Managed Persona");

        // Any additional containers should have the "unmanaged-" prefix
        for c in &containers2 {
            if c.id != "mc-1" {
                assert!(
                    c.id.starts_with("unmanaged-"),
                    "Non-DB containers should have 'unmanaged-' prefix, got: {}",
                    c.id
                );
                assert!(c.live_status_confirmed);
            }
        }
    }

    // -----------------------------------------------------------------------
    // Test: start container success (Docker start fails for non-existent container → error)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_start_container_success() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        seed_container(
            &state.db,
            "mc-1",
            "persona-1",
            Some("docker-abc123"),
            9100,
            "stopped",
        );

        let app = test_router(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/api/mcp-containers/mc-1/start")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();

        // The handler looks up the DB record successfully, then attempts to start
        // the Docker container "docker-abc123" which doesn't exist in Docker.
        // This results in a Docker error (502 BAD_GATEWAY).
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    // -----------------------------------------------------------------------
    // Test: start container already running (idempotent — Docker can't confirm)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_start_container_already_running() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        // Container with status "running" but Docker container doesn't actually exist.
        // Handler will try to inspect via Docker, fail (container not found),
        // then try to start it, which also fails.
        seed_container(
            &state.db,
            "mc-1",
            "persona-1",
            Some("docker-abc123"),
            9100,
            "running",
        );

        let app = test_router(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/api/mcp-containers/mc-1/start")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();

        // Docker inspect fails (container doesn't exist) → handler proceeds to start
        // → start also fails → Docker error
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    // -----------------------------------------------------------------------
    // Test: start container not found (404)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_start_container_not_found() {
        let state = test_app_state();

        let app = test_router(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/api/mcp-containers/nonexistent/start")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["error"]["code"], "NOT_FOUND");
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("nonexistent"));
    }

    // -----------------------------------------------------------------------
    // Test: stop container success (Docker stop fails for non-existent container → error)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_stop_container_success() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        seed_container(
            &state.db,
            "mc-1",
            "persona-1",
            Some("docker-abc123"),
            9100,
            "running",
        );

        let app = test_router(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/api/mcp-containers/mc-1/stop")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();

        // Handler finds the DB record (status != "stopped"), connects to Docker,
        // tries to stop "docker-abc123" which doesn't exist → Docker error
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    }

    // -----------------------------------------------------------------------
    // Test: stop container already stopped (idempotent — returns 200)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_stop_container_already_stopped() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        seed_container(
            &state.db,
            "mc-1",
            "persona-1",
            Some("docker-abc123"),
            9100,
            "stopped",
        );

        let app = test_router(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/api/mcp-containers/mc-1/stop")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();

        // Already stopped → idempotent 200 response without calling Docker
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let container: McpContainerListResponse = serde_json::from_slice(&body).unwrap();

        assert_eq!(container.id, "mc-1");
        assert_eq!(container.status, "stopped");
        assert_eq!(container.persona_name, "Test Persona");
        assert!(!container.live_status_confirmed);
    }

    // -----------------------------------------------------------------------
    // Test: stop container not found (404)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_stop_container_not_found() {
        let state = test_app_state();

        let app = test_router(&state);
        let req = Request::builder()
            .method("POST")
            .uri("/api/mcp-containers/nonexistent/stop")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let error: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(error["error"]["code"], "NOT_FOUND");
    }

    // -----------------------------------------------------------------------
    // Test: remove container success (best-effort Docker + DB cleanup)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_remove_container_success() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        seed_container(
            &state.db,
            "mc-1",
            "persona-1",
            Some("docker-abc123"),
            9100,
            "running",
        );
        seed_port_allocation(&state.db, 9100, "mc-1");

        let app = test_router(&state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/api/mcp-containers/mc-1")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();

        // DELETE succeeds with 204 — Docker ops are best-effort (ignored if unavailable)
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // Verify DB record was deleted
        let count: i64 = state
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM mcp_containers WHERE id = 'mc-1'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))
            })
            .unwrap();
        assert_eq!(count, 0, "Container record should be deleted from DB");

        // Verify port allocation was released
        let port_count: i64 = state
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM port_allocations WHERE port = 9100",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))
            })
            .unwrap();
        assert_eq!(port_count, 0, "Port allocation should be released");
    }

    // -----------------------------------------------------------------------
    // Test: remove container with volume deletion flag
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_remove_container_with_volume() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        seed_container(
            &state.db,
            "mc-1",
            "persona-1",
            Some("docker-abc123"),
            9100,
            "stopped",
        );
        seed_port_allocation(&state.db, 9100, "mc-1");

        let app = test_router(&state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/api/mcp-containers/mc-1?delete_volume=true")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();

        // DELETE succeeds with 204 — volume deletion is best-effort via Docker
        // (Docker unavailable in test, so volume delete is silently skipped)
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // Verify DB cleanup happened regardless
        let count: i64 = state
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM mcp_containers WHERE id = 'mc-1'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))
            })
            .unwrap();
        assert_eq!(count, 0, "Container record should be deleted from DB");

        let port_count: i64 = state
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM port_allocations WHERE port = 9100",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))
            })
            .unwrap();
        assert_eq!(port_count, 0, "Port allocation should be released");
    }

    // -----------------------------------------------------------------------
    // Test: remove container — Docker failure still cleans DB (best-effort)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_remove_container_docker_failure_still_cleans_db() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        // Container with a Docker ID that doesn't exist — Docker ops will fail
        seed_container(
            &state.db,
            "mc-1",
            "persona-1",
            Some("nonexistent-docker-id"),
            9100,
            "running",
        );
        seed_port_allocation(&state.db, 9100, "mc-1");

        let app = test_router(&state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/api/mcp-containers/mc-1")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();

        // Per requirement 8.9: port release and DB record deletion MUST happen
        // even if Docker operations fail. Returns 204.
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // Verify DB record was deleted despite Docker failure
        let count: i64 = state
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM mcp_containers WHERE id = 'mc-1'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))
            })
            .unwrap();
        assert_eq!(count, 0, "Container record should be deleted even on Docker failure");

        // Verify port was released despite Docker failure
        let port_count: i64 = state
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM port_allocations WHERE port = 9100",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))
            })
            .unwrap();
        assert_eq!(port_count, 0, "Port should be released even on Docker failure");
    }

    // -----------------------------------------------------------------------
    // Test: remove container not found (404)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_remove_container_not_found() {
        let state = test_app_state();

        let app = test_router(&state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/api/mcp-containers/nonexistent")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // Test: list containers with no persona (LEFT JOIN returns empty name)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_list_containers_missing_persona() {
        let state = test_app_state();

        // Insert agent_type for FK chain
        state
            .db
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                     VALUES ('at1', 'test-agent', 'test', 0, '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                    [],
                ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
                Ok(())
            })
            .unwrap();

        // Insert container with a persona_id that doesn't exist in personas table
        // (the FK on mcp_containers.persona_id is nullable/soft — it uses REFERENCES but
        // the persona might have been deleted)
        // Actually, let's insert a persona first then delete it to test the LEFT JOIN
        seed_persona(&state.db, "persona-orphan", "Orphan Persona");
        seed_container(&state.db, "mc-orphan", "persona-orphan", None, 9100, "stopped");

        // Delete the persona (mcp_containers FK doesn't cascade)
        state
            .db
            .with_conn(|conn| {
                // Disable FK temporarily to allow orphan
                conn.execute_batch("PRAGMA foreign_keys=OFF;")
                    .map_err(|e| OrchestratorError::Database(e.to_string()))?;
                conn.execute("DELETE FROM personas WHERE id = 'persona-orphan'", [])
                    .map_err(|e| OrchestratorError::Database(e.to_string()))?;
                conn.execute_batch("PRAGMA foreign_keys=ON;")
                    .map_err(|e| OrchestratorError::Database(e.to_string()))?;
                Ok(())
            })
            .unwrap();

        let app = test_router(&state);
        let req = Request::builder()
            .uri("/api/mcp-containers")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let containers: Vec<McpContainerListResponse> =
            serde_json::from_slice(&body).unwrap();

        assert_eq!(containers.len(), 1);
        // COALESCE(p.name, '') returns empty string when persona is missing
        assert_eq!(containers[0].persona_name, "");
    }

    // -----------------------------------------------------------------------
    // Test: remove container without port allocation (still succeeds)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_remove_container_no_port_allocation() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        seed_container(&state.db, "mc-1", "persona-1", None, 9100, "stopped");
        // No port_allocation record — DELETE should still succeed

        let app = test_router(&state);
        let req = Request::builder()
            .method("DELETE")
            .uri("/api/mcp-containers/mc-1")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        // DB record should be gone
        let count: i64 = state
            .db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM mcp_containers WHERE id = 'mc-1'",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))
            })
            .unwrap();
        assert_eq!(count, 0);
    }
}
