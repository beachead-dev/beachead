//! Unit tests for db_ops repo sync CRUD operations.
//! Tests insert, get, list, update, delete for managed_repos and repo_credentials.
//! Also tests unique constraint violations and cascade deletes.

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use crate::db::Database;
    use crate::db_ops::*;
    use crate::types::*;

    /// Helper: create an in-memory database and seed an agent type + persona.
    /// Returns (db, persona_id) for use in tests.
    fn setup_db_with_persona() -> (Database, PersonaId) {
        let db = Database::open_in_memory().expect("Failed to open in-memory database");
        let now = Utc::now();

        let agent = AgentType {
            id: AgentTypeId::new(),
            name: "test-agent".to_string(),
            sbx_agent: Some("claude".to_string()),
            kit_ref: None,
            is_builtin: true,
            metadata: AgentMetadata {
                required_secrets: vec![],
                auth_methods: vec![],
                description: "Test agent".to_string(),
                supports_interactive_auth: false,
                mcp_config_path: None,
            },
            created_at: now,
            updated_at: now,
        };

        db.with_conn(|conn| insert_agent_type(conn, &agent))
            .unwrap();

        let persona_id = PersonaId::new();
        let persona = Persona {
            id: persona_id.clone(),
            name: "test-persona".to_string(),
            agent_type_id: agent.id.clone(),
            workspace_path: std::path::PathBuf::from("/tmp/workspace"),
            memory_enabled: false,
            agent_cli_args: vec![],
            mcp_servers: vec![],
            additional_workspaces: vec![],
            created_at: now,
            updated_at: now,
        };

        db.with_conn(|conn| insert_persona(conn, &persona)).unwrap();

        (db, persona_id)
    }

    /// Helper: create a ManagedRepo with sensible defaults.
    fn make_managed_repo(persona_id: &PersonaId, workspace: &str, mirror: &str) -> ManagedRepo {
        let now = Utc::now();
        ManagedRepo {
            id: ManagedRepoId::new(),
            persona_id: persona_id.clone(),
            workspace_path: workspace.to_string(),
            mirror_path: mirror.to_string(),
            remote_url: Some("https://github.com/user/repo.git".to_string()),
            remote_provider: Some(RemoteProvider::Github),
            branch_strategy: BranchStrategy::Direct,
            branch_pattern: Some("ai/<persona-name>/<date>".to_string()),
            attribution_mode: AttributionMode::KeepAgent,
            sync_mode: SyncMode::Remote,
            secret_scan_mode: SecretScanMode::Block,
            check_interval_seconds: 300,
            created_at: now,
            updated_at: now,
        }
    }

    // --- managed_repos CRUD tests ---

    #[test]
    fn test_insert_and_get_managed_repo() {
        let (db, persona_id) = setup_db_with_persona();
        let repo = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/project");

        db.with_conn(|conn| insert_managed_repo(conn, &repo))
            .unwrap();

        let fetched = db
            .with_conn(|conn| get_managed_repo(conn, &repo.id))
            .unwrap();

        assert_eq!(fetched.id.0, repo.id.0);
        assert_eq!(fetched.persona_id.0, persona_id.0);
        assert_eq!(fetched.workspace_path, "/home/user/project");
        assert_eq!(fetched.mirror_path, "/mirrors/project");
        assert_eq!(
            fetched.remote_url,
            Some("https://github.com/user/repo.git".to_string())
        );
        assert_eq!(fetched.remote_provider, Some(RemoteProvider::Github));
        assert_eq!(fetched.branch_strategy, BranchStrategy::Direct);
        assert_eq!(
            fetched.branch_pattern,
            Some("ai/<persona-name>/<date>".to_string())
        );
        assert_eq!(fetched.attribution_mode, AttributionMode::KeepAgent);
        assert_eq!(fetched.sync_mode, SyncMode::Remote);
        assert_eq!(fetched.secret_scan_mode, SecretScanMode::Block);
        assert_eq!(fetched.check_interval_seconds, 300);
    }

    #[test]
    fn test_get_managed_repo_not_found() {
        let (db, _) = setup_db_with_persona();
        let fake_id = ManagedRepoId("nonexistent".to_string());

        let result = db.with_conn(|conn| get_managed_repo(conn, &fake_id));
        assert!(result.is_err());
    }

    #[test]
    fn test_list_managed_repos_empty() {
        let (db, _) = setup_db_with_persona();

        let repos = db.with_conn(|conn| list_managed_repos(conn)).unwrap();
        assert!(repos.is_empty());
    }

    #[test]
    fn test_list_managed_repos_returns_all() {
        let (db, persona_id) = setup_db_with_persona();

        let repo1 = make_managed_repo(&persona_id, "/home/user/alpha", "/mirrors/alpha");
        let repo2 = make_managed_repo(&persona_id, "/home/user/beta", "/mirrors/beta");

        db.with_conn(|conn| insert_managed_repo(conn, &repo1))
            .unwrap();
        db.with_conn(|conn| insert_managed_repo(conn, &repo2))
            .unwrap();

        let repos = db.with_conn(|conn| list_managed_repos(conn)).unwrap();
        assert_eq!(repos.len(), 2);
        // Ordered by workspace_path
        assert_eq!(repos[0].workspace_path, "/home/user/alpha");
        assert_eq!(repos[1].workspace_path, "/home/user/beta");
    }

    #[test]
    fn test_list_managed_repos_by_persona() {
        let (db, persona_id) = setup_db_with_persona();

        // Create a second persona
        let now = Utc::now();
        let persona2_id = PersonaId::new();
        let agent_types = db.with_conn(|conn| list_agent_types(conn)).unwrap();
        let persona2 = Persona {
            id: persona2_id.clone(),
            name: "other-persona".to_string(),
            agent_type_id: agent_types[0].id.clone(),
            workspace_path: std::path::PathBuf::from("/tmp/other"),
            memory_enabled: false,
            agent_cli_args: vec![],
            mcp_servers: vec![],
            additional_workspaces: vec![],
            created_at: now,
            updated_at: now,
        };
        db.with_conn(|conn| insert_persona(conn, &persona2))
            .unwrap();

        // Insert repos for both personas
        let repo1 = make_managed_repo(&persona_id, "/home/user/project1", "/mirrors/project1");
        let repo2 = make_managed_repo(&persona2_id, "/home/user/project2", "/mirrors/project2");

        db.with_conn(|conn| insert_managed_repo(conn, &repo1))
            .unwrap();
        db.with_conn(|conn| insert_managed_repo(conn, &repo2))
            .unwrap();

        // List by first persona
        let repos = db
            .with_conn(|conn| list_managed_repos_by_persona(conn, &persona_id))
            .unwrap();
        assert_eq!(repos.len(), 1);
        assert_eq!(repos[0].workspace_path, "/home/user/project1");

        // List by second persona
        let repos2 = db
            .with_conn(|conn| list_managed_repos_by_persona(conn, &persona2_id))
            .unwrap();
        assert_eq!(repos2.len(), 1);
        assert_eq!(repos2[0].workspace_path, "/home/user/project2");
    }

    #[test]
    fn test_update_managed_repo() {
        let (db, persona_id) = setup_db_with_persona();
        let repo = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/project");

        db.with_conn(|conn| insert_managed_repo(conn, &repo))
            .unwrap();

        // Modify fields
        let mut updated = repo.clone();
        updated.remote_url = Some("https://gitlab.com/user/repo.git".to_string());
        updated.remote_provider = Some(RemoteProvider::Gitlab);
        updated.branch_strategy = BranchStrategy::FeatureBranch;
        updated.branch_pattern = Some("feature/<persona>/<date>".to_string());
        updated.attribution_mode = AttributionMode::CoAuthoredBy;
        updated.sync_mode = SyncMode::LocalOnly;
        updated.secret_scan_mode = SecretScanMode::WarnOnly;
        updated.check_interval_seconds = 600;
        updated.updated_at = Utc::now();

        db.with_conn(|conn| update_managed_repo(conn, &repo.id, &updated))
            .unwrap();

        let fetched = db
            .with_conn(|conn| get_managed_repo(conn, &repo.id))
            .unwrap();
        assert_eq!(
            fetched.remote_url,
            Some("https://gitlab.com/user/repo.git".to_string())
        );
        assert_eq!(fetched.remote_provider, Some(RemoteProvider::Gitlab));
        assert_eq!(fetched.branch_strategy, BranchStrategy::FeatureBranch);
        assert_eq!(
            fetched.branch_pattern,
            Some("feature/<persona>/<date>".to_string())
        );
        assert_eq!(fetched.attribution_mode, AttributionMode::CoAuthoredBy);
        assert_eq!(fetched.sync_mode, SyncMode::LocalOnly);
        assert_eq!(fetched.secret_scan_mode, SecretScanMode::WarnOnly);
        assert_eq!(fetched.check_interval_seconds, 600);
    }

    #[test]
    fn test_update_managed_repo_not_found() {
        let (db, persona_id) = setup_db_with_persona();
        let repo = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/project");
        let fake_id = ManagedRepoId("nonexistent".to_string());

        let result = db.with_conn(|conn| update_managed_repo(conn, &fake_id, &repo));
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_managed_repo() {
        let (db, persona_id) = setup_db_with_persona();
        let repo = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/project");

        db.with_conn(|conn| insert_managed_repo(conn, &repo))
            .unwrap();

        // Delete
        db.with_conn(|conn| delete_managed_repo(conn, &repo.id))
            .unwrap();

        // Verify gone
        let result = db.with_conn(|conn| get_managed_repo(conn, &repo.id));
        assert!(result.is_err());
    }

    #[test]
    fn test_delete_managed_repo_not_found() {
        let (db, _) = setup_db_with_persona();
        let fake_id = ManagedRepoId("nonexistent".to_string());

        let result = db.with_conn(|conn| delete_managed_repo(conn, &fake_id));
        assert!(result.is_err());
    }

    #[test]
    fn test_managed_repo_exists() {
        let (db, persona_id) = setup_db_with_persona();
        let repo = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/project");

        // Before insert
        let exists = db
            .with_conn(|conn| managed_repo_exists(conn, &persona_id, "/home/user/project"))
            .unwrap();
        assert!(!exists);

        // After insert
        db.with_conn(|conn| insert_managed_repo(conn, &repo))
            .unwrap();
        let exists = db
            .with_conn(|conn| managed_repo_exists(conn, &persona_id, "/home/user/project"))
            .unwrap();
        assert!(exists);

        // Different path should not exist
        let exists = db
            .with_conn(|conn| managed_repo_exists(conn, &persona_id, "/home/user/other"))
            .unwrap();
        assert!(!exists);
    }

    // --- Unique constraint test ---

    #[test]
    fn test_insert_duplicate_persona_workspace_fails() {
        let (db, persona_id) = setup_db_with_persona();

        let repo1 = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/project1");
        db.with_conn(|conn| insert_managed_repo(conn, &repo1))
            .unwrap();

        // Same persona_id + workspace_path, different id and mirror
        let repo2 = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/project2");
        let result = db.with_conn(|conn| insert_managed_repo(conn, &repo2));
        assert!(
            result.is_err(),
            "UNIQUE(persona_id, workspace_path) should prevent duplicate"
        );
    }

    #[test]
    fn test_same_workspace_different_persona_succeeds() {
        let (db, persona_id) = setup_db_with_persona();

        // Create second persona
        let now = Utc::now();
        let persona2_id = PersonaId::new();
        let agent_types = db.with_conn(|conn| list_agent_types(conn)).unwrap();
        let persona2 = Persona {
            id: persona2_id.clone(),
            name: "persona-two".to_string(),
            agent_type_id: agent_types[0].id.clone(),
            workspace_path: std::path::PathBuf::from("/tmp/ws2"),
            memory_enabled: false,
            agent_cli_args: vec![],
            mcp_servers: vec![],
            additional_workspaces: vec![],
            created_at: now,
            updated_at: now,
        };
        db.with_conn(|conn| insert_persona(conn, &persona2))
            .unwrap();

        // Same workspace_path but different persona_id should succeed
        let repo1 = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/p1/project");
        let repo2 = make_managed_repo(&persona2_id, "/home/user/project", "/mirrors/p2/project");

        db.with_conn(|conn| insert_managed_repo(conn, &repo1))
            .unwrap();
        db.with_conn(|conn| insert_managed_repo(conn, &repo2))
            .unwrap();

        let all = db.with_conn(|conn| list_managed_repos(conn)).unwrap();
        assert_eq!(all.len(), 2);
    }

    // --- Cascade delete tests ---

    #[test]
    fn test_cascade_delete_managed_repos_on_persona_delete() {
        let (db, persona_id) = setup_db_with_persona();

        let repo = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/project");
        db.with_conn(|conn| insert_managed_repo(conn, &repo))
            .unwrap();

        // Delete persona via raw SQL (simulating what the app does)
        db.with_conn(|conn| {
            conn.execute(
                "DELETE FROM personas WHERE id = ?1",
                rusqlite::params![persona_id.0],
            )
            .map_err(|e| crate::error::OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })
        .unwrap();

        // Managed repo should be gone
        let repos = db.with_conn(|conn| list_managed_repos(conn)).unwrap();
        assert!(
            repos.is_empty(),
            "Managed repos should be cascade-deleted when persona is deleted"
        );
    }

    #[test]
    fn test_cascade_delete_credentials_on_persona_delete() {
        let (db, persona_id) = setup_db_with_persona();

        let repo = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/project");
        db.with_conn(|conn| insert_managed_repo(conn, &repo))
            .unwrap();

        // Insert credential for the repo
        let cred = RepoCredential {
            id: uuid::Uuid::new_v4().to_string(),
            repo_id: repo.id.clone(),
            keyring_service_name: format!("beachead-repo-sync-{}", repo.id.0),
            credential_type: CredentialType::Token,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.with_conn(|conn| insert_repo_credential(conn, &cred))
            .unwrap();

        // Delete persona
        db.with_conn(|conn| {
            conn.execute(
                "DELETE FROM personas WHERE id = ?1",
                rusqlite::params![persona_id.0],
            )
            .map_err(|e| crate::error::OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })
        .unwrap();

        // Both repo and credential should be gone
        let repos = db.with_conn(|conn| list_managed_repos(conn)).unwrap();
        assert!(repos.is_empty());

        // Verify credential is gone by trying to get it
        let cred_result = db
            .with_conn(|conn| get_repo_credential_by_repo(conn, &repo.id))
            .unwrap();
        assert!(
            cred_result.is_none(),
            "Repo credentials should be cascade-deleted when persona is deleted"
        );
    }

    // --- Repo credential CRUD tests ---

    #[test]
    fn test_insert_and_get_repo_credential() {
        let (db, persona_id) = setup_db_with_persona();
        let repo = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/project");
        db.with_conn(|conn| insert_managed_repo(conn, &repo))
            .unwrap();

        let cred = RepoCredential {
            id: uuid::Uuid::new_v4().to_string(),
            repo_id: repo.id.clone(),
            keyring_service_name: format!("beachead-repo-sync-{}", repo.id.0),
            credential_type: CredentialType::Token,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.with_conn(|conn| insert_repo_credential(conn, &cred))
            .unwrap();

        let fetched = db
            .with_conn(|conn| get_repo_credential_by_repo(conn, &repo.id))
            .unwrap();
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.id, cred.id);
        assert_eq!(fetched.repo_id.0, repo.id.0);
        assert_eq!(
            fetched.keyring_service_name,
            format!("beachead-repo-sync-{}", repo.id.0)
        );
        assert_eq!(fetched.credential_type, CredentialType::Token);
    }

    #[test]
    fn test_get_repo_credential_none_when_missing() {
        let (db, persona_id) = setup_db_with_persona();
        let repo = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/project");
        db.with_conn(|conn| insert_managed_repo(conn, &repo))
            .unwrap();

        let result = db
            .with_conn(|conn| get_repo_credential_by_repo(conn, &repo.id))
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_delete_repo_credential() {
        let (db, persona_id) = setup_db_with_persona();
        let repo = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/project");
        db.with_conn(|conn| insert_managed_repo(conn, &repo))
            .unwrap();

        let cred = RepoCredential {
            id: uuid::Uuid::new_v4().to_string(),
            repo_id: repo.id.clone(),
            keyring_service_name: "beachead-repo-sync-test".to_string(),
            credential_type: CredentialType::UsernamePassword,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.with_conn(|conn| insert_repo_credential(conn, &cred))
            .unwrap();

        // Delete credential
        db.with_conn(|conn| delete_repo_credential(conn, &repo.id))
            .unwrap();

        // Verify gone
        let result = db
            .with_conn(|conn| get_repo_credential_by_repo(conn, &repo.id))
            .unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_cascade_delete_credential_on_repo_delete() {
        let (db, persona_id) = setup_db_with_persona();
        let repo = make_managed_repo(&persona_id, "/home/user/project", "/mirrors/project");
        db.with_conn(|conn| insert_managed_repo(conn, &repo))
            .unwrap();

        let cred = RepoCredential {
            id: uuid::Uuid::new_v4().to_string(),
            repo_id: repo.id.clone(),
            keyring_service_name: "beachead-repo-sync-test".to_string(),
            credential_type: CredentialType::Token,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.with_conn(|conn| insert_repo_credential(conn, &cred))
            .unwrap();

        // Delete the repo (not the persona)
        db.with_conn(|conn| delete_managed_repo(conn, &repo.id))
            .unwrap();

        // Credential should be cascade-deleted
        let result = db
            .with_conn(|conn| get_repo_credential_by_repo(conn, &repo.id))
            .unwrap();
        assert!(
            result.is_none(),
            "Repo credentials should be cascade-deleted when repo is deleted"
        );
    }

    // --- Insert with nullable fields ---

    #[test]
    fn test_insert_managed_repo_with_null_optional_fields() {
        let (db, persona_id) = setup_db_with_persona();
        let now = Utc::now();

        let repo = ManagedRepo {
            id: ManagedRepoId::new(),
            persona_id: persona_id.clone(),
            workspace_path: "/home/user/local-only".to_string(),
            mirror_path: "/mirrors/local-only".to_string(),
            remote_url: None,
            remote_provider: None,
            branch_strategy: BranchStrategy::Direct,
            branch_pattern: None,
            attribution_mode: AttributionMode::KeepAgent,
            sync_mode: SyncMode::LocalOnly,
            secret_scan_mode: SecretScanMode::Block,
            check_interval_seconds: 300,
            created_at: now,
            updated_at: now,
        };

        db.with_conn(|conn| insert_managed_repo(conn, &repo))
            .unwrap();

        let fetched = db
            .with_conn(|conn| get_managed_repo(conn, &repo.id))
            .unwrap();
        assert_eq!(fetched.remote_url, None);
        assert_eq!(fetched.remote_provider, None);
        assert_eq!(fetched.branch_pattern, None);
        assert_eq!(fetched.sync_mode, SyncMode::LocalOnly);
    }
}
