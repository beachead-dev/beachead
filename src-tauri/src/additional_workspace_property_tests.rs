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

    /// Generate a unique workspace path based on an index
    fn arb_indexed_workspace_path(index: usize) -> PathBuf {
        PathBuf::from(format!("/workspace/additional_{}", index))
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

    // Feature: multi-workspace-mounts, Property 10: Position assignment and retrieval ordering
    // **Validates: Requirements 9.2, 9.3**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_position_assignment_and_retrieval_ordering(
            agent in arb_agent_type(),
            persona_name in arb_name(),
            primary_workspace in arb_workspace_path(),
            n in 1usize..20,
            read_only_flags in prop::collection::vec(any::<bool>(), 20),
            labels in prop::collection::vec(arb_label(), 20),
        ) {
            let db = Database::open_in_memory().unwrap();

            let workspaces_result = db.with_conn(|conn| {
                // Setup: insert agent type and persona
                insert_agent_type(conn, &agent)?;

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

                // Insert N additional workspaces with sequential positions 0..N-1
                let mut inserted_paths: Vec<String> = Vec::new();
                let mut inserted_read_only: Vec<bool> = Vec::new();
                let mut inserted_labels: Vec<Option<String>> = Vec::new();
                for i in 0..n {
                    let path = arb_indexed_workspace_path(i);
                    let ro = read_only_flags[i % read_only_flags.len()];
                    let label = labels[i % labels.len()].clone();
                    let ws = AdditionalWorkspace {
                        id: uuid::Uuid::new_v4().to_string(),
                        persona_id: persona_id.clone(),
                        path: path.clone(),
                        read_only: ro,
                        position: i as i32,
                        label: label.clone(),
                        created_at: now,
                    };
                    insert_additional_workspace(conn, &ws)?;
                    inserted_paths.push(path.to_string_lossy().to_string());
                    inserted_read_only.push(ro);
                    inserted_labels.push(label);
                }

                // Retrieve workspaces
                let retrieved = list_additional_workspaces(conn, &persona_id)?;

                Ok((n, inserted_paths, inserted_read_only, inserted_labels, retrieved))
            }).unwrap();

            let (expected_count, inserted_paths, inserted_read_only, inserted_labels, retrieved) = workspaces_result;

            // Verify: returned vec has exactly N elements
            prop_assert_eq!(
                retrieved.len(),
                expected_count,
                "Expected {} workspaces, got {}",
                expected_count,
                retrieved.len()
            );

            // Verify: positions are 0, 1, 2, ..., N-1 in order
            for (i, ws) in retrieved.iter().enumerate() {
                prop_assert_eq!(
                    ws.position,
                    i as i32,
                    "Workspace at index {} has position {}, expected {}",
                    i,
                    ws.position,
                    i
                );
            }

            // Verify: the order matches the insertion order (paths match)
            for (i, ws) in retrieved.iter().enumerate() {
                let retrieved_path = ws.path.to_string_lossy().to_string();
                prop_assert_eq!(
                    &retrieved_path,
                    &inserted_paths[i],
                    "Workspace at index {} has path '{}', expected '{}'",
                    i,
                    retrieved_path,
                    inserted_paths[i]
                );
                // Also verify read_only and label are preserved in order
                prop_assert_eq!(
                    ws.read_only,
                    inserted_read_only[i],
                    "Workspace at index {} has read_only={}, expected {}",
                    i,
                    ws.read_only,
                    inserted_read_only[i]
                );
                prop_assert_eq!(
                    &ws.label,
                    &inserted_labels[i],
                    "Workspace at index {} has label {:?}, expected {:?}",
                    i,
                    ws.label,
                    inserted_labels[i]
                );
            }
        }
    }
}
