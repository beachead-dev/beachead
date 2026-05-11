//! Property-based tests for additional workspace operations.
//! Uses proptest to generate arbitrary domain objects and verify correctness properties.

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use proptest::prelude::*;
    use std::path::PathBuf;

    use crate::db::Database;
    use crate::db_ops::*;
    use crate::types::*;

    // --- Arbitrary Generators ---

    /// Generate a valid identifier string (alphanumeric + hyphens, 1-30 chars)
    fn arb_name() -> impl Strategy<Value = String> {
        "[a-zA-Z][a-zA-Z0-9_-]{0,29}".prop_map(|s| s)
    }

    /// Generate a valid workspace path
    fn arb_workspace_path() -> impl Strategy<Value = PathBuf> {
        "[a-zA-Z]{1,5}(/[a-zA-Z0-9_-]{1,10}){1,4}"
            .prop_map(|s| PathBuf::from(format!("/tmp/{}", s)))
    }

    /// Generate an arbitrary AgentType for test setup
    fn arb_agent_type() -> impl Strategy<Value = AgentType> {
        arb_name().prop_map(|name| {
            let now = Utc::now();
            AgentType {
                id: AgentTypeId::new(),
                name,
                sbx_agent: Some("claude".to_string()),
                kit_ref: None,
                is_builtin: true,
                metadata: AgentMetadata {
                    required_secrets: vec!["anthropic".to_string()],
                    auth_methods: vec![AuthMethod::ApiKey],
                    description: "Test agent".to_string(),
                    supports_interactive_auth: false,
                    mcp_config_path: None,
                },
                created_at: now,
                updated_at: now,
            }
        })
    }

    /// Generate an optional label for an additional workspace
    fn arb_label() -> impl Strategy<Value = Option<String>> {
        prop::option::of("[a-zA-Z][a-zA-Z0-9 _-]{0,20}")
    }

    // --- Property Tests ---

    // Feature: multi-workspace-mounts, Property 1: Cascade delete removes all additional workspaces
    // **Validates: Requirements 1.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_cascade_delete_removes_all_additional_workspaces(
            agent in arb_agent_type(),
            persona_name in arb_name(),
            primary_workspace in arb_workspace_path(),
            workspace_count in 0usize..20,
            workspace_paths in prop::collection::vec(arb_workspace_path(), 0..20),
            read_only_flags in prop::collection::vec(any::<bool>(), 0..20),
            labels in prop::collection::vec(arb_label(), 0..20),
        ) {
            let db = Database::open_in_memory().unwrap();

            // Clamp workspace_paths to workspace_count
            let n = workspace_count.min(workspace_paths.len()).min(read_only_flags.len()).min(labels.len());

            let persona_id = db.with_conn(|conn| {
                // Insert agent type
                insert_agent_type(conn, &agent)?;

                // Insert persona
                let now = Utc::now();
                let persona_id = PersonaId::new();
                let persona = Persona {
                    id: persona_id.clone(),
                    name: persona_name,
                    agent_type_id: agent.id.clone(),
                    workspace_path: primary_workspace,
                    memory_enabled: false,
                    agent_cli_args: vec![],
                    mcp_servers: vec![],
                    additional_workspaces: vec![],
                    created_at: now,
                    updated_at: now,
                };
                insert_persona(conn, &persona)?;

                // Insert N additional workspaces
                for i in 0..n {
                    let ws = AdditionalWorkspace {
                        id: uuid::Uuid::new_v4().to_string(),
                        persona_id: persona_id.clone(),
                        path: workspace_paths[i].clone(),
                        read_only: read_only_flags[i],
                        position: i as i32,
                        label: labels[i].clone(),
                        created_at: now,
                    };
                    insert_additional_workspace(conn, &ws)?;
                }

                // Verify workspaces were inserted
                let before = list_additional_workspaces(conn, &persona_id)?;
                assert_eq!(before.len(), n, "Should have {} workspaces before delete", n);

                Ok(persona_id)
            }).unwrap();

            // Delete the persona (CASCADE should remove additional workspaces)
            db.with_conn(|conn| {
                conn.execute(
                    "DELETE FROM personas WHERE id = ?1",
                    rusqlite::params![persona_id.0],
                )?;
                Ok(())
            }).unwrap();

            // Verify zero additional workspace records remain for that persona_id
            let remaining = db.with_conn(|conn| {
                list_additional_workspaces(conn, &persona_id)
            }).unwrap();

            prop_assert_eq!(remaining.len(), 0, "All additional workspaces should be cascade-deleted when persona is deleted");
        }
    }

}
