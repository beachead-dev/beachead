//! Persona Manager: handles CRUD operations for persona configurations.
//!
//! A persona binds an agent type to a workspace path with optional MCP servers.
//! Validates name uniqueness, agent type existence, workspace path validity,
//! and MCP server URL format. On update with active sessions, classifies changes
//! as live-applicable (additive) vs. requires-restart (removal).

use std::sync::Arc;

use chrono::Utc;

use crate::db::Database;
use crate::db_ops;
use crate::error::OrchestratorError;
use crate::types::{
    CreateMcpServerEntry, CreatePersonaRequest, Persona, PersonaId, PersonaMcpServer,
    UpdatePersonaRequest, UpdateResult,
};

/// Manages persona CRUD and MCP server entry operations.
pub struct PersonaManager {
    db: Arc<Database>,
}

impl PersonaManager {
    /// Create a new PersonaManager.
    pub fn new(db: Arc<Database>) -> Self {
        Self { db }
    }

    /// Create a new persona with validation.
    ///
    /// Validates:
    /// - Name is not empty
    /// - Name is unique
    /// - Agent type exists
    /// - Workspace path is absolute and exists
    /// - MCP server URLs have valid scheme and host
    pub fn create(&self, req: CreatePersonaRequest) -> Result<Persona, OrchestratorError> {
        // Validate name is not empty
        if req.name.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Persona name cannot be empty".to_string(),
            ));
        }

        // Validate workspace path is absolute
        if !req.workspace_path.is_absolute() {
            return Err(OrchestratorError::Validation(
                "Workspace path must be absolute".to_string(),
            ));
        }

        // Validate workspace path exists
        if !req.workspace_path.exists() {
            return Err(OrchestratorError::WorkspaceNotFound(
                req.workspace_path.to_string_lossy().to_string(),
            ));
        }

        // Validate MCP server URLs if provided
        if let Some(ref mcp_servers) = req.mcp_servers {
            for entry in mcp_servers {
                validate_mcp_url(&entry.url)?;
            }
        }

        // Check name uniqueness and agent type existence inside DB transaction
        self.db.with_conn(|conn| {
            if db_ops::persona_name_exists(conn, &req.name, None)? {
                return Err(OrchestratorError::DuplicateName(format!(
                    "Persona with name '{}' already exists",
                    req.name
                )));
            }

            // Verify agent type exists
            db_ops::get_agent_type(conn, &req.agent_type_id)?;

            let now = Utc::now();
            let persona_id = PersonaId::new();

            // Build MCP server entries
            let mcp_servers: Vec<PersonaMcpServer> = req
                .mcp_servers
                .unwrap_or_default()
                .into_iter()
                .map(|entry| PersonaMcpServer {
                    id: uuid::Uuid::new_v4().to_string(),
                    persona_id: persona_id.clone(),
                    name: entry.name,
                    url: entry.url,
                    description: entry.description,
                    auth_headers: entry.auth_headers,
                    created_at: now,
                    updated_at: now,
                })
                .collect();

            let persona = Persona {
                id: persona_id,
                name: req.name,
                agent_type_id: req.agent_type_id,
                workspace_path: req.workspace_path,
                memory_enabled: req.memory_enabled.unwrap_or(false),
                agent_cli_args: req.agent_cli_args.unwrap_or_default(),
                mcp_servers,
                created_at: now,
                updated_at: now,
            };

            db_ops::insert_persona(conn, &persona)?;

            Ok(persona)
        })
    }

    /// Get a persona by ID.
    pub fn get(&self, id: &PersonaId) -> Result<Persona, OrchestratorError> {
        self.db.with_conn(|conn| db_ops::get_persona(conn, id))
    }

    /// List all personas.
    pub fn list(&self) -> Result<Vec<Persona>, OrchestratorError> {
        self.db.with_conn(|conn| db_ops::list_personas(conn))
    }

    /// Update a persona.
    ///
    /// If there are active sessions, classifies changes:
    /// - Additive (new/modified MCP servers): returns `UpdateResult::Applied`
    /// - Removal (MCP servers removed): returns `UpdateResult::RequiresRestart`
    ///
    /// If no active sessions, always returns `UpdateResult::Applied`.
    pub fn update(
        &self,
        id: &PersonaId,
        req: UpdatePersonaRequest,
    ) -> Result<UpdateResult, OrchestratorError> {
        let existing = self.get(id)?;

        let new_name = req.name.unwrap_or(existing.name.clone());
        let new_agent_type_id = req.agent_type_id.unwrap_or(existing.agent_type_id.clone());
        let new_workspace_path = req.workspace_path.unwrap_or(existing.workspace_path.clone());
        let new_memory_enabled = req.memory_enabled.unwrap_or(existing.memory_enabled);
        let new_cli_args = req.agent_cli_args.unwrap_or(existing.agent_cli_args.clone());

        // Validate name is not empty
        if new_name.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Persona name cannot be empty".to_string(),
            ));
        }

        // Validate workspace path
        if !new_workspace_path.is_absolute() {
            return Err(OrchestratorError::Validation(
                "Workspace path must be absolute".to_string(),
            ));
        }
        if !new_workspace_path.exists() {
            return Err(OrchestratorError::WorkspaceNotFound(
                new_workspace_path.to_string_lossy().to_string(),
            ));
        }

        // Validate MCP server URLs if provided
        if let Some(ref mcp_servers) = req.mcp_servers {
            for entry in mcp_servers {
                validate_mcp_url(&entry.url)?;
            }
        }

        // Determine if MCP servers are being removed (requires-restart)
        let has_active_sessions = self.db.with_conn(|conn| {
            db_ops::count_active_sessions_for_persona(conn, id)
        })? > 0;

        let requires_restart = if has_active_sessions {
            if let Some(ref new_mcp_servers) = req.mcp_servers {
                classify_mcp_changes(&existing.mcp_servers, new_mcp_servers)
            } else {
                false
            }
        } else {
            false
        };

        self.db.with_conn(|conn| {
            // Check name uniqueness (excluding self)
            if new_name != existing.name {
                if db_ops::persona_name_exists(conn, &new_name, Some(id))? {
                    return Err(OrchestratorError::DuplicateName(format!(
                        "Persona with name '{}' already exists",
                        new_name
                    )));
                }
            }

            // Verify agent type exists if changed
            if new_agent_type_id != existing.agent_type_id {
                db_ops::get_agent_type(conn, &new_agent_type_id)?;
            }

            let now = Utc::now();
            let cli_args_json = serde_json::to_string(&new_cli_args)
                .map_err(|e| OrchestratorError::Internal(e.to_string()))?;

            conn.execute(
                "UPDATE personas SET name = ?1, agent_type_id = ?2, workspace_path = ?3, \
                 memory_enabled = ?4, agent_cli_args = ?5, updated_at = ?6 WHERE id = ?7",
                rusqlite::params![
                    new_name,
                    new_agent_type_id.0,
                    new_workspace_path.to_string_lossy().to_string(),
                    new_memory_enabled as i32,
                    cli_args_json,
                    now.to_rfc3339(),
                    id.0,
                ],
            )?;

            // Handle MCP server updates: replace all entries
            if let Some(new_mcp_entries) = req.mcp_servers {
                // Remove existing MCP servers
                let existing_servers = db_ops::list_persona_mcp_servers(conn, id)?;
                for server in &existing_servers {
                    db_ops::delete_persona_mcp_server(conn, &server.id)?;
                }

                // Insert new MCP servers
                for entry in &new_mcp_entries {
                    let mcp = PersonaMcpServer {
                        id: uuid::Uuid::new_v4().to_string(),
                        persona_id: id.clone(),
                        name: entry.name.clone(),
                        url: entry.url.clone(),
                        description: entry.description.clone(),
                        auth_headers: entry.auth_headers.clone(),
                        created_at: now,
                        updated_at: now,
                    };
                    db_ops::insert_persona_mcp_server(conn, &mcp)?;
                }
            }

            Ok(())
        })?;

        let updated_persona = self.get(id)?;

        if requires_restart {
            Ok(UpdateResult::RequiresRestart {
                persona: updated_persona,
                reason: "MCP server(s) were removed; active sessions need restart".to_string(),
            })
        } else {
            Ok(UpdateResult::Applied {
                persona: updated_persona,
            })
        }
    }

    /// Delete a persona.
    ///
    /// Rejects deletion if there are active sessions (status = 'running' or 'starting').
    /// Removes inactive sessions referencing this persona before deletion.
    pub fn delete(&self, id: &PersonaId) -> Result<(), OrchestratorError> {
        // Verify persona exists
        let _existing = self.get(id)?;

        self.db.with_conn(|conn| {
            let active_count = db_ops::count_active_sessions_for_persona(conn, id)?;
            if active_count > 0 {
                return Err(OrchestratorError::ActiveSessions);
            }

            // Remove inactive sessions referencing this persona (FK constraint)
            conn.execute(
                "DELETE FROM sessions WHERE persona_id = ?1",
                rusqlite::params![id.0],
            )?;

            conn.execute("DELETE FROM personas WHERE id = ?1", rusqlite::params![id.0])?;
            Ok(())
        })
    }

    /// Add an MCP server entry to an existing persona.
    pub fn add_mcp_server(
        &self,
        persona_id: &PersonaId,
        entry: CreateMcpServerEntry,
    ) -> Result<PersonaMcpServer, OrchestratorError> {
        // Verify persona exists
        let _persona = self.get(persona_id)?;

        validate_mcp_url(&entry.url)?;

        let now = Utc::now();
        let mcp = PersonaMcpServer {
            id: uuid::Uuid::new_v4().to_string(),
            persona_id: persona_id.clone(),
            name: entry.name,
            url: entry.url,
            description: entry.description,
            auth_headers: entry.auth_headers,
            created_at: now,
            updated_at: now,
        };

        self.db.with_conn(|conn| {
            db_ops::insert_persona_mcp_server(conn, &mcp)?;
            Ok(mcp.clone())
        })
    }

    /// Update an MCP server entry.
    pub fn update_mcp_server(
        &self,
        mcp_id: &str,
        name: &str,
        url: &str,
        description: Option<&str>,
        auth_headers: Option<&serde_json::Value>,
    ) -> Result<PersonaMcpServer, OrchestratorError> {
        validate_mcp_url(url)?;

        let now = Utc::now();
        self.db.with_conn(|conn| {
            db_ops::update_persona_mcp_server(conn, mcp_id, name, url, description, auth_headers, &now)?;
            db_ops::get_persona_mcp_server(conn, mcp_id)
        })
    }

    /// Remove an MCP server entry.
    pub fn remove_mcp_server(&self, mcp_id: &str) -> Result<(), OrchestratorError> {
        self.db.with_conn(|conn| {
            db_ops::delete_persona_mcp_server(conn, mcp_id)
        })
    }
}

