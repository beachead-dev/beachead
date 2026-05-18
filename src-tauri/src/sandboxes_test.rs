//! Unit tests for sandbox action endpoints (stop, start, remove).
//!
//! Tests use mock shell scripts to simulate `sbx` CLI behavior and
//! exercise the HTTP handlers via axum's Router with tower::ServiceExt.
//!
//! Requirements: 5.1–5.7

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::Router;
    use http_body_util::BodyExt;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tower::ServiceExt;

    use crate::agent_manager::AgentManager;
    use crate::db::Database;
    use crate::export_import_manager::ExportImportManager;
    use crate::kit_generator::KitGenerator;
    use crate::persona_manager::PersonaManager;
    use crate::pty_bridge::PtyBridge;
    use crate::routes::sandboxes::{router, SandboxActionResponse, SandboxStartResponse};
    use crate::sbx::SbxCli;
    use crate::server::AppState;

    /// Create a minimal AppState for testing with an optional SbxCli.
    fn test_app_state(db: Arc<Database>, sbx: Option<Arc<SbxCli>>) -> AppState {
        let persona_manager = Arc::new(PersonaManager::new(db.clone()));
        let agent_manager = Arc::new(AgentManager::new(db.clone(), sbx.clone()));
        let pty_bridge = Arc::new(PtyBridge::new());
        let kit_generator = Arc::new(KitGenerator::new(PathBuf::from("/tmp/test-kits")));
        let export_import_manager = Arc::new(ExportImportManager::new(db.clone()));

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
            sbx,
            pty_bridge,
            kit_generator,
            repo_sync_manager: None,
        }
    }

    /// Build the test router with the sandbox routes.
    fn test_router(state: AppState) -> Router {
        router().with_state(state)
    }

    /// Create a mock sbx script that responds based on the subcommand.
    /// The script is written to a temp file and made executable.
    fn create_mock_sbx(script_content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("sbx");
        std::fs::write(&script_path, script_content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }
        (dir, script_path)
    }

    /// Helper to extract JSON body from response.
    async fn body_json<T: serde::de::DeserializeOwned>(body: Body) -> T {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// Helper to extract raw body bytes.
    async fn body_bytes(body: Body) -> Vec<u8> {
        body.collect().await.unwrap().to_bytes().to_vec()
    }

    // ─── Test: sbx unavailable returns 503 ───────────────────────────────

    /// Validates: Requirement 5.4
    /// When sbx CLI is not available, stop returns 503.
    #[tokio::test]
    async fn test_stop_sandbox_sbx_unavailable() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let state = test_app_state(db, None);
        let app = test_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/api/sandboxes/test-id/stop")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body: serde_json::Value = body_json(resp.into_body()).await;
        assert_eq!(body["error"]["code"], "SBX_UNAVAILABLE");
    }

    /// Validates: Requirement 5.4
    /// When sbx CLI is not available, start returns 503.
    #[tokio::test]
    async fn test_start_sandbox_sbx_unavailable() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let state = test_app_state(db, None);
        let app = test_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/api/sandboxes/test-id/start")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body: serde_json::Value = body_json(resp.into_body()).await;
        assert_eq!(body["error"]["code"], "SBX_UNAVAILABLE");
    }

    /// Validates: Requirement 5.4
    /// When sbx CLI is not available, remove returns 503.
    #[tokio::test]
    async fn test_remove_sandbox_sbx_unavailable() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let state = test_app_state(db, None);
        let app = test_router(state);

        let req = Request::builder()
            .method("DELETE")
            .uri("/api/sandboxes/test-id")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);

        let body: serde_json::Value = body_json(resp.into_body()).await;
        assert_eq!(body["error"]["code"], "SBX_UNAVAILABLE");
    }

    // ─── Test: stop sandbox success ──────────────────────────────────────

    /// Validates: Requirement 5.1
    /// Stopping a running sandbox returns 200 with id and status.
    #[tokio::test]
    async fn test_stop_sandbox_success() {
        // Mock sbx that:
        // - `ls --json` returns a running sandbox first, then stopped after stop is called
        // - `stop sandbox-123` succeeds and creates a state file
        // We use a state file to track whether stop has been called.
        let dir = tempfile::tempdir().unwrap();
        let state_file = dir.path().join("stopped");
        let script_path = dir.path().join("sbx");

        let script = format!(
            r#"#!/bin/sh
STATE_FILE="{}"
if [ "$1" = "ls" ] && [ "$2" = "--json" ]; then
    if [ -f "$STATE_FILE" ]; then
        echo '[{{"name":"my-sandbox","id":"sandbox-123","status":"stopped"}}]'
    else
        echo '[{{"name":"my-sandbox","id":"sandbox-123","status":"running"}}]'
    fi
    exit 0
elif [ "$1" = "stop" ] && [ "$2" = "sandbox-123" ]; then
    touch "$STATE_FILE"
    exit 0
fi
echo "unknown command: $@" >&2
exit 1
"#,
            state_file.display()
        );

        std::fs::write(&script_path, script).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755))
                .unwrap();
        }

        let sbx = SbxCli::with_path(script_path);

        let db = Arc::new(Database::open_in_memory().unwrap());
        let state = test_app_state(db, Some(Arc::new(sbx)));
        let app = test_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/api/sandboxes/sandbox-123/stop")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body: SandboxActionResponse = body_json(resp.into_body()).await;
        assert_eq!(body.id, "sandbox-123");
        assert_eq!(body.status, "stopped");
    }

    // ─── Test: stop sandbox already stopped (idempotent) ─────────────────

    /// Validates: Requirement 5.7
    /// Stopping an already-stopped sandbox returns 200 with "stopped" status.
    #[tokio::test]
    async fn test_stop_sandbox_already_stopped() {
        // Mock sbx that returns a stopped sandbox
        let script = r#"#!/bin/sh
if [ "$1" = "ls" ] && [ "$2" = "--json" ]; then
    echo '[{"name":"my-sandbox","id":"sandbox-456","status":"stopped"}]'
    exit 0
fi
echo "unexpected command: $@" >&2
exit 1
"#;
        let (_dir, script_path) = create_mock_sbx(script);
        let sbx = SbxCli::with_path(script_path);

        let db = Arc::new(Database::open_in_memory().unwrap());
        let state = test_app_state(db, Some(Arc::new(sbx)));
        let app = test_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/api/sandboxes/sandbox-456/stop")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body: SandboxActionResponse = body_json(resp.into_body()).await;
        assert_eq!(body.id, "sandbox-456");
        assert_eq!(body.status, "stopped");
    }

    // ─── Test: stop sandbox not found ────────────────────────────────────

    /// Validates: Requirement 5.5
    /// Stopping a non-existent sandbox returns 404.
    #[tokio::test]
    async fn test_stop_sandbox_not_found() {
        // Mock sbx that returns an empty list (no sandboxes)
        let script = r#"#!/bin/sh
if [ "$1" = "ls" ] && [ "$2" = "--json" ]; then
    echo '[]'
    exit 0
fi
echo "unexpected command: $@" >&2
exit 1
"#;
        let (_dir, script_path) = create_mock_sbx(script);
        let sbx = SbxCli::with_path(script_path);

        let db = Arc::new(Database::open_in_memory().unwrap());
        let state = test_app_state(db, Some(Arc::new(sbx)));
        let app = test_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/api/sandboxes/nonexistent-id/stop")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let body: serde_json::Value = body_json(resp.into_body()).await;
        assert_eq!(body["error"]["code"], "NOT_FOUND");
        assert!(body["error"]["message"].as_str().unwrap().contains("nonexistent-id"));
    }

    // ─── Test: start sandbox success ─────────────────────────────────────

    /// Validates: Requirement 5.2
    /// Starting a sandbox returns 200 with the new sandbox ID.
    #[tokio::test]
    async fn test_start_sandbox_success() {
        // Mock sbx that:
        // - `run ...` outputs a new sandbox ID
        let script = r#"#!/bin/sh
if [ "$1" = "run" ]; then
    echo "new-sandbox-789"
    exit 0
fi
echo "unexpected command: $@" >&2
exit 1
"#;
        let (_dir, script_path) = create_mock_sbx(script);
        let sbx = SbxCli::with_path(script_path);

        let db = Arc::new(Database::open_in_memory().unwrap());

        // Set up DB with required data: agent_type, persona, session
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('agent-1', 'claude', 'claude', 1, '{\"required_secrets\":[],\"auth_methods\":[],\"description\":\"\",\"supports_interactive_auth\":false}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| crate::error::OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, agent_cli_args, created_at, updated_at)
                 VALUES ('persona-1', 'test-persona', 'agent-1', '/tmp/workspace', 0, '[]', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| crate::error::OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO sessions (id, persona_id, sandbox_id, status, created_at, updated_at)
                 VALUES ('session-1', 'persona-1', 'old-sandbox-id', 'stopped', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| crate::error::OrchestratorError::Database(e.to_string()))?;

            Ok(())
        }).unwrap();

        let state = test_app_state(db, Some(Arc::new(sbx)));
        let app = test_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/api/sandboxes/old-sandbox-id/start")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body: SandboxStartResponse = body_json(resp.into_body()).await;
        assert_eq!(body.id, "new-sandbox-789");
    }

    // ─── Test: start sandbox not found ───────────────────────────────────

    /// Validates: Requirement 5.5
    /// Starting a sandbox that doesn't exist in sessions returns 404.
    #[tokio::test]
    async fn test_start_sandbox_not_found() {
        let script = r#"#!/bin/sh
echo "should not be called" >&2
exit 1
"#;
        let (_dir, script_path) = create_mock_sbx(script);
        let sbx = SbxCli::with_path(script_path);

        let db = Arc::new(Database::open_in_memory().unwrap());
        // No sessions in DB — sandbox ID won't be found
        let state = test_app_state(db, Some(Arc::new(sbx)));
        let app = test_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/api/sandboxes/nonexistent-id/start")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let body: serde_json::Value = body_json(resp.into_body()).await;
        assert_eq!(body["error"]["code"], "NOT_FOUND");
        assert!(body["error"]["message"].as_str().unwrap().contains("nonexistent-id"));
    }

    // ─── Test: remove sandbox success ────────────────────────────────────

    /// Validates: Requirement 5.3
    /// Removing a sandbox returns 204 with no body.
    #[tokio::test]
    async fn test_remove_sandbox_success() {
        let script = r#"#!/bin/sh
if [ "$1" = "rm" ] && [ "$2" = "--force" ] && [ "$3" = "sandbox-to-remove" ]; then
    exit 0
fi
echo "unexpected command: $@" >&2
exit 1
"#;
        let (_dir, script_path) = create_mock_sbx(script);
        let sbx = SbxCli::with_path(script_path);

        let db = Arc::new(Database::open_in_memory().unwrap());
        let state = test_app_state(db, Some(Arc::new(sbx)));
        let app = test_router(state);

        let req = Request::builder()
            .method("DELETE")
            .uri("/api/sandboxes/sandbox-to-remove")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);

        let bytes = body_bytes(resp.into_body()).await;
        assert!(bytes.is_empty());
    }

    // ─── Test: remove sandbox not found ──────────────────────────────────

    /// Validates: Requirement 5.5
    /// Removing a non-existent sandbox returns 404.
    #[tokio::test]
    async fn test_remove_sandbox_not_found() {
        let script = r#"#!/bin/sh
if [ "$1" = "rm" ]; then
    echo "No such sandbox: $3" >&2
    exit 1
fi
echo "unexpected command: $@" >&2
exit 1
"#;
        let (_dir, script_path) = create_mock_sbx(script);
        let sbx = SbxCli::with_path(script_path);

        let db = Arc::new(Database::open_in_memory().unwrap());
        let state = test_app_state(db, Some(Arc::new(sbx)));
        let app = test_router(state);

        let req = Request::builder()
            .method("DELETE")
            .uri("/api/sandboxes/nonexistent-id")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        let body: serde_json::Value = body_json(resp.into_body()).await;
        assert_eq!(body["error"]["code"], "NOT_FOUND");
    }

    // ─── Test: managed filtering (show_all=false) ────────────────────────

    /// Validates: Requirement 3.8, 3.10
    /// When show_all=false (default), only managed sandboxes are returned.
    #[tokio::test]
    async fn test_managed_filtering_show_all_false() {
        // Mock sbx returns 3 sandboxes
        let script = r#"#!/bin/sh
if [ "$1" = "ls" ] && [ "$2" = "--json" ]; then
    echo '[{"name":"managed-1","id":"sbx-aaa","status":"running"},{"name":"unmanaged","id":"sbx-bbb","status":"running"},{"name":"managed-2","id":"sbx-ccc","status":"stopped"}]'
    exit 0
fi
echo "unexpected command: $@" >&2
exit 1
"#;
        let (_dir, script_path) = create_mock_sbx(script);
        let sbx = SbxCli::with_path(script_path);

        let db = Arc::new(Database::open_in_memory().unwrap());

        // Insert sessions for sbx-aaa and sbx-ccc (making them "managed")
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('agent-1', 'claude', 'claude', 1, '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| crate::error::OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('persona-1', 'test-persona', 'agent-1', '/tmp/ws', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| crate::error::OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO sessions (id, persona_id, sandbox_id, status, created_at, updated_at)
                 VALUES ('s1', 'persona-1', 'sbx-aaa', 'running', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| crate::error::OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO sessions (id, persona_id, sandbox_id, status, created_at, updated_at)
                 VALUES ('s2', 'persona-1', 'sbx-ccc', 'stopped', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| crate::error::OrchestratorError::Database(e.to_string()))?;

            Ok(())
        }).unwrap();

        let state = test_app_state(db, Some(Arc::new(sbx)));
        let app = test_router(state);

        // Default (show_all not specified) should only return managed sandboxes
        let req = Request::builder()
            .method("GET")
            .uri("/api/sandboxes")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body: Vec<serde_json::Value> = body_json(resp.into_body()).await;
        assert_eq!(body.len(), 2);

        let ids: Vec<&str> = body.iter().map(|s| s["id"].as_str().unwrap()).collect();
        assert!(ids.contains(&"sbx-aaa"));
        assert!(ids.contains(&"sbx-ccc"));
        assert!(!ids.contains(&"sbx-bbb"));

        // All returned sandboxes should have managed=true
        for sandbox in &body {
            assert_eq!(sandbox["managed"], true);
        }
    }

    // ─── Test: managed filtering (show_all=true) ─────────────────────────

    /// Validates: Requirement 3.9
    /// When show_all=true, all sandboxes are returned with managed flag set.
    #[tokio::test]
    async fn test_managed_filtering_show_all_true() {
        // Mock sbx returns 3 sandboxes
        let script = r#"#!/bin/sh
if [ "$1" = "ls" ] && [ "$2" = "--json" ]; then
    echo '[{"name":"managed-1","id":"sbx-aaa","status":"running"},{"name":"unmanaged","id":"sbx-bbb","status":"running"},{"name":"managed-2","id":"sbx-ccc","status":"stopped"}]'
    exit 0
fi
echo "unexpected command: $@" >&2
exit 1
"#;
        let (_dir, script_path) = create_mock_sbx(script);
        let sbx = SbxCli::with_path(script_path);

        let db = Arc::new(Database::open_in_memory().unwrap());

        // Insert sessions for sbx-aaa and sbx-ccc (making them "managed")
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('agent-1', 'claude', 'claude', 1, '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| crate::error::OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('persona-1', 'test-persona', 'agent-1', '/tmp/ws', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| crate::error::OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO sessions (id, persona_id, sandbox_id, status, created_at, updated_at)
                 VALUES ('s1', 'persona-1', 'sbx-aaa', 'running', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| crate::error::OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO sessions (id, persona_id, sandbox_id, status, created_at, updated_at)
                 VALUES ('s2', 'persona-1', 'sbx-ccc', 'stopped', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| crate::error::OrchestratorError::Database(e.to_string()))?;

            Ok(())
        }).unwrap();

        let state = test_app_state(db, Some(Arc::new(sbx)));
        let app = test_router(state);

        let req = Request::builder()
            .method("GET")
            .uri("/api/sandboxes?show_all=true")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body: Vec<serde_json::Value> = body_json(resp.into_body()).await;
        assert_eq!(body.len(), 3);

        // Check managed flags
        let managed_sandbox = body.iter().find(|s| s["id"] == "sbx-aaa").unwrap();
        assert_eq!(managed_sandbox["managed"], true);

        let unmanaged_sandbox = body.iter().find(|s| s["id"] == "sbx-bbb").unwrap();
        assert_eq!(unmanaged_sandbox["managed"], false);

        let managed_sandbox_2 = body.iter().find(|s| s["id"] == "sbx-ccc").unwrap();
        assert_eq!(managed_sandbox_2["managed"], true);
    }

    // ─── Test: timeout enforcement ───────────────────────────────────────

    /// Validates: Requirement 5.6
    /// When sbx command takes longer than 30 seconds, returns 504.
    /// Note: We use a shorter sleep to avoid actually waiting 30s in tests.
    /// The timeout is enforced by tokio::time::timeout in the handler.
    /// This test uses a script that sleeps for 60s to trigger the timeout.
    #[tokio::test]
    async fn test_stop_sandbox_timeout() {
        // Mock sbx that hangs on ls (simulating timeout)
        // We can't actually wait 30s in a test, so we test the error path
        // by having the script sleep longer than the timeout.
        // However, this would make the test take 30s. Instead, we verify
        // the timeout error response format by testing with a script that
        // returns a timeout-like error.
        //
        // For a true timeout test, we'd need to reduce the timeout constant.
        // Instead, we verify the error response format matches expectations.
        let script = r#"#!/bin/sh
if [ "$1" = "ls" ] && [ "$2" = "--json" ]; then
    echo '[{"name":"my-sandbox","id":"sandbox-timeout","status":"running"}]'
    exit 0
elif [ "$1" = "stop" ]; then
    # Sleep longer than the 30s timeout
    sleep 60
    exit 0
fi
exit 1
"#;
        let (_dir, script_path) = create_mock_sbx(script);
        let sbx = SbxCli::with_path(script_path);

        let db = Arc::new(Database::open_in_memory().unwrap());
        let state = test_app_state(db, Some(Arc::new(sbx)));
        let app = test_router(state);

        let req = Request::builder()
            .method("POST")
            .uri("/api/sandboxes/sandbox-timeout/stop")
            .body(Body::empty())
            .unwrap();

        // Use tokio::time::timeout to avoid the test itself hanging
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(35),
            app.oneshot(req),
        )
        .await;

        let resp = result.expect("Test itself timed out").unwrap();
        assert_eq!(resp.status(), StatusCode::GATEWAY_TIMEOUT);

        let body: serde_json::Value = body_json(resp.into_body()).await;
        assert_eq!(body["error"]["code"], "SBX_TIMEOUT");
    }
}
