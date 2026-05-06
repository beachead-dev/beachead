//! Property-based tests for export/import functionality.
//!
//! Property 15: Export/import encryption round-trip
//! - encrypt → decrypt with same password produces original data
//! - decrypt with wrong password fails
//! - decrypt corrupted ciphertext fails
//!
//! **Validates: Requirements 20.3, 20.6, 20.8**
//!
//! Property 16: Export data completeness and secret exclusion
//! - export includes all config data and secret service names
//! - export excludes all secret values
//!
//! **Validates: Requirements 20.1, 20.4, 20.5, 20.9**
//!
//! Property 17: Import conflict detection and resolution
//! - conflicts detected and reported
//! - rename creates with new name
//! - skip leaves existing unchanged
//! - overwrite replaces
//! - non-existent workspaces flagged
//!
//! **Validates: Requirements 20.10, 20.11**

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use std::collections::HashMap;
    use std::sync::Arc;

    use crate::db::Database;
    use crate::error::OrchestratorError;
    use crate::export_import_manager::{
        ConflictAction, ConflictResolutions, ExportImportManager,
    };

    // Argon2id key derivation is intentionally slow, so we limit test cases.
    const CRYPTO_CASES: u32 = 10;
    const DB_CASES: u32 = 15;

    // --- Strategies ---

    /// Strategy for generating random plaintext data (1 to 1024 bytes).
    fn plaintext_strategy() -> impl Strategy<Value = Vec<u8>> {
        prop::collection::vec(any::<u8>(), 1..1024)
    }

    /// Strategy for generating random password strings (1 to 32 chars, printable ASCII).
    fn password_strategy() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9!@#$%]{1,32}"
    }

    /// Strategy for generating a persona name (alphanumeric + hyphens, 1-20 chars).
    fn persona_name_strategy() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9\\-]{0,19}"
    }

    /// Strategy for generating an agent name (alphanumeric, 1-15 chars).
    fn agent_name_strategy() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9]{0,14}"
    }

    /// Strategy for generating secret service names.
    fn secret_name_strategy() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_]{0,10}"
    }

    /// Strategy for generating workspace paths (non-existent paths for testing).
    fn workspace_path_strategy() -> impl Strategy<Value = String> {
        "/tmp/beachead-test-[a-z0-9]{4,8}"
    }

    // --- Helper functions ---

    /// Set up a database with an agent type and return the agent type id.
    fn setup_db_with_agent(
        db: &Database,
        agent_name: &str,
        secrets: &[String],
    ) -> String {
        let agent_id = uuid::Uuid::new_v4().to_string();
        let secrets_json: Vec<String> = secrets.iter().map(|s| format!("\"{}\"", s)).collect();
        let metadata = format!(
            "{{\"required_secrets\":[{}],\"auth_methods\":[\"api_key\"],\"description\":\"Test agent\",\"supports_interactive_auth\":false}}",
            secrets_json.join(",")
        );

        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 0, ?4, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                rusqlite::params![agent_id, agent_name, agent_name, metadata],
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })
        .unwrap();

        agent_id
    }

    /// Insert a persona into the database.
    fn insert_persona_raw(
        db: &Database,
        persona_name: &str,
        agent_type_id: &str,
        workspace_path: &str,
    ) -> String {
        let persona_id = uuid::Uuid::new_v4().to_string();

        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, agent_cli_args, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, '[]', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                rusqlite::params![persona_id, persona_name, agent_type_id, workspace_path],
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })
        .unwrap();

        persona_id
    }

    /// Insert an MCP container record (bearer_token excluded from export).
    fn insert_mcp_container(
        db: &Database,
        persona_id: &str,
        port: i64,
    ) {
        let container_id = uuid::Uuid::new_v4().to_string();

        db.with_conn(|conn| {
            conn.execute(
                "INSERT INTO mcp_containers (id, persona_id, port, bearer_token, volume_name, status, created_at, updated_at)
                 VALUES (?1, ?2, ?3, 'secret-bearer-token-value', ?4, 'running', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                rusqlite::params![container_id, persona_id, port, format!("vol-{}", container_id)],
            )
            .map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        })
        .unwrap();
    }

    // =========================================================================
    // Property 15: Export/import encryption round-trip
    // =========================================================================

    mod property_15 {
        use super::*;
        use crate::export_import_manager::{encrypt_data, decrypt_data};

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(CRYPTO_CASES))]

            /// Property 15: encrypt → decrypt with same password produces original data.
            ///
            /// **Validates: Requirements 20.3, 20.6**
            #[test]
            fn prop_encrypt_decrypt_roundtrip(
                plaintext in plaintext_strategy(),
                password in password_strategy(),
            ) {
                let encrypted = encrypt_data(&plaintext, &password).unwrap();
                let decrypted = decrypt_data(&encrypted, &password).unwrap();
                prop_assert_eq!(&decrypted, &plaintext,
                    "Decrypted data must match original plaintext");
            }

            /// Property 15: decrypt with wrong password fails.
            ///
            /// **Validates: Requirements 20.8**
            #[test]
            fn prop_decrypt_wrong_password_fails(
                plaintext in plaintext_strategy(),
                correct_password in password_strategy(),
                wrong_password in password_strategy(),
            ) {
                // Only test when passwords are actually different
                prop_assume!(correct_password != wrong_password);

                let encrypted = encrypt_data(&plaintext, &correct_password).unwrap();
                let result = decrypt_data(&encrypted, &wrong_password);

                prop_assert!(result.is_err(),
                    "Decryption with wrong password must fail");
                match result.unwrap_err() {
                    OrchestratorError::DecryptionFailed(_) => {}
                    other => prop_assert!(false,
                        "Expected DecryptionFailed, got: {:?}", other),
                }
            }

            /// Property 15: decrypt corrupted ciphertext fails.
            ///
            /// **Validates: Requirements 20.8**
            #[test]
            fn prop_decrypt_corrupted_ciphertext_fails(
                plaintext in plaintext_strategy(),
                password in password_strategy(),
                corruption_offset in 0usize..1024,
                corruption_byte in any::<u8>(),
            ) {
                let encrypted = encrypt_data(&plaintext, &password).unwrap();

                // Corrupt a byte in the ciphertext portion (after salt + nonce = 28 bytes)
                let ciphertext_start = 28; // SALT_LEN(16) + NONCE_LEN(12)
                if encrypted.len() > ciphertext_start {
                    let mut corrupted = encrypted.clone();
                    let idx = ciphertext_start + (corruption_offset % (corrupted.len() - ciphertext_start));
                    // Only test if we actually change the byte
                    if corrupted[idx] != corruption_byte {
                        corrupted[idx] = corruption_byte;
                        let result = decrypt_data(&corrupted, &password);
                        prop_assert!(result.is_err(),
                            "Decryption of corrupted ciphertext must fail");
                    }
                }
            }
        }
    }

    // =========================================================================
    // Property 16: Export data completeness and secret exclusion
    // =========================================================================

    mod property_16 {
        use super::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(DB_CASES))]

            /// Property 16: Export includes all config data and secret service names;
            /// export excludes all secret values.
            ///
            /// **Validates: Requirements 20.1, 20.4, 20.5, 20.9**
            #[test]
            fn prop_export_completeness_and_secret_exclusion(
                agent_name in agent_name_strategy(),
                persona_names in prop::collection::vec(persona_name_strategy(), 1..4),
                secret_names in prop::collection::vec(secret_name_strategy(), 1..3),
                workspace_path in workspace_path_strategy(),
            ) {
                // Ensure unique persona names by appending index
                let mut unique_names: Vec<String> = Vec::new();
                for (i, name) in persona_names.iter().enumerate() {
                    let unique = format!("{}-{}", name, i);
                    unique_names.push(unique);
                }

                let db = Arc::new(Database::open_in_memory().unwrap());
                let agent_id = setup_db_with_agent(&db, &agent_name, &secret_names);

                // Insert personas
                let mut persona_ids = Vec::new();
                for name in &unique_names {
                    let pid = insert_persona_raw(&db, name, &agent_id, &workspace_path);
                    persona_ids.push(pid);
                }

                // Insert an MCP container for the first persona
                if !persona_ids.is_empty() {
                    insert_mcp_container(&db, &persona_ids[0], 9100);
                }

                // Export
                let manager = ExportImportManager::new(db.clone());
                let password = "test-export-password";
                let exported = manager.export(password).unwrap();

                // Decrypt and preview on a fresh DB to verify contents
                let db2 = Arc::new(Database::open_in_memory().unwrap());
                let manager2 = ExportImportManager::new(db2.clone());
                let preview = manager2.preview_import(&exported, password).unwrap();

                // Verify all personas are present
                for name in &unique_names {
                    prop_assert!(
                        preview.personas.iter().any(|p| p.name == *name),
                        "Exported data must include persona '{}'", name
                    );
                }

                // Verify agent is present
                prop_assert!(
                    preview.agents.iter().any(|a| a.name == agent_name),
                    "Exported data must include agent '{}'", agent_name
                );

                // Verify secret service names are listed
                for secret in &secret_names {
                    prop_assert!(
                        preview.missing_secrets.contains(secret),
                        "Exported data must include secret service name '{}'", secret
                    );
                }

                // Verify no secret values in the exported bytes (encrypted, so shouldn't be readable)
                let exported_str = String::from_utf8_lossy(&exported);
                prop_assert!(
                    !exported_str.contains("secret-bearer-token-value"),
                    "Exported encrypted data must not contain bearer token in plaintext"
                );

                // Decrypt the export and check the JSON payload doesn't contain bearer_token values
                let decrypted = crate::export_import_manager::decrypt_data(&exported, password).unwrap();
                let payload_str = String::from_utf8_lossy(&decrypted);
                prop_assert!(
                    !payload_str.contains("secret-bearer-token-value"),
                    "Decrypted export payload must not contain bearer token values"
                );
            }
        }
    }

    // =========================================================================
    // Property 17: Import conflict detection and resolution
    // =========================================================================

    mod property_17 {
        use super::*;

        proptest! {
            #![proptest_config(ProptestConfig::with_cases(DB_CASES))]

            /// Property 17: Conflicts are detected and reported when persona names collide.
            ///
            /// **Validates: Requirements 20.10**
            #[test]
            fn prop_conflict_detection(
                persona_name in persona_name_strategy(),
                agent_name in agent_name_strategy(),
            ) {
                let db = Arc::new(Database::open_in_memory().unwrap());
                let agent_id = setup_db_with_agent(&db, &agent_name, &[]);

                // Insert a persona
                insert_persona_raw(&db, &persona_name, &agent_id, "/tmp");

                // Export
                let manager = ExportImportManager::new(db.clone());
                let password = "test";
                let exported = manager.export(password).unwrap();

                // Preview import into the same DB (should detect conflict)
                let preview = manager.preview_import(&exported, password).unwrap();

                prop_assert!(
                    !preview.conflicts.is_empty(),
                    "Must detect conflict for persona name '{}'", persona_name
                );

                // Verify the conflict references the correct name
                let has_conflict = preview.conflicts.iter().any(|c| {
                    match c {
                        crate::export_import_manager::Conflict::PersonaNameConflict {
                            imported_name, ..
                        } => imported_name == &persona_name,
                    }
                });
                prop_assert!(has_conflict,
                    "Conflict must reference persona name '{}'", persona_name);
            }

            /// Property 17: Skip resolution leaves existing persona unchanged.
            ///
            /// **Validates: Requirements 20.10**
            #[test]
            fn prop_skip_leaves_existing_unchanged(
                persona_name in persona_name_strategy(),
                agent_name in agent_name_strategy(),
            ) {
                let db = Arc::new(Database::open_in_memory().unwrap());
                let agent_id = setup_db_with_agent(&db, &agent_name, &[]);

                // Insert a persona
                let original_id = insert_persona_raw(&db, &persona_name, &agent_id, "/tmp/original");

                // Export
                let manager = ExportImportManager::new(db.clone());
                let password = "test";
                let exported = manager.export(password).unwrap();

                // Import with skip resolution
                let mut resolutions_map = HashMap::new();
                resolutions_map.insert(persona_name.clone(), ConflictAction::Skip);
                let resolutions = ConflictResolutions {
                    persona_resolutions: resolutions_map,
                };

                let summary = manager.import(&exported, password, &resolutions).unwrap();

                prop_assert_eq!(summary.personas_skipped, 1,
                    "Skip resolution must result in 1 skipped persona");
                prop_assert_eq!(summary.personas_imported, 0,
                    "Skip resolution must not import the persona");

                // Verify original persona still exists with original workspace
                db.with_conn(|conn| {
                    let ws: String = conn.query_row(
                        "SELECT workspace_path FROM personas WHERE id = ?1",
                        rusqlite::params![original_id],
                        |row| row.get(0),
                    ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
                    assert_eq!(ws, "/tmp/original");
                    Ok(())
                }).unwrap();
            }

            /// Property 17: Rename resolution creates persona with new name.
            ///
            /// **Validates: Requirements 20.10**
            #[test]
            fn prop_rename_creates_with_new_name(
                persona_name in persona_name_strategy(),
                new_name_suffix in "[a-z]{3,8}",
                agent_name in agent_name_strategy(),
            ) {
                let new_name = format!("{}-{}", persona_name, new_name_suffix);

                let db = Arc::new(Database::open_in_memory().unwrap());
                let agent_id = setup_db_with_agent(&db, &agent_name, &[]);

                // Insert a persona (creates the conflict)
                insert_persona_raw(&db, &persona_name, &agent_id, "/tmp/original");

                // Export
                let manager = ExportImportManager::new(db.clone());
                let password = "test";
                let exported = manager.export(password).unwrap();

                // Import with rename resolution
                let mut resolutions_map = HashMap::new();
                resolutions_map.insert(
                    persona_name.clone(),
                    ConflictAction::Rename { new_name: new_name.clone() },
                );
                let resolutions = ConflictResolutions {
                    persona_resolutions: resolutions_map,
                };

                let summary = manager.import(&exported, password, &resolutions).unwrap();

                prop_assert_eq!(summary.personas_imported, 1,
                    "Rename resolution must import 1 persona");

                // Verify the new name exists in the database
                db.with_conn(|conn| {
                    let exists: bool = conn.query_row(
                        "SELECT COUNT(*) > 0 FROM personas WHERE name = ?1",
                        rusqlite::params![new_name],
                        |row| row.get(0),
                    ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
                    assert!(exists, "Renamed persona '{}' must exist in DB", new_name);
                    Ok(())
                }).unwrap();
            }

            /// Property 17: Overwrite resolution replaces existing persona.
            ///
            /// **Validates: Requirements 20.10**
            #[test]
            fn prop_overwrite_replaces_existing(
                persona_name in persona_name_strategy(),
                agent_name in agent_name_strategy(),
            ) {
                let db = Arc::new(Database::open_in_memory().unwrap());
                let agent_id = setup_db_with_agent(&db, &agent_name, &[]);

                // Insert a persona
                insert_persona_raw(&db, &persona_name, &agent_id, "/tmp/original");

                // Export
                let manager = ExportImportManager::new(db.clone());
                let password = "test";
                let exported = manager.export(password).unwrap();

                // Import with overwrite resolution
                let mut resolutions_map = HashMap::new();
                resolutions_map.insert(persona_name.clone(), ConflictAction::Overwrite);
                let resolutions = ConflictResolutions {
                    persona_resolutions: resolutions_map,
                };

                let summary = manager.import(&exported, password, &resolutions).unwrap();

                prop_assert_eq!(summary.personas_imported, 1,
                    "Overwrite resolution must import 1 persona");

                // Verify the persona name still exists in the database
                db.with_conn(|conn| {
                    let count: i64 = conn.query_row(
                        "SELECT COUNT(*) FROM personas WHERE name = ?1",
                        rusqlite::params![persona_name],
                        |row| row.get(0),
                    ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
                    assert!(count >= 1, "Persona '{}' must exist after overwrite", persona_name);
                    Ok(())
                }).unwrap();
            }

            /// Property 17: Non-existent workspace paths are flagged with warnings.
            ///
            /// **Validates: Requirements 20.11**
            #[test]
            fn prop_nonexistent_workspaces_flagged(
                persona_name in persona_name_strategy(),
                agent_name in agent_name_strategy(),
                workspace_path in "/nonexistent/path/[a-z0-9]{4,8}",
            ) {
                let db = Arc::new(Database::open_in_memory().unwrap());
                let agent_id = setup_db_with_agent(&db, &agent_name, &[]);

                // Insert a persona with a non-existent workspace path
                insert_persona_raw(&db, &persona_name, &agent_id, &workspace_path);

                // Export
                let manager = ExportImportManager::new(db.clone());
                let password = "test";
                let exported = manager.export(password).unwrap();

                // Preview import on a fresh DB
                let db2 = Arc::new(Database::open_in_memory().unwrap());
                let manager2 = ExportImportManager::new(db2.clone());
                let preview = manager2.preview_import(&exported, password).unwrap();

                // Verify the non-existent workspace is flagged
                prop_assert!(
                    preview.invalid_workspaces.iter().any(|w| {
                        w.persona_name == persona_name && w.workspace_path == workspace_path
                    }),
                    "Non-existent workspace '{}' for persona '{}' must be flagged",
                    workspace_path, persona_name
                );
            }
        }
    }
}
