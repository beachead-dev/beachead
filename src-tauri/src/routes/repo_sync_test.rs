//! Unit tests for repo sync route handlers (task 8.6).
//!
//! Tests cover:
//! - Request validation (missing required fields → 422 from axum JSON rejection)
//! - Not found (non-existent repo ID → 404)
//! - Operation conflict detection (OperationGuard → 409)
//! - Credential validation (empty username/secret → 400)
//!
//! **Validates: Requirements 18.14, 18.16, 18.17**

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
    use crate::routes::repo_sync::{router, OperationGuard, OPERATION_LOCKS};
    use crate::server::AppState;

    /// Create a minimal AppState with an in-memory database for testing.
    fn test_app_state() -> AppState {
        let db = Arc::new(Database::open_in_memory().expect("Failed to open in-memory db"));
        let pty_bridge = Arc::new(crate::pty_bridge::PtyBridge::new());
        let kit_generator = Arc::new(crate::kit_generator::KitGenerator::new(
            std::path::PathBuf::from("/tmp/test-kits"),
        ));
        let persona_manager = Arc::new(crate::persona_manager::PersonaManager::new(db.clone()));
        let agent_manager = Arc::new(crate::agent_manager::AgentManager::new(db.clone(), None));
        let export_import_manager = Arc::new(
            crate::export_import_manager::ExportImportManager::new(db.clone()),
        );

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
            api_token: Arc::new("test-token".to_string()),
            frontend_dist: None,
        }
    }

    /// Build the test router with repo sync routes.
    fn test_router(state: &AppState) -> Router {
        router().with_state(state.clone())
    }

    /// Insert prerequisite data: an agent_type and a persona.
    fn seed_persona(db: &Database, persona_id: &str, persona_name: &str) {
        db.with_conn(|conn| {
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

    /// Insert a managed repo record into the database.
    fn seed_managed_repo(db: &Database, repo_id: &str, persona_id: &str) {
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO managed_repos (id, persona_id, workspace_path, mirror_path, remote_url, branch_strategy, attribution_mode, sync_mode, secret_scan_mode, check_interval_seconds, created_at, updated_at)
                 VALUES (?1, ?2, '/tmp/workspace/project', '/tmp/mirrors/project', 'https://github.com/test/repo.git', 'direct', 'keep_agent', 'remote', 'block', 300, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                params![repo_id, persona_id],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })
        .unwrap();
    }

    // -----------------------------------------------------------------------
    // Test: POST /api/repo-sync/repos with missing required fields → 422
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_enable_repo_missing_fields_returns_422() {
        let state = test_app_state();
        let app = test_router(&state);

        // Send empty JSON body — missing persona_id and workspace_path
        let req = Request::builder()
            .method("POST")
            .uri("/api/repo-sync/repos")
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn test_enable_repo_missing_workspace_path_returns_422() {
        let state = test_app_state();
        let app = test_router(&state);

        // Missing workspace_path field
        let body = serde_json::json!({
            "persona_id": "some-persona-id"
        });

        let req = Request::builder()
            .method("POST")
            .uri("/api/repo-sync/repos")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn test_enable_repo_invalid_json_returns_400() {
        let state = test_app_state();
        let app = test_router(&state);

        // Completely invalid JSON — axum returns 400 for malformed bodies
        let req = Request::builder()
            .method("POST")
            .uri("/api/repo-sync/repos")
            .header("content-type", "application/json")
            .body(Body::from("not json at all"))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    // -----------------------------------------------------------------------
    // Test: Non-existent repo ID → 404
    // The set_credentials handler does a direct DB lookup for the repo
    // before calling the keyring, so we can test 404 without needing
    // the full RepoSyncManager.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_set_credentials_nonexistent_repo_returns_404() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        // No managed repo seeded — repo "nonexistent-repo" doesn't exist

        let app = test_router(&state);

        let body = serde_json::json!({
            "username": "testuser",
            "secret": "testtoken",
            "credential_type": "token"
        });

        let req = Request::builder()
            .method("PUT")
            .uri("/api/repo-sync/repos/nonexistent-repo/credentials")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let resp_body = response.into_body().collect().await.unwrap().to_bytes();
        let error: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(error["error"]["code"], "NOT_FOUND");
    }

    // -----------------------------------------------------------------------
    // Test: Operation conflict detection (OperationGuard → 409)
    // The sync operation handlers acquire the OperationGuard BEFORE calling
    // require_repo_sync_manager(), so we can test 409 even with manager=None.
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_operation_guard_conflict_pull_from_agent() {
        // Manually insert a lock for a repo ID to simulate an in-progress operation
        OPERATION_LOCKS.insert("test-repo-locked".to_string(), ());

        let state = test_app_state();
        let app = test_router(&state);

        let req = Request::builder()
            .method("POST")
            .uri("/api/repo-sync/repos/test-repo-locked/pull-from-agent")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);

        let resp_body = response.into_body().collect().await.unwrap().to_bytes();
        let error: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(error["error"]["code"], "SYNC_IN_PROGRESS");
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("test-repo-locked"));

        // Clean up the lock
        OPERATION_LOCKS.remove("test-repo-locked");
    }

    #[tokio::test]
    async fn test_operation_guard_conflict_push_to_remote() {
        OPERATION_LOCKS.insert("repo-busy".to_string(), ());

        let state = test_app_state();
        let app = test_router(&state);

        let body = serde_json::json!({
            "commit_shas": ["abc123"],
            "squash": false
        });

        let req = Request::builder()
            .method("POST")
            .uri("/api/repo-sync/repos/repo-busy/push-to-remote")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);

        let resp_body = response.into_body().collect().await.unwrap().to_bytes();
        let error: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(error["error"]["code"], "SYNC_IN_PROGRESS");

        OPERATION_LOCKS.remove("repo-busy");
    }

    #[tokio::test]
    async fn test_operation_guard_conflict_fetch_from_remote() {
        OPERATION_LOCKS.insert("repo-fetching".to_string(), ());

        let state = test_app_state();
        let app = test_router(&state);

        let req = Request::builder()
            .method("POST")
            .uri("/api/repo-sync/repos/repo-fetching/fetch-from-remote")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);

        OPERATION_LOCKS.remove("repo-fetching");
    }

    #[tokio::test]
    async fn test_operation_guard_conflict_push_to_agent() {
        OPERATION_LOCKS.insert("repo-pushing".to_string(), ());

        let state = test_app_state();
        let app = test_router(&state);

        let req = Request::builder()
            .method("POST")
            .uri("/api/repo-sync/repos/repo-pushing/push-to-agent")
            .body(Body::empty())
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);

        OPERATION_LOCKS.remove("repo-pushing");
    }

    // -----------------------------------------------------------------------
    // Test: Credential validation — empty username rejected
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn test_set_credentials_empty_username_returns_400() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        seed_managed_repo(&state.db, "repo-1", "persona-1");

        let app = test_router(&state);

        let body = serde_json::json!({
            "username": "",
            "secret": "valid-token",
            "credential_type": "token"
        });

        let req = Request::builder()
            .method("PUT")
            .uri("/api/repo-sync/repos/repo-1/credentials")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let resp_body = response.into_body().collect().await.unwrap().to_bytes();
        let error: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(error["error"]["code"], "VALIDATION_ERROR");
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("username"));
    }

    #[tokio::test]
    async fn test_set_credentials_whitespace_only_username_returns_400() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        seed_managed_repo(&state.db, "repo-1", "persona-1");

        let app = test_router(&state);

        let body = serde_json::json!({
            "username": "   ",
            "secret": "valid-token",
            "credential_type": "token"
        });

        let req = Request::builder()
            .method("PUT")
            .uri("/api/repo-sync/repos/repo-1/credentials")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let resp_body = response.into_body().collect().await.unwrap().to_bytes();
        let error: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(error["error"]["code"], "VALIDATION_ERROR");
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("username"));
    }

    #[tokio::test]
    async fn test_set_credentials_empty_secret_returns_400() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        seed_managed_repo(&state.db, "repo-1", "persona-1");

        let app = test_router(&state);

        let body = serde_json::json!({
            "username": "valid-user",
            "secret": "",
            "credential_type": "token"
        });

        let req = Request::builder()
            .method("PUT")
            .uri("/api/repo-sync/repos/repo-1/credentials")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let resp_body = response.into_body().collect().await.unwrap().to_bytes();
        let error: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(error["error"]["code"], "VALIDATION_ERROR");
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("secret"));
    }

    #[tokio::test]
    async fn test_set_credentials_whitespace_only_secret_returns_400() {
        let state = test_app_state();
        seed_persona(&state.db, "persona-1", "Test Persona");
        seed_managed_repo(&state.db, "repo-1", "persona-1");

        let app = test_router(&state);

        let body = serde_json::json!({
            "username": "valid-user",
            "secret": "  \t  ",
            "credential_type": "token"
        });

        let req = Request::builder()
            .method("PUT")
            .uri("/api/repo-sync/repos/repo-1/credentials")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let resp_body = response.into_body().collect().await.unwrap().to_bytes();
        let error: serde_json::Value = serde_json::from_slice(&resp_body).unwrap();
        assert_eq!(error["error"]["code"], "VALIDATION_ERROR");
        assert!(error["error"]["message"]
            .as_str()
            .unwrap()
            .contains("secret"));
    }

    #[tokio::test]
    async fn test_set_credentials_missing_fields_returns_422() {
        let state = test_app_state();
        let app = test_router(&state);

        // Missing credential_type field
        let body = serde_json::json!({
            "username": "user",
            "secret": "pass"
        });

        let req = Request::builder()
            .method("PUT")
            .uri("/api/repo-sync/repos/some-repo/credentials")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();

        let response = app.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    // -----------------------------------------------------------------------
    // Test: OperationGuard unit tests (direct, no HTTP)
    // -----------------------------------------------------------------------

    #[test]
    fn test_operation_guard_acquire_and_release() {
        // Ensure clean state
        OPERATION_LOCKS.remove("unit-test-repo");

        // First acquire should succeed
        let guard = OperationGuard::try_acquire("unit-test-repo");
        assert!(guard.is_ok());

        // While held, second acquire should fail
        let guard2 = OperationGuard::try_acquire("unit-test-repo");
        assert!(guard2.is_err());

        // Drop the first guard
        drop(guard);

        // Now acquire should succeed again
        let guard3 = OperationGuard::try_acquire("unit-test-repo");
        assert!(guard3.is_ok());

        // Clean up
        drop(guard3);
    }

    #[test]
    fn test_operation_guard_different_repos_independent() {
        OPERATION_LOCKS.remove("repo-a");
        OPERATION_LOCKS.remove("repo-b");

        // Acquire lock for repo-a
        let guard_a = OperationGuard::try_acquire("repo-a");
        assert!(guard_a.is_ok());

        // Acquire lock for repo-b should succeed (different repo)
        let guard_b = OperationGuard::try_acquire("repo-b");
        assert!(guard_b.is_ok());

        // Clean up
        drop(guard_a);
        drop(guard_b);
    }
}