/// Validate that a URL has a valid scheme (http or https) and a host.
fn validate_mcp_url(url: &str) -> Result<(), OrchestratorError> {
    // Check scheme
    let has_valid_scheme = url.starts_with("http://") || url.starts_with("https://");
    if !has_valid_scheme {
        return Err(OrchestratorError::Validation(format!(
            "MCP server URL must use http:// or https:// scheme: {}",
            url
        )));
    }

    // Extract host portion (after scheme, before optional port/path)
    let after_scheme = if url.starts_with("https://") {
        &url[8..]
    } else {
        &url[7..]
    };

    // Host is the part before the first '/' or ':' or end of string
    let host = after_scheme
        .split(|c| c == '/' || c == ':')
        .next()
        .unwrap_or("");

    if host.is_empty() {
        return Err(OrchestratorError::Validation(format!(
            "MCP server URL must have a valid host: {}",
            url
        )));
    }

    Ok(())
}

/// Classify MCP server changes to determine if a restart is required.
///
/// Returns `true` if any existing MCP servers were removed (requires restart).
/// Returns `false` if changes are only additive or modifications.
fn classify_mcp_changes(
    existing: &[PersonaMcpServer],
    new_entries: &[CreateMcpServerEntry],
) -> bool {
    // Check if any existing server name is missing from the new list
    let new_names: std::collections::HashSet<&str> =
        new_entries.iter().map(|e| e.name.as_str()).collect();

    for existing_server in existing {
        if !new_names.contains(existing_server.name.as_str()) {
            return true; // A server was removed
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentMetadata, AgentType, AgentTypeId, AuthMethod};
    use proptest::prelude::*;

    fn setup_manager() -> PersonaManager {
        let db = Arc::new(Database::open_in_memory().unwrap());
        PersonaManager::new(db)
    }

    fn setup_manager_with_agent(db: &Arc<Database>) -> AgentTypeId {
        let now = Utc::now();
        let agent_id = AgentTypeId::new();
        let agent = AgentType {
            id: agent_id.clone(),
            name: "Test Agent".to_string(),
            sbx_agent: Some("test".to_string()),
            kit_ref: None,
            is_builtin: true,
            metadata: AgentMetadata {
                required_secrets: vec![],
                auth_methods: vec![AuthMethod::ApiKey],
                description: "Test agent".to_string(),
                supports_interactive_auth: false,
                mcp_config_path: None,
            },
            created_at: now,
            updated_at: now,
        };
        db.with_conn(|conn| db_ops::insert_agent_type(conn, &agent)).unwrap();
        agent_id
    }

    fn temp_workspace() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn test_create_persona_success() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "My Persona".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: Some(true),
            agent_cli_args: Some(vec!["--verbose".to_string()]),
            mcp_servers: Some(vec![CreateMcpServerEntry {
                name: "test-mcp".to_string(),
                url: "http://localhost:8080".to_string(),
                description: Some("A test MCP server".to_string()),
                auth_headers: None,
            }]),
        };

        let persona = mgr.create(req).unwrap();
        assert_eq!(persona.name, "My Persona");
        assert!(persona.memory_enabled);
        assert_eq!(persona.agent_cli_args, vec!["--verbose"]);
        assert_eq!(persona.mcp_servers.len(), 1);
        assert_eq!(persona.mcp_servers[0].name, "test-mcp");
    }

    #[test]
    fn test_create_persona_empty_name_fails() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "  ".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };

        let result = mgr.create(req);
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[test]
    fn test_create_persona_duplicate_name_fails() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "Duplicate".to_string(),
            agent_type_id: agent_id.clone(),
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        mgr.create(req).unwrap();

        let req2 = CreatePersonaRequest {
            name: "Duplicate".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        let result = mgr.create(req2);
        assert!(matches!(result, Err(OrchestratorError::DuplicateName(_))));
    }

    #[test]
    fn test_create_persona_nonexistent_agent_fails() {
        let mgr = setup_manager();
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "Bad Agent".to_string(),
            agent_type_id: AgentTypeId("nonexistent".to_string()),
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };

        let result = mgr.create(req);
        assert!(matches!(result, Err(OrchestratorError::NotFound(_))));
    }

    #[test]
    fn test_create_persona_relative_workspace_fails() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);

        let req = CreatePersonaRequest {
            name: "Relative Path".to_string(),
            agent_type_id: agent_id,
            workspace_path: std::path::PathBuf::from("relative/path"),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };

        let result = mgr.create(req);
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[test]
    fn test_create_persona_nonexistent_workspace_fails() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);

        let req = CreatePersonaRequest {
            name: "Bad Workspace".to_string(),
            agent_type_id: agent_id,
            workspace_path: std::path::PathBuf::from("/nonexistent/path/xyz123"),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };

        let result = mgr.create(req);
        assert!(matches!(result, Err(OrchestratorError::WorkspaceNotFound(_))));
    }

    #[test]
    fn test_create_persona_invalid_mcp_url_fails() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "Bad MCP".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: Some(vec![CreateMcpServerEntry {
                name: "bad".to_string(),
                url: "ftp://invalid.com".to_string(),
                description: None,
                auth_headers: None,
            }]),
        };

        let result = mgr.create(req);
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[test]
    fn test_create_persona_mcp_url_no_host_fails() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "No Host".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: Some(vec![CreateMcpServerEntry {
                name: "bad".to_string(),
                url: "http://".to_string(),
                description: None,
                auth_headers: None,
            }]),
        };

        let result = mgr.create(req);
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[test]
    fn test_get_persona() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "Get Me".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        let created = mgr.create(req).unwrap();

        let fetched = mgr.get(&created.id).unwrap();
        assert_eq!(fetched.name, "Get Me");
        assert_eq!(fetched.id.0, created.id.0);
    }

    #[test]
    fn test_get_nonexistent_persona_fails() {
        let mgr = setup_manager();
        let result = mgr.get(&PersonaId("nonexistent".to_string()));
        assert!(matches!(result, Err(OrchestratorError::NotFound(_))));
    }

    #[test]
    fn test_list_personas() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req1 = CreatePersonaRequest {
            name: "Alpha".to_string(),
            agent_type_id: agent_id.clone(),
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        let req2 = CreatePersonaRequest {
            name: "Beta".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        mgr.create(req1).unwrap();
        mgr.create(req2).unwrap();

        let personas = mgr.list().unwrap();
        assert_eq!(personas.len(), 2);
    }

    #[test]
    fn test_update_persona_name() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "Original".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        let created = mgr.create(req).unwrap();

        let update_req = UpdatePersonaRequest {
            name: Some("Renamed".to_string()),
            agent_type_id: None,
            workspace_path: None,
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        let result = mgr.update(&created.id, update_req).unwrap();
        match result {
            UpdateResult::Applied { persona } => {
                assert_eq!(persona.name, "Renamed");
            }
            _ => panic!("Expected Applied result"),
        }
    }

    #[test]
    fn test_update_persona_duplicate_name_fails() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req1 = CreatePersonaRequest {
            name: "First".to_string(),
            agent_type_id: agent_id.clone(),
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        mgr.create(req1).unwrap();

        let req2 = CreatePersonaRequest {
            name: "Second".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        let second = mgr.create(req2).unwrap();

        let update_req = UpdatePersonaRequest {
            name: Some("First".to_string()),
            agent_type_id: None,
            workspace_path: None,
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        let result = mgr.update(&second.id, update_req);
        assert!(matches!(result, Err(OrchestratorError::DuplicateName(_))));
    }

    #[test]
    fn test_update_persona_with_mcp_servers() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "MCP Persona".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: Some(vec![CreateMcpServerEntry {
                name: "server-a".to_string(),
                url: "http://localhost:9000".to_string(),
                description: None,
                auth_headers: None,
            }]),
        };
        let created = mgr.create(req).unwrap();
        assert_eq!(created.mcp_servers.len(), 1);

        // Update with new MCP servers
        let update_req = UpdatePersonaRequest {
            name: None,
            agent_type_id: None,
            workspace_path: None,
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: Some(vec![
                CreateMcpServerEntry {
                    name: "server-a".to_string(),
                    url: "http://localhost:9001".to_string(),
                    description: None,
                    auth_headers: None,
                },
                CreateMcpServerEntry {
                    name: "server-b".to_string(),
                    url: "https://example.com/mcp".to_string(),
                    description: Some("New server".to_string()),
                    auth_headers: None,
                },
            ]),
        };
        let result = mgr.update(&created.id, update_req).unwrap();
        match result {
            UpdateResult::Applied { persona } => {
                assert_eq!(persona.mcp_servers.len(), 2);
            }
            _ => panic!("Expected Applied result (no active sessions)"),
        }
    }

    #[test]
    fn test_update_with_active_sessions_additive_is_applied() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db.clone());
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "Active Persona".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: Some(vec![CreateMcpServerEntry {
                name: "existing".to_string(),
                url: "http://localhost:8080".to_string(),
                description: None,
                auth_headers: None,
            }]),
        };
        let created = mgr.create(req).unwrap();

        // Insert an active session
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, persona_id, status, created_at, updated_at) \
                 VALUES ('s1', ?1, 'running', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                rusqlite::params![created.id.0],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        }).unwrap();

        // Additive change: keep existing + add new
        let update_req = UpdatePersonaRequest {
            name: None,
            agent_type_id: None,
            workspace_path: None,
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: Some(vec![
                CreateMcpServerEntry {
                    name: "existing".to_string(),
                    url: "http://localhost:8080".to_string(),
                    description: None,
                    auth_headers: None,
                },
                CreateMcpServerEntry {
                    name: "new-server".to_string(),
                    url: "http://localhost:9090".to_string(),
                    description: None,
                    auth_headers: None,
                },
            ]),
        };
        let result = mgr.update(&created.id, update_req).unwrap();
        assert!(matches!(result, UpdateResult::Applied { .. }));
    }

    #[test]
    fn test_update_with_active_sessions_removal_requires_restart() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db.clone());
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "Restart Persona".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: Some(vec![
                CreateMcpServerEntry {
                    name: "keep".to_string(),
                    url: "http://localhost:8080".to_string(),
                    description: None,
                    auth_headers: None,
                },
                CreateMcpServerEntry {
                    name: "remove-me".to_string(),
                    url: "http://localhost:9090".to_string(),
                    description: None,
                    auth_headers: None,
                },
            ]),
        };
        let created = mgr.create(req).unwrap();

        // Insert an active session
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, persona_id, status, created_at, updated_at) \
                 VALUES ('s1', ?1, 'running', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                rusqlite::params![created.id.0],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        }).unwrap();

        // Remove "remove-me" server
        let update_req = UpdatePersonaRequest {
            name: None,
            agent_type_id: None,
            workspace_path: None,
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: Some(vec![CreateMcpServerEntry {
                name: "keep".to_string(),
                url: "http://localhost:8080".to_string(),
                description: None,
                auth_headers: None,
            }]),
        };
        let result = mgr.update(&created.id, update_req).unwrap();
        match result {
            UpdateResult::RequiresRestart { reason, .. } => {
                assert!(reason.contains("removed"));
            }
            _ => panic!("Expected RequiresRestart result"),
        }
    }

    #[test]
    fn test_delete_persona_success() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "Deletable".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        let created = mgr.create(req).unwrap();

        mgr.delete(&created.id).unwrap();
        let result = mgr.get(&created.id);
        assert!(matches!(result, Err(OrchestratorError::NotFound(_))));
    }

    #[test]
    fn test_delete_persona_with_active_session_fails() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db.clone());
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "Active Delete".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        let created = mgr.create(req).unwrap();

        // Insert an active session
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, persona_id, status, created_at, updated_at) \
                 VALUES ('s1', ?1, 'starting', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                rusqlite::params![created.id.0],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        }).unwrap();

        let result = mgr.delete(&created.id);
        assert!(matches!(result, Err(OrchestratorError::ActiveSessions)));
    }

    #[test]
    fn test_delete_persona_with_stopped_session_succeeds() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db.clone());
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "Stopped Session".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        let created = mgr.create(req).unwrap();

        // Insert a stopped session (not active)
        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sessions (id, persona_id, status, created_at, updated_at) \
                 VALUES ('s1', ?1, 'stopped', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                rusqlite::params![created.id.0],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        }).unwrap();

        // Should succeed because session is stopped
        mgr.delete(&created.id).unwrap();
    }

    #[test]
    fn test_add_mcp_server() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "MCP Add".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        let created = mgr.create(req).unwrap();
        assert_eq!(created.mcp_servers.len(), 0);

        let entry = CreateMcpServerEntry {
            name: "new-mcp".to_string(),
            url: "https://mcp.example.com".to_string(),
            description: Some("Added later".to_string()),
            auth_headers: None,
        };
        let mcp = mgr.add_mcp_server(&created.id, entry).unwrap();
        assert_eq!(mcp.name, "new-mcp");
        assert_eq!(mcp.url, "https://mcp.example.com");

        // Verify it's persisted
        let fetched = mgr.get(&created.id).unwrap();
        assert_eq!(fetched.mcp_servers.len(), 1);
    }

    #[test]
    fn test_add_mcp_server_invalid_url_fails() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "MCP Bad URL".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: None,
        };
        let created = mgr.create(req).unwrap();

        let entry = CreateMcpServerEntry {
            name: "bad".to_string(),
            url: "ws://invalid.com".to_string(),
            description: None,
            auth_headers: None,
        };
        let result = mgr.add_mcp_server(&created.id, entry);
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[test]
    fn test_update_mcp_server() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "MCP Update".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: Some(vec![CreateMcpServerEntry {
                name: "original".to_string(),
                url: "http://localhost:8080".to_string(),
                description: None,
                auth_headers: None,
            }]),
        };
        let created = mgr.create(req).unwrap();
        let mcp_id = &created.mcp_servers[0].id;

        let updated = mgr
            .update_mcp_server(mcp_id, "renamed", "https://new.example.com", Some("Updated"), None)
            .unwrap();
        assert_eq!(updated.name, "renamed");
        assert_eq!(updated.url, "https://new.example.com");
        assert_eq!(updated.description, Some("Updated".to_string()));
    }

    #[test]
    fn test_remove_mcp_server() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let agent_id = setup_manager_with_agent(&db);
        let mgr = PersonaManager::new(db);
        let ws = temp_workspace();

        let req = CreatePersonaRequest {
            name: "MCP Remove".to_string(),
            agent_type_id: agent_id,
            workspace_path: ws.path().to_path_buf(),
            memory_enabled: None,
            agent_cli_args: None,
            mcp_servers: Some(vec![CreateMcpServerEntry {
                name: "to-remove".to_string(),
                url: "http://localhost:8080".to_string(),
                description: None,
                auth_headers: None,
            }]),
        };
        let created = mgr.create(req).unwrap();
        let mcp_id = created.mcp_servers[0].id.clone();

        mgr.remove_mcp_server(&mcp_id).unwrap();

        let fetched = mgr.get(&created.id).unwrap();
        assert_eq!(fetched.mcp_servers.len(), 0);
    }

    #[test]
    fn test_validate_mcp_url_valid_http() {
        assert!(validate_mcp_url("http://localhost:8080").is_ok());
        assert!(validate_mcp_url("http://example.com/path").is_ok());
        assert!(validate_mcp_url("https://secure.example.com").is_ok());
        assert!(validate_mcp_url("https://host.docker.internal:9000").is_ok());
    }

    #[test]
    fn test_validate_mcp_url_invalid() {
        assert!(validate_mcp_url("ftp://bad.com").is_err());
        assert!(validate_mcp_url("ws://bad.com").is_err());
        assert!(validate_mcp_url("http://").is_err());
        assert!(validate_mcp_url("https://").is_err());
        assert!(validate_mcp_url("not-a-url").is_err());
        assert!(validate_mcp_url("").is_err());
    }

    #[test]
    fn test_classify_mcp_changes_no_removal() {
        let existing = vec![make_mcp_server("server-a")];
        let new_entries = vec![
            CreateMcpServerEntry {
                name: "server-a".to_string(),
                url: "http://localhost:8080".to_string(),
                description: None,
                auth_headers: None,
            },
            CreateMcpServerEntry {
                name: "server-b".to_string(),
                url: "http://localhost:9090".to_string(),
                description: None,
                auth_headers: None,
            },
        ];
        assert!(!classify_mcp_changes(&existing, &new_entries));
    }

    #[test]
    fn test_classify_mcp_changes_with_removal() {
        let existing = vec![
            make_mcp_server("server-a"),
            make_mcp_server("server-b"),
        ];
        let new_entries = vec![CreateMcpServerEntry {
            name: "server-a".to_string(),
            url: "http://localhost:8080".to_string(),
            description: None,
            auth_headers: None,
        }];
        assert!(classify_mcp_changes(&existing, &new_entries));
    }

    fn make_mcp_server(name: &str) -> PersonaMcpServer {
        let now = Utc::now();
        PersonaMcpServer {
            id: uuid::Uuid::new_v4().to_string(),
            persona_id: PersonaId("test".to_string()),
            name: name.to_string(),
            url: "http://localhost:8080".to_string(),
            description: None,
            auth_headers: None,
            created_at: now,
            updated_at: now,
        }
    }

    // --- Property-Based Tests ---

    /// Generate a unique MCP server name
    fn arb_mcp_name() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9-]{1,15}".prop_map(|s| s)
    }

    /// Generate a set of unique MCP server names (1..8 names)
    fn arb_unique_names(min: usize, max: usize) -> impl Strategy<Value = Vec<String>> {
        proptest::collection::hash_set(arb_mcp_name(), min..max)
            .prop_map(|set| set.into_iter().collect::<Vec<_>>())
    }

    // Property 3: Edit classification — additive vs. removal
    // **Validates: Requirements 1.8, 1.9**
    //
    // For any persona edit diff applied to a persona with active sessions:
    // - If all existing MCP server names appear in the new entries (additive-only),
    //   classify_mcp_changes returns false (live-applicable).
    // - If any existing MCP server name is missing from the new entries (removal),
    //   classify_mcp_changes returns true (requires-restart).
    proptest! {
        #[test]
        fn prop_edit_classification_additive_only(
            existing_names in arb_unique_names(1, 6),
            extra_names in arb_unique_names(0, 5),
        ) {
            // Build existing MCP servers from existing_names
            let existing: Vec<PersonaMcpServer> = existing_names
                .iter()
                .map(|n| make_mcp_server(n))
                .collect();

            // Build new entries that include ALL existing names plus extras
            // Filter extras to avoid duplicates with existing names
            let mut new_entry_names: Vec<String> = existing_names.clone();
            for extra in &extra_names {
                if !new_entry_names.contains(extra) {
                    new_entry_names.push(extra.clone());
                }
            }

            let new_entries: Vec<CreateMcpServerEntry> = new_entry_names
                .iter()
                .enumerate()
                .map(|(i, name)| CreateMcpServerEntry {
                    name: name.clone(),
                    url: format!("http://localhost:{}", 8000 + i),
                    description: None,
                    auth_headers: None,
                })
                .collect();

            // Additive-only: all existing names present in new entries
            let result = classify_mcp_changes(&existing, &new_entries);
            prop_assert!(
                !result,
                "Expected additive-only (false) but got requires-restart (true). \
                 existing={:?}, new={:?}",
                existing_names, new_entry_names
            );
        }

        #[test]
        fn prop_edit_classification_with_removal(
            all_names in arb_unique_names(2, 8),
            remove_count in 1usize..4,
        ) {
            // Ensure we have enough names to remove at least one
            prop_assume!(all_names.len() >= 2);
            let actual_remove = remove_count.min(all_names.len() - 1);

            // Build existing MCP servers from all names
            let existing: Vec<PersonaMcpServer> = all_names
                .iter()
                .map(|n| make_mcp_server(n))
                .collect();

            // Build new entries that are MISSING some existing names (removal)
            let kept_names = &all_names[actual_remove..];
            let new_entries: Vec<CreateMcpServerEntry> = kept_names
                .iter()
                .enumerate()
                .map(|(i, name)| CreateMcpServerEntry {
                    name: name.clone(),
                    url: format!("http://localhost:{}", 9000 + i),
                    description: None,
                    auth_headers: None,
                })
                .collect();

            // Removal detected: at least one existing name missing from new entries
            let result = classify_mcp_changes(&existing, &new_entries);
            prop_assert!(
                result,
                "Expected requires-restart (true) but got additive (false). \
                 existing={:?}, kept={:?}, removed={:?}",
                all_names, kept_names, &all_names[..actual_remove]
            );
        }

        // Property 12: MCP server URL validation
        // **Validates: Requirements 10.6**
        //
        // For any well-formed URL (http:// or https:// scheme with a non-empty host,
        // optional port, optional path), validate_mcp_url accepts it.
        // For any malformed string (missing scheme, wrong scheme, empty host, empty
        // string, random strings), validate_mcp_url rejects it.

        #[test]
        fn prop_wellformed_urls_accepted(
            scheme in prop_oneof![Just("http"), Just("https")],
            host in "[a-z][a-z0-9]{0,15}(\\.[a-z]{2,6}){0,2}",
            port in proptest::option::of(1024u16..65535),
            path in proptest::option::of("/[a-z0-9/]{1,20}"),
        ) {
            let url = match (port, path) {
                (Some(p), Some(ref pa)) => format!("{}://{}:{}{}", scheme, host, p, pa),
                (Some(p), None) => format!("{}://{}:{}", scheme, host, p),
                (None, Some(ref pa)) => format!("{}://{}{}", scheme, host, pa),
                (None, None) => format!("{}://{}", scheme, host),
            };
            prop_assert!(
                validate_mcp_url(&url).is_ok(),
                "Expected well-formed URL to be accepted: {}",
                url
            );
        }

        #[test]
        fn prop_malformed_urls_rejected(
            malformed in prop_oneof![
                // Missing scheme entirely: random alphanumeric string
                "[a-z][a-z0-9]{2,20}",
                // Wrong scheme (ftp)
                Just("ftp://".to_string()).prop_flat_map(|s| {
                    "[a-z]{3,10}".prop_map(move |host| format!("{}{}", s, host))
                }),
                // Wrong scheme (ws)
                Just("ws://".to_string()).prop_flat_map(|s| {
                    "[a-z]{3,10}".prop_map(move |host| format!("{}{}", s, host))
                }),
                // http:// with empty host
                Just("http://".to_string()),
                // https:// with empty host
                Just("https://".to_string()),
                // Empty string
                Just("".to_string()),
            ],
        ) {
            prop_assert!(
                validate_mcp_url(&malformed).is_err(),
                "Expected malformed URL to be rejected: {}",
                malformed
            );
        }
    }
}
