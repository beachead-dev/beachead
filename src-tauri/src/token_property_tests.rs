//! Property-based tests for bearer token generation and persistence.
//!
//! Property 14: Bearer token generation and persistence
//! - For any set of generated bearer tokens, each token SHALL be unique
//! - Each token SHALL have sufficient length (≥ 32 bytes of entropy)
//! - Persisting then reading back the token SHALL produce the same value
//!
//! **Validates: Requirements 17.1, 17.3**

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rusqlite::params;
    use std::collections::HashSet;
    use std::sync::Arc;

    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;

    use crate::db::Database;
    use crate::error::OrchestratorError;
    use crate::token::generate_bearer_token;

    /// Strategy for generating the number of tokens to create (50-200).
    fn token_count_strategy() -> impl Strategy<Value = usize> {
        50usize..=200
    }

    /// Set up an in-memory database with prerequisite records for mcp_containers FK chain.
    fn setup_db() -> Arc<Database> {
        let db = Arc::new(Database::open_in_memory().expect("Failed to open in-memory db"));

        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('at1', 'test-agent', 'test', 0, '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('p1', 'test-persona', 'at1', '/tmp', 1, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                [],
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;

            Ok(())
        })
        .unwrap();

        db
    }

    proptest! {
        /// Property 14: All generated tokens in a set are unique (no collisions).
        ///
        /// **Validates: Requirements 17.1, 17.3**
        #[test]
        fn prop_token_uniqueness(count in token_count_strategy()) {
            let tokens: Vec<String> = (0..count).map(|_| generate_bearer_token()).collect();
            let unique: HashSet<&String> = tokens.iter().collect();

            prop_assert_eq!(
                unique.len(),
                tokens.len(),
                "Generated {} tokens but only {} are unique — collision detected",
                tokens.len(),
                unique.len()
            );
        }

        /// Property 14: Each generated token decodes to at least 32 bytes of entropy.
        ///
        /// **Validates: Requirements 17.1, 17.3**
        #[test]
        fn prop_token_sufficient_entropy(count in token_count_strategy()) {
            for _ in 0..count {
                let token = generate_bearer_token();
                let decoded = URL_SAFE_NO_PAD
                    .decode(&token)
                    .expect("Token should be valid base64url");

                prop_assert!(
                    decoded.len() >= 32,
                    "Token decoded to {} bytes, expected at least 32 bytes of entropy",
                    decoded.len()
                );
            }
        }

        /// Property 14: Persisting a token to mcp_containers and reading it back
        /// produces the exact same value (round-trip integrity).
        ///
        /// **Validates: Requirements 17.1, 17.3**
        #[test]
        fn prop_token_persistence_round_trip(count in 1usize..=50) {
            let db = setup_db();

            // Generate tokens and insert them into mcp_containers
            let mut tokens: Vec<(String, String)> = Vec::new(); // (container_id, token)
            for i in 0..count {
                let token = generate_bearer_token();
                let container_id = format!("mc-prop-{}", i);
                let port = 9300 + i as i64;

                db.with_conn(|conn| {
                    conn.execute(
                        "INSERT INTO mcp_containers (id, persona_id, port, bearer_token, volume_name, status, created_at, updated_at)
                         VALUES (?1, 'p1', ?2, ?3, ?4, 'running', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                        params![container_id, port, token, format!("vol-{}", i)],
                    ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
                    Ok(())
                }).unwrap();

                tokens.push((container_id, token));
            }

            // Read back each token and verify it matches exactly
            for (container_id, expected_token) in &tokens {
                let read_back: String = db.with_conn(|conn| {
                    conn.query_row(
                        "SELECT bearer_token FROM mcp_containers WHERE id = ?1",
                        params![container_id],
                        |row| row.get(0),
                    ).map_err(|e| OrchestratorError::Database(e.to_string()))
                }).unwrap();

                prop_assert_eq!(
                    &read_back,
                    expected_token,
                    "Token round-trip failed for container '{}': persisted '{}' but read back '{}'",
                    container_id,
                    expected_token,
                    read_back
                );
            }
        }
    }
}
