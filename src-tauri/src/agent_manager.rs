//! Agent Manager: handles CRUD operations for agent type configurations.
//!
//! Pre-seeds built-in agents on first run. Stores credential metadata
//! (required services, auth methods) but never stores secret values.

use std::sync::Arc;

use chrono::Utc;

use crate::db::Database;
use crate::db_ops;
use crate::error::OrchestratorError;
use crate::sbx::SbxCli;
use crate::types::{
    AgentMetadata, AgentType, AgentTypeId, AuthMethod, CreateAgentRequest, UpdateAgentRequest,
};

/// Manages agent type registrations (built-in and custom).
pub struct AgentManager {
    db: Arc<Database>,
    sbx: Option<Arc<SbxCli>>,
}

impl AgentManager {
    /// Create a new AgentManager.
    ///
    /// `sbx` is optional to allow testing without a real sbx CLI binary.
    pub fn new(db: Arc<Database>, sbx: Option<Arc<SbxCli>>) -> Self {
        Self { db, sbx }
    }

    /// Seed all 10 built-in agents if they don't already exist.
    /// This is idempotent — agents that already exist are skipped.
    pub fn seed_builtin_agents(&self) -> Result<(), OrchestratorError> {
        let builtins = builtin_agent_definitions();

        self.db.with_conn(|conn| {
            for agent in &builtins {
                // Check if this built-in already exists by name
                let exists: bool = conn
                    .query_row(
                        "SELECT COUNT(*) > 0 FROM agent_types WHERE name = ?1 AND is_builtin = 1",
                        rusqlite::params![agent.name],
                        |row| row.get(0),
                    )
                    .map_err(|e| OrchestratorError::Database(e.to_string()))?;

                if !exists {
                    db_ops::insert_agent_type(conn, agent)?;
                }
            }
            Ok(())
        })
    }

