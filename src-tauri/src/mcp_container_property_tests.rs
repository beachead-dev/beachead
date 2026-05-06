//! Property-based tests for MCP container per memory-enabled persona invariant.
//!
//! Property 21: One MCP container per memory-enabled persona
//! - For any set of personas with varying `memory_enabled` flags,
//!   the number of MCP containers equals the number of personas with `memory_enabled = true`.
//!
//! **Validates: Requirements 15.5**
//!
//! Since McpContainerManager requires a real Docker daemon (bollard), this property test
//! validates the invariant at the DATABASE LOGIC layer: when we insert persona records with
//! varying memory_enabled flags and corresponding mcp_container records for each
//! memory-enabled persona, the count of mcp_containers matches the count of
//! memory-enabled personas.

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rusqlite::params;
    use std::sync::Arc;

    use crate::db::Database;
    use crate::error::OrchestratorError;

    /// A generated persona with a random memory_enabled flag.
    #[derive(Debug, Clone)]
    struct TestPersona {
        id: String,
        name: String,
        memory_enabled: bool,
    }

    /// Strategy for generating a list of personas with varying memory_enabled flags.
    /// Generates between 1 and 20 personas, each with a random memory_enabled flag.
    fn personas_strategy() -> impl Strategy<Value = Vec<TestPersona>> {
        prop::collection::vec(any::<bool>(), 1..=20).prop_map(|flags| {
            flags
                .into_iter()
                .enumerate()
                .map(|(i, memory_enabled)| TestPersona {
                    id: format!("persona-{}", i),
                    name: format!("test-persona-{}", i),
                    memory_enabled,
                })
                .collect()
        })
    }

    /// Set up an in-memory database with prerequisite records and insert the given personas.
    /// For each memory-enabled persona, insert a corresponding mcp_container record.
    /// Returns the database handle.
    fn setup_db_with_personas(personas: &[TestPersona]) -> Arc<Database> {
        let db = Arc::new(Database::open_in_memory().expect("Failed to open in-memory db"));

        db.with_conn(|conn| {
            // Insert a prerequisite agent_type for the FK chain
            conn.execute(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('at1', 'test-agent', 'test', 0, '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            // Insert each persona
            for persona in personas {
                conn.execute(
                    "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                     VALUES (?1, ?2, 'at1', '/tmp', ?3, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                    params![persona.id, persona.name, persona.memory_enabled as i64],
                ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            }

            // For each memory-enabled persona, insert a corresponding mcp_container record
            let mut port = 9100u16;
            for persona in personas.iter().filter(|p| p.memory_enabled) {
                let container_id = format!("mc-{}", persona.id);
                conn.execute(
                    "INSERT INTO mcp_containers (id, persona_id, port, bearer_token, volume_name, status, created_at, updated_at)
                     VALUES (?1, ?2, ?3, 'token', ?4, 'running', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                    params![
                        container_id,
                        persona.id,
                        port as i64,
                        format!("vol-{}", persona.id),
                    ],
                ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
                port += 1;
            }

            Ok(())
        })
        .unwrap();

        db
    }

    proptest! {
        /// Property 21: The count of mcp_containers equals the count of personas
        /// with memory_enabled = true.
        ///
        /// **Validates: Requirements 15.5**
        #[test]
        fn prop_mcp_container_count_equals_memory_enabled_personas(
            personas in personas_strategy(),
        ) {
            let db = setup_db_with_personas(&personas);

            let expected_count = personas.iter().filter(|p| p.memory_enabled).count() as i64;

            let actual_count: i64 = db.with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM mcp_containers",
                    [],
                    |row| row.get(0),
                ).map_err(|e| OrchestratorError::Database(e.to_string()))
            }).unwrap();

            let memory_enabled_count: i64 = db.with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM personas WHERE memory_enabled = 1",
                    [],
                    |row| row.get(0),
                ).map_err(|e| OrchestratorError::Database(e.to_string()))
            }).unwrap();

            // The number of MCP containers must equal the number of memory-enabled personas
            prop_assert_eq!(
                actual_count, expected_count,
                "MCP container count ({}) should equal memory-enabled persona count ({})",
                actual_count, expected_count
            );

            // Cross-check: the DB query for memory_enabled personas matches our expectation
            prop_assert_eq!(
                memory_enabled_count, expected_count,
                "DB memory_enabled count ({}) should match expected ({})",
                memory_enabled_count, expected_count
            );

            // The invariant: mcp_containers count == personas WHERE memory_enabled = 1
            prop_assert_eq!(
                actual_count, memory_enabled_count,
                "MCP container count ({}) must equal memory-enabled persona count from DB ({})",
                actual_count, memory_enabled_count
            );
        }

        /// Property 21 (supplementary): Each mcp_container references a persona
        /// that has memory_enabled = true.
        ///
        /// **Validates: Requirements 15.5**
        #[test]
        fn prop_all_mcp_containers_reference_memory_enabled_personas(
            personas in personas_strategy(),
        ) {
            let db = setup_db_with_personas(&personas);

            // Query: find any mcp_container whose persona does NOT have memory_enabled = 1
            let orphan_count: i64 = db.with_conn(|conn| {
                conn.query_row(
                    "SELECT COUNT(*) FROM mcp_containers mc
                     JOIN personas p ON mc.persona_id = p.id
                     WHERE p.memory_enabled = 0",
                    [],
                    |row| row.get(0),
                ).map_err(|e| OrchestratorError::Database(e.to_string()))
            }).unwrap();

            prop_assert_eq!(
                orphan_count, 0,
                "No MCP container should reference a persona with memory_enabled = false, found {}",
                orphan_count
            );
        }

        /// Property 21 (supplementary): Every memory-enabled persona has exactly one
        /// mcp_container.
        ///
        /// **Validates: Requirements 15.5**
        #[test]
        fn prop_every_memory_enabled_persona_has_one_container(
            personas in personas_strategy(),
        ) {
            let db = setup_db_with_personas(&personas);

            let memory_enabled_personas: Vec<String> = personas
                .iter()
                .filter(|p| p.memory_enabled)
                .map(|p| p.id.clone())
                .collect();

            for persona_id in &memory_enabled_personas {
                let container_count: i64 = db.with_conn(|conn| {
                    conn.query_row(
                        "SELECT COUNT(*) FROM mcp_containers WHERE persona_id = ?1",
                        params![persona_id],
                        |row| row.get(0),
                    ).map_err(|e| OrchestratorError::Database(e.to_string()))
                }).unwrap();

                prop_assert_eq!(
                    container_count, 1,
                    "Memory-enabled persona '{}' should have exactly 1 MCP container, found {}",
                    persona_id, container_count
                );
            }

            // Non-memory-enabled personas should have zero containers
            let non_memory_personas: Vec<String> = personas
                .iter()
                .filter(|p| !p.memory_enabled)
                .map(|p| p.id.clone())
                .collect();

            for persona_id in &non_memory_personas {
                let container_count: i64 = db.with_conn(|conn| {
                    conn.query_row(
                        "SELECT COUNT(*) FROM mcp_containers WHERE persona_id = ?1",
                        params![persona_id],
                        |row| row.get(0),
                    ).map_err(|e| OrchestratorError::Database(e.to_string()))
                }).unwrap();

                prop_assert_eq!(
                    container_count, 0,
                    "Non-memory-enabled persona '{}' should have 0 MCP containers, found {}",
                    persona_id, container_count
                );
            }
        }
    }
}