    /// Create a custom agent type.
    ///
    /// If a `kit_ref` is provided and points to a local path, validates the kit
    /// via `sbx kit validate` before saving.
    pub async fn create(&self, req: CreateAgentRequest) -> Result<AgentType, OrchestratorError> {
        // Validate name is not empty
        if req.name.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Agent name cannot be empty".to_string(),
            ));
        }

        // Check name uniqueness
        self.db.with_conn(|conn| {
            if db_ops::agent_type_name_exists(conn, &req.name, None)? {
                return Err(OrchestratorError::DuplicateName(format!(
                    "Agent type with name '{}' already exists",
                    req.name
                )));
            }
            Ok(())
        })?;

        // If kit_ref is a local path, validate it
        if let Some(ref kit_ref) = req.kit_ref {
            let path = std::path::Path::new(kit_ref);
            if path.exists() {
                if let Some(ref sbx) = self.sbx {
                    let result = sbx.kit_validate(path).await?;
                    if !result.valid {
                        return Err(OrchestratorError::Validation(format!(
                            "Kit validation failed: {}",
                            result.errors.join("; ")
                        )));
                    }
                }
            }
        }

        let now = Utc::now();
        let agent = AgentType {
            id: AgentTypeId::new(),
            name: req.name,
            sbx_agent: None, // Custom agents don't have a built-in sbx_agent identifier
            kit_ref: req.kit_ref,
            is_builtin: false,
            metadata: req.metadata.unwrap_or(AgentMetadata {
                required_secrets: vec![],
                auth_methods: vec![],
                description: String::new(),
                supports_interactive_auth: false,
            }),
            created_at: now,
            updated_at: now,
        };

        self.db.with_conn(|conn| {
            db_ops::insert_agent_type(conn, &agent)?;
            Ok(())
        })?;

        Ok(agent)
    }

    /// Get an agent type by ID.
    pub fn get(&self, id: &AgentTypeId) -> Result<AgentType, OrchestratorError> {
        self.db.with_conn(|conn| db_ops::get_agent_type(conn, id))
    }

    /// List all agent types (built-in and custom).
    pub fn list(&self) -> Result<Vec<AgentType>, OrchestratorError> {
        self.db.with_conn(|conn| db_ops::list_agent_types(conn))
    }

    /// Update a custom agent type.
    ///
    /// Built-in agents cannot be updated.
    pub async fn update(
        &self,
        id: &AgentTypeId,
        req: UpdateAgentRequest,
    ) -> Result<AgentType, OrchestratorError> {
        // Fetch existing
        let existing = self.get(id)?;

        if existing.is_builtin {
            return Err(OrchestratorError::Validation(
                "Cannot modify built-in agent types".to_string(),
            ));
        }

        let new_name = req.name.unwrap_or(existing.name.clone());
        let new_kit_ref = req.kit_ref.or(existing.kit_ref.clone());
        let new_metadata = req.metadata.unwrap_or(existing.metadata.clone());

        // Validate name is not empty
        if new_name.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Agent name cannot be empty".to_string(),
            ));
        }

        // Check name uniqueness (excluding self)
        if new_name != existing.name {
            self.db.with_conn(|conn| {
                if db_ops::agent_type_name_exists(conn, &new_name, Some(id))? {
                    return Err(OrchestratorError::DuplicateName(format!(
                        "Agent type with name '{}' already exists",
                        new_name
                    )));
                }
                Ok(())
            })?;
        }

        // If kit_ref is a local path, validate it
        if let Some(ref kit_ref) = new_kit_ref {
            let path = std::path::Path::new(kit_ref);
            if path.exists() {
                if let Some(ref sbx) = self.sbx {
                    let result = sbx.kit_validate(path).await?;
                    if !result.valid {
                        return Err(OrchestratorError::Validation(format!(
                            "Kit validation failed: {}",
                            result.errors.join("; ")
                        )));
                    }
                }
            }
        }

        let now = Utc::now();
        self.db.with_conn(|conn| {
            db_ops::update_agent_type(
                conn,
                id,
                &new_name,
                new_kit_ref.as_deref(),
                &new_metadata,
                &now,
            )
        })?;

        self.get(id)
    }

    /// Delete a custom agent type.
    ///
    /// Rejects deletion if:
    /// - The agent is a built-in type
    /// - Any personas reference this agent type
    pub fn delete(&self, id: &AgentTypeId) -> Result<(), OrchestratorError> {
        let existing = self.get(id)?;

        if existing.is_builtin {
            return Err(OrchestratorError::Validation(
                "Cannot delete built-in agent types".to_string(),
            ));
        }

        self.db.with_conn(|conn| {
            // Check referential integrity: are there personas using this agent?
            let count = db_ops::count_personas_by_agent_type(conn, id)?;
            if count > 0 {
                return Err(OrchestratorError::HasDependents(format!(
                    "{} persona(s) reference agent type '{}'",
                    count, existing.name
                )));
            }

            db_ops::delete_agent_type(conn, id)
        })
    }
}

/// Returns the definitions for all 10 built-in agents.
fn builtin_agent_definitions() -> Vec<AgentType> {
    let now = Utc::now();

    vec![
        AgentType {
            id: AgentTypeId::new(),
            name: "Claude Code".to_string(),
            sbx_agent: Some("claude".to_string()),
            kit_ref: None,
            is_builtin: true,
            metadata: AgentMetadata {
                required_secrets: vec!["anthropic".to_string()],
                auth_methods: vec![AuthMethod::ApiKey, AuthMethod::OAuth],
                description: "Anthropic's Claude Code agent for software development".to_string(),
                supports_interactive_auth: true,
            },
            created_at: now,
            updated_at: now,
        },
        AgentType {
            id: AgentTypeId::new(),
            name: "Codex".to_string(),
            sbx_agent: Some("codex".to_string()),
            kit_ref: None,
            is_builtin: true,
            metadata: AgentMetadata {
                required_secrets: vec!["openai".to_string()],
                auth_methods: vec![AuthMethod::ApiKey, AuthMethod::OAuth],
                description: "OpenAI's Codex agent for code generation and editing".to_string(),
                supports_interactive_auth: false,
            },
            created_at: now,
            updated_at: now,
        },
        AgentType {
            id: AgentTypeId::new(),
            name: "Copilot".to_string(),
            sbx_agent: Some("copilot".to_string()),
            kit_ref: None,
            is_builtin: true,
            metadata: AgentMetadata {
                required_secrets: vec!["github".to_string()],
                auth_methods: vec![AuthMethod::ApiKey],
                description: "GitHub Copilot agent for code assistance".to_string(),
                supports_interactive_auth: false,
            },
            created_at: now,
            updated_at: now,
        },
        AgentType {
            id: AgentTypeId::new(),
            name: "Cursor".to_string(),
            sbx_agent: Some("cursor".to_string()),
            kit_ref: None,
            is_builtin: true,
            metadata: AgentMetadata {
                required_secrets: vec!["cursor".to_string()],
                auth_methods: vec![AuthMethod::ApiKey, AuthMethod::OAuth],
                description: "Cursor AI agent for code editing and generation".to_string(),
                supports_interactive_auth: true,
            },
            created_at: now,
            updated_at: now,
        },
        AgentType {
            id: AgentTypeId::new(),
            name: "Droid".to_string(),
            sbx_agent: Some("droid".to_string()),
            kit_ref: None,
            is_builtin: true,
            metadata: AgentMetadata {
                required_secrets: vec!["droid".to_string()],
                auth_methods: vec![AuthMethod::ApiKey, AuthMethod::OAuth],
                description: "Factory/Droid AI agent for automated development".to_string(),
                supports_interactive_auth: true,
            },
            created_at: now,
            updated_at: now,
        },
        AgentType {
            id: AgentTypeId::new(),
            name: "Gemini".to_string(),
            sbx_agent: Some("gemini".to_string()),
            kit_ref: None,
            is_builtin: true,
            metadata: AgentMetadata {
                required_secrets: vec!["google".to_string()],
                auth_methods: vec![AuthMethod::ApiKey, AuthMethod::OAuth],
                description: "Google's Gemini agent for code and reasoning tasks".to_string(),
                supports_interactive_auth: true,
            },
            created_at: now,
            updated_at: now,
        },
        AgentType {
            id: AgentTypeId::new(),
            name: "Kiro".to_string(),
            sbx_agent: Some("kiro".to_string()),
            kit_ref: None,
            is_builtin: true,
            metadata: AgentMetadata {
                required_secrets: vec![],
                auth_methods: vec![AuthMethod::DeviceFlow],
                description: "AWS Kiro agent with device flow authentication".to_string(),
                supports_interactive_auth: true,
            },
            created_at: now,
            updated_at: now,
        },
        AgentType {
            id: AgentTypeId::new(),
            name: "OpenCode".to_string(),
            sbx_agent: Some("opencode".to_string()),
            kit_ref: None,
            is_builtin: true,
            metadata: AgentMetadata {
                required_secrets: vec![
                    "openai".to_string(),
                    "anthropic".to_string(),
                    "google".to_string(),
                    "xai".to_string(),
                    "groq".to_string(),
                    "aws".to_string(),
                ],
                auth_methods: vec![AuthMethod::ApiKey],
                description: "Multi-provider open-source coding agent".to_string(),
                supports_interactive_auth: false,
            },
            created_at: now,
            updated_at: now,
        },
        AgentType {
            id: AgentTypeId::new(),
            name: "Docker Agent".to_string(),
            sbx_agent: Some("docker-agent".to_string()),
            kit_ref: None,
            is_builtin: true,
            metadata: AgentMetadata {
                required_secrets: vec![
                    "openai".to_string(),
                    "anthropic".to_string(),
                    "google".to_string(),
                    "xai".to_string(),
                    "nebius".to_string(),
                    "mistral".to_string(),
                ],
                auth_methods: vec![AuthMethod::ApiKey],
                description: "Docker's built-in multi-provider AI agent".to_string(),
                supports_interactive_auth: false,
            },
            created_at: now,
            updated_at: now,
        },
        AgentType {
            id: AgentTypeId::new(),
            name: "Shell".to_string(),
            sbx_agent: Some("shell".to_string()),
            kit_ref: None,
            is_builtin: true,
            metadata: AgentMetadata {
                required_secrets: vec![],
                auth_methods: vec![],
                description: "Plain shell sandbox with no pre-installed agent".to_string(),
                supports_interactive_auth: false,
            },
            created_at: now,
            updated_at: now,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_manager() -> AgentManager {
        let db = Arc::new(Database::open_in_memory().unwrap());
        AgentManager::new(db, None)
    }

    #[test]
    fn test_seed_builtin_agents_creates_all_10() {
        let mgr = setup_manager();
        mgr.seed_builtin_agents().unwrap();

        let agents = mgr.list().unwrap();
        assert_eq!(agents.len(), 10);

        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"Claude Code"));
        assert!(names.contains(&"Codex"));
        assert!(names.contains(&"Copilot"));
        assert!(names.contains(&"Cursor"));
        assert!(names.contains(&"Droid"));
        assert!(names.contains(&"Gemini"));
        assert!(names.contains(&"Kiro"));
        assert!(names.contains(&"OpenCode"));
        assert!(names.contains(&"Docker Agent"));
        assert!(names.contains(&"Shell"));
    }

    #[test]
    fn test_seed_builtin_agents_is_idempotent() {
        let mgr = setup_manager();
        mgr.seed_builtin_agents().unwrap();
        mgr.seed_builtin_agents().unwrap();

        let agents = mgr.list().unwrap();
        assert_eq!(agents.len(), 10);
    }

    #[test]
    fn test_builtin_agents_have_correct_sbx_identifiers() {
        let mgr = setup_manager();
        mgr.seed_builtin_agents().unwrap();

        let agents = mgr.list().unwrap();
        let expected = vec![
            ("Claude Code", "claude"),
            ("Codex", "codex"),
            ("Copilot", "copilot"),
            ("Cursor", "cursor"),
            ("Droid", "droid"),
            ("Gemini", "gemini"),
            ("Kiro", "kiro"),
            ("OpenCode", "opencode"),
            ("Docker Agent", "docker-agent"),
            ("Shell", "shell"),
        ];

        for (name, sbx_agent) in expected {
            let agent = agents.iter().find(|a| a.name == name).unwrap();
            assert_eq!(
                agent.sbx_agent.as_deref(),
                Some(sbx_agent),
                "Agent '{}' should have sbx_agent '{}'",
                name,
                sbx_agent
            );
            assert!(agent.is_builtin);
        }
    }

    #[test]
    fn test_builtin_agents_have_correct_metadata() {
        let mgr = setup_manager();
        mgr.seed_builtin_agents().unwrap();

        let agents = mgr.list().unwrap();

        // Claude Code: requires anthropic, supports ApiKey + OAuth, interactive auth
        let claude = agents.iter().find(|a| a.name == "Claude Code").unwrap();
        assert_eq!(claude.metadata.required_secrets, vec!["anthropic"]);
        assert!(claude.metadata.auth_methods.contains(&AuthMethod::ApiKey));
        assert!(claude.metadata.auth_methods.contains(&AuthMethod::OAuth));
        assert!(claude.metadata.supports_interactive_auth);

        // Kiro: no required secrets, device flow only, interactive auth
        let kiro = agents.iter().find(|a| a.name == "Kiro").unwrap();
        assert!(kiro.metadata.required_secrets.is_empty());
        assert_eq!(kiro.metadata.auth_methods, vec![AuthMethod::DeviceFlow]);
        assert!(kiro.metadata.supports_interactive_auth);

        // Shell: no secrets, no auth methods, no interactive auth
        let shell = agents.iter().find(|a| a.name == "Shell").unwrap();
        assert!(shell.metadata.required_secrets.is_empty());
        assert!(shell.metadata.auth_methods.is_empty());
        assert!(!shell.metadata.supports_interactive_auth);

        // OpenCode: multi-provider
        let opencode = agents.iter().find(|a| a.name == "OpenCode").unwrap();
        assert!(opencode.metadata.required_secrets.contains(&"openai".to_string()));
        assert!(opencode.metadata.required_secrets.contains(&"anthropic".to_string()));
        assert!(opencode.metadata.required_secrets.contains(&"google".to_string()));
        assert!(opencode.metadata.required_secrets.contains(&"xai".to_string()));
        assert!(opencode.metadata.required_secrets.contains(&"groq".to_string()));
        assert!(opencode.metadata.required_secrets.contains(&"aws".to_string()));
    }

    #[tokio::test]
    async fn test_create_custom_agent() {
        let mgr = setup_manager();

        let req = CreateAgentRequest {
            name: "My Custom Agent".to_string(),
            kit_ref: Some("/path/to/kit".to_string()),
            metadata: Some(AgentMetadata {
                required_secrets: vec!["openai".to_string()],
                auth_methods: vec![AuthMethod::ApiKey],
                description: "A custom agent".to_string(),
                supports_interactive_auth: false,
            }),
        };

        let agent = mgr.create(req).await.unwrap();
        assert_eq!(agent.name, "My Custom Agent");
        assert_eq!(agent.kit_ref, Some("/path/to/kit".to_string()));
        assert!(!agent.is_builtin);
        assert!(agent.sbx_agent.is_none());
        assert_eq!(agent.metadata.required_secrets, vec!["openai"]);
    }

    #[tokio::test]
    async fn test_create_agent_with_empty_name_fails() {
        let mgr = setup_manager();

        let req = CreateAgentRequest {
            name: "  ".to_string(),
            kit_ref: None,
            metadata: None,
        };

        let result = mgr.create(req).await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_create_agent_duplicate_name_fails() {
        let mgr = setup_manager();

        let req = CreateAgentRequest {
            name: "Agent X".to_string(),
            kit_ref: None,
            metadata: None,
        };
        mgr.create(req).await.unwrap();

        let req2 = CreateAgentRequest {
            name: "Agent X".to_string(),
            kit_ref: None,
            metadata: None,
        };
        let result = mgr.create(req2).await;
        assert!(matches!(result, Err(OrchestratorError::DuplicateName(_))));
    }

    #[tokio::test]
    async fn test_get_agent_by_id() {
        let mgr = setup_manager();

        let req = CreateAgentRequest {
            name: "Test Agent".to_string(),
            kit_ref: None,
            metadata: None,
        };
        let created = mgr.create(req).await.unwrap();

        let fetched = mgr.get(&created.id).unwrap();
        assert_eq!(fetched.name, "Test Agent");
        assert_eq!(fetched.id.0, created.id.0);
    }

    #[test]
    fn test_get_nonexistent_agent_fails() {
        let mgr = setup_manager();

        let result = mgr.get(&AgentTypeId("nonexistent".to_string()));
        assert!(matches!(result, Err(OrchestratorError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_update_custom_agent() {
        let mgr = setup_manager();

        let req = CreateAgentRequest {
            name: "Original Name".to_string(),
            kit_ref: None,
            metadata: None,
        };
        let created = mgr.create(req).await.unwrap();

        let update_req = UpdateAgentRequest {
            name: Some("Updated Name".to_string()),
            kit_ref: Some("/new/kit/path".to_string()),
            metadata: None,
        };
        let updated = mgr.update(&created.id, update_req).await.unwrap();
        assert_eq!(updated.name, "Updated Name");
        assert_eq!(updated.kit_ref, Some("/new/kit/path".to_string()));
    }

    #[tokio::test]
    async fn test_update_builtin_agent_fails() {
        let mgr = setup_manager();
        mgr.seed_builtin_agents().unwrap();

        let agents = mgr.list().unwrap();
        let claude = agents.iter().find(|a| a.name == "Claude Code").unwrap();

        let update_req = UpdateAgentRequest {
            name: Some("Renamed Claude".to_string()),
            kit_ref: None,
            metadata: None,
        };
        let result = mgr.update(&claude.id, update_req).await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_update_agent_duplicate_name_fails() {
        let mgr = setup_manager();

        let req1 = CreateAgentRequest {
            name: "Agent A".to_string(),
            kit_ref: None,
            metadata: None,
        };
        mgr.create(req1).await.unwrap();

        let req2 = CreateAgentRequest {
            name: "Agent B".to_string(),
            kit_ref: None,
            metadata: None,
        };
        let agent_b = mgr.create(req2).await.unwrap();

        let update_req = UpdateAgentRequest {
            name: Some("Agent A".to_string()),
            kit_ref: None,
            metadata: None,
        };
        let result = mgr.update(&agent_b.id, update_req).await;
        assert!(matches!(result, Err(OrchestratorError::DuplicateName(_))));
    }

    #[tokio::test]
    async fn test_delete_custom_agent() {
        let mgr = setup_manager();

        let req = CreateAgentRequest {
            name: "Deletable Agent".to_string(),
            kit_ref: None,
            metadata: None,
        };
        let created = mgr.create(req).await.unwrap();

        mgr.delete(&created.id).unwrap();

        let result = mgr.get(&created.id);
        assert!(matches!(result, Err(OrchestratorError::NotFound(_))));
    }

    #[test]
    fn test_delete_builtin_agent_fails() {
        let mgr = setup_manager();
        mgr.seed_builtin_agents().unwrap();

        let agents = mgr.list().unwrap();
        let claude = agents.iter().find(|a| a.name == "Claude Code").unwrap();

        let result = mgr.delete(&claude.id);
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_delete_agent_with_dependent_persona_fails() {
        let mgr = setup_manager();

        let req = CreateAgentRequest {
            name: "Referenced Agent".to_string(),
            kit_ref: None,
            metadata: None,
        };
        let agent = mgr.create(req).await.unwrap();

        // Manually insert a persona referencing this agent
        mgr.db
            .with_conn(|conn| {
                conn.execute(
                    "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                     VALUES ('p1', 'test-persona', ?1, '/tmp', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                    rusqlite::params![agent.id.0],
                )
                .map_err(|e| OrchestratorError::Database(e.to_string()))?;
                Ok(())
            })
            .unwrap();

        let result = mgr.delete(&agent.id);
        assert!(matches!(result, Err(OrchestratorError::HasDependents(_))));
    }

    #[test]
    fn test_delete_nonexistent_agent_fails() {
        let mgr = setup_manager();

        let result = mgr.delete(&AgentTypeId("nonexistent".to_string()));
        assert!(matches!(result, Err(OrchestratorError::NotFound(_))));
    }

    #[tokio::test]
    async fn test_create_agent_with_default_metadata() {
        let mgr = setup_manager();

        let req = CreateAgentRequest {
            name: "Minimal Agent".to_string(),
            kit_ref: None,
            metadata: None,
        };
        let agent = mgr.create(req).await.unwrap();

        assert!(agent.metadata.required_secrets.is_empty());
        assert!(agent.metadata.auth_methods.is_empty());
        assert!(agent.metadata.description.is_empty());
        assert!(!agent.metadata.supports_interactive_auth);
    }
}
