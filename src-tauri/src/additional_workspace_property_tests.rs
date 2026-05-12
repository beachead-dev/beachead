//! Property-based tests for additional workspace operations.
//! Uses proptest to generate arbitrary domain objects and verify correctness properties.

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use proptest::prelude::*;
    use std::path::PathBuf;

    use crate::db::Database;
    use crate::db_ops::*;
    use crate::error::OrchestratorError;
    use crate::persona_manager::validate_additional_workspaces;
    use crate::types::*;

    fn temp_workspace() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

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

    // Feature: multi-workspace-mounts, Property 2: Non-absolute paths are rejected
    // **Validates: Requirements 2.1, 2.3**

    /// Generate a random relative path (does NOT start with `/`)
    fn arb_relative_path() -> impl Strategy<Value = PathBuf> {
        // Generate path segments that form a relative path (no leading `/`)
        prop::string::string_regex("[a-zA-Z0-9_.][a-zA-Z0-9_./\\-]{0,50}")
            .unwrap()
            .prop_filter("must not start with /", |s| !s.starts_with('/'))
            .prop_filter("must not be empty", |s| !s.is_empty())
            .prop_map(PathBuf::from)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_non_absolute_paths_are_rejected(
            relative_path in arb_relative_path(),
            read_only in any::<bool>(),
            label in arb_label(),
        ) {
            // Ensure the generated path is indeed not absolute
            prop_assume!(!relative_path.is_absolute());

            let entries = vec![CreateAdditionalWorkspaceEntry {
                path: relative_path.clone(),
                read_only,
                label,
            }];

            // Use /tmp as the primary workspace (a valid existing absolute path)
            let primary = std::path::Path::new("/tmp");
            let result = validate_additional_workspaces(&entries, primary);

            // Validation must return an error
            prop_assert!(result.is_err(), "Expected validation error for relative path: {:?}", relative_path);

            // The error message must contain "Additional workspace path must be absolute"
            match result {
                Err(OrchestratorError::Validation(msg)) => {
                    prop_assert!(
                        msg.contains("Additional workspace path must be absolute"),
                        "Error message should contain 'Additional workspace path must be absolute', got: {}",
                        msg
                    );
                }
                Err(other) => {
                    prop_assert!(false, "Expected Validation error, got: {:?}", other);
                }
                Ok(_) => {
                    prop_assert!(false, "Expected error for relative path {:?}, but got Ok", relative_path);
                }
            }
        }
    }

    // Feature: multi-workspace-mounts, Property 3: Stored paths are canonicalized
    // **Validates: Requirements 2.7**

    /// Generate a subdirectory name for use in path construction
    fn arb_subdir_name() -> impl Strategy<Value = String> {
        "[a-zA-Z][a-zA-Z0-9_]{0,9}".prop_map(|s| s)
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_stored_paths_are_canonicalized(
            subdir_name in arb_subdir_name(),
            read_only in any::<bool>(),
            label in arb_label(),
        ) {
            // Create a temp directory to serve as the additional workspace
            let temp_dir = tempfile::tempdir().unwrap();
            // Create a subdirectory inside the temp dir
            let subdir = temp_dir.path().join(&subdir_name);
            std::fs::create_dir_all(&subdir).unwrap();

            // Build a path with `..` segments that resolves back to the subdir
            // e.g., /tmp/somedir/subdir/../subdir
            let path_with_dotdot = subdir.join("..").join(&subdir_name);

            // Create a separate temp directory for the primary workspace
            let primary_dir = tempfile::tempdir().unwrap();

            let entries = vec![CreateAdditionalWorkspaceEntry {
                path: path_with_dotdot.clone(),
                read_only,
                label,
            }];

            let result = validate_additional_workspaces(&entries, primary_dir.path());

            // Validation should succeed
            prop_assert!(result.is_ok(), "Expected Ok for valid path with .. segments, got: {:?}", result.err());

            let validation_result = result.unwrap();

            // The canonical path from validation should equal std::fs::canonicalize() of the input
            let expected_canonical = std::fs::canonicalize(&path_with_dotdot).unwrap();
            prop_assert_eq!(
                &validation_result.canonical_paths[0],
                &expected_canonical,
                "Stored path should equal canonicalize() result. Input: {:?}, Got: {:?}, Expected: {:?}",
                path_with_dotdot,
                validation_result.canonical_paths[0],
                expected_canonical
            );
        }
    }

    // Feature: multi-workspace-mounts, Property 11: Label validation
    // **Validates: Requirements 10.6, 10.7**

    /// Generate a string with length between 65 and 200 characters (exceeds 64-char limit)
    fn arb_long_label() -> impl Strategy<Value = String> {
        prop::collection::vec(prop::char::range('a', 'z'), 65..=200)
            .prop_map(|chars| chars.into_iter().collect::<String>())
    }

    /// Generate a string containing at least one control character (ASCII 0x00–0x1F)
    fn arb_label_with_control_chars() -> impl Strategy<Value = String> {
        // Generate a prefix of printable chars, a control char, and a suffix of printable chars
        (
            prop::collection::vec(prop::char::range('a', 'z'), 0..30),
            prop::char::range('\x00', '\x1F'),
            prop::collection::vec(prop::char::range('a', 'z'), 0..30),
        )
            .prop_map(|(prefix, ctrl, suffix)| {
                let mut s: String = prefix.into_iter().collect();
                s.push(ctrl);
                s.extend(suffix.into_iter());
                s
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_labels_exceeding_64_chars_are_rejected(
            long_label in arb_long_label(),
            read_only in any::<bool>(),
        ) {
            // Verify the generated label is indeed > 64 chars
            prop_assume!(long_label.len() > 64);

            let primary = temp_workspace();
            let additional = temp_workspace();

            let entries = vec![CreateAdditionalWorkspaceEntry {
                path: additional.path().to_path_buf(),
                read_only,
                label: Some(long_label.clone()),
            }];

            let result = validate_additional_workspaces(&entries, primary.path());

            prop_assert!(result.is_err(), "Expected validation error for label with {} chars", long_label.len());

            match result {
                Err(OrchestratorError::Validation(msg)) => {
                    prop_assert!(
                        msg.contains("Label exceeds maximum length of 64 characters"),
                        "Error message should contain 'Label exceeds maximum length of 64 characters', got: {}",
                        msg
                    );
                }
                Err(other) => {
                    prop_assert!(false, "Expected Validation error, got: {:?}", other);
                }
                Ok(_) => {
                    prop_assert!(false, "Expected error for long label (len={}), but got Ok", long_label.len());
                }
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_labels_with_control_chars_are_rejected(
            label_with_ctrl in arb_label_with_control_chars(),
            read_only in any::<bool>(),
        ) {
            // Verify the generated label contains at least one control character
            prop_assume!(label_with_ctrl.chars().any(|c| (c as u32) < 0x20));

            let primary = temp_workspace();
            let additional = temp_workspace();

            let entries = vec![CreateAdditionalWorkspaceEntry {
                path: additional.path().to_path_buf(),
                read_only,
                label: Some(label_with_ctrl.clone()),
            }];

            let result = validate_additional_workspaces(&entries, primary.path());

            prop_assert!(result.is_err(), "Expected validation error for label with control chars");

            match result {
                Err(OrchestratorError::Validation(msg)) => {
                    prop_assert!(
                        msg.contains("Label contains invalid control characters"),
                        "Error message should contain 'Label contains invalid control characters', got: {}",
                        msg
                    );
                }
                Err(other) => {
                    prop_assert!(false, "Expected Validation error, got: {:?}", other);
                }
                Ok(_) => {
                    prop_assert!(false, "Expected error for label with control chars, but got Ok");
                }
            }
        }
    }

    // Feature: multi-workspace-mounts, Property 4: Paths containing null bytes are rejected
    // **Validates: Requirements 2.8**

    /// Generate a string that contains at least one null byte, then make it an absolute path.
    fn arb_path_with_null_bytes() -> impl Strategy<Value = PathBuf> {
        (
            prop::string::string_regex("[a-zA-Z0-9_/.-]{0,20}").unwrap(),
            prop::string::string_regex("[a-zA-Z0-9_/.-]{0,20}").unwrap(),
        )
            .prop_map(|(prefix, suffix)| {
                // Build a path with at least one embedded null byte, made absolute
                let path_str = format!("/{}\0{}", prefix, suffix);
                PathBuf::from(path_str)
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_paths_containing_null_bytes_are_rejected(
            path_with_null in arb_path_with_null_bytes(),
            read_only in any::<bool>(),
            label in arb_label(),
        ) {
            // Verify the generated path actually contains a null byte
            let path_str = path_with_null.to_string_lossy();
            prop_assume!(path_str.contains('\0'));

            let entries = vec![CreateAdditionalWorkspaceEntry {
                path: path_with_null.clone(),
                read_only,
                label,
            }];

            // Use /tmp as the primary workspace
            let primary = std::path::Path::new("/tmp");
            let result = validate_additional_workspaces(&entries, primary);

            // Validation must return an error
            prop_assert!(result.is_err(), "Expected validation error for path with null bytes: {:?}", path_with_null);

            // The error message must contain "null bytes"
            match result {
                Err(OrchestratorError::Validation(msg)) => {
                    prop_assert!(
                        msg.contains("null bytes"),
                        "Error message should contain 'null bytes', got: {}",
                        msg
                    );
                }
                Err(other) => {
                    prop_assert!(false, "Expected Validation error, got: {:?}", other);
                }
                Ok(_) => {
                    prop_assert!(false, "Expected error for path with null bytes {:?}, but got Ok", path_with_null);
                }
            }
        }
    }

    // Feature: multi-workspace-mounts, Property 7: Duplicate canonicalized paths are rejected
    // **Validates: Requirements 7.1, 7.3, 7.5**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_duplicate_canonicalized_paths_are_rejected(
            extra_count in 0usize..5,
            read_only_flags in prop::collection::vec(any::<bool>(), 2..10),
            labels in prop::collection::vec(arb_label(), 2..10),
            use_dot_dot_variant in any::<bool>(),
        ) {
            // Create a real temp directory so paths exist and can be canonicalized
            let temp_dir = tempfile::tempdir().unwrap();
            let base_path = temp_dir.path().to_path_buf();

            // Create a subdirectory to use as the duplicate workspace
            let dup_dir = base_path.join("workspace_dup");
            std::fs::create_dir_all(&dup_dir).unwrap();

            // Create a primary workspace that is different from the duplicate
            let primary_dir = base_path.join("primary");
            std::fs::create_dir_all(&primary_dir).unwrap();

            // Create additional unique directories for non-duplicate entries
            let mut unique_dirs: Vec<PathBuf> = Vec::new();
            for i in 0..extra_count {
                let dir = base_path.join(format!("unique_{}", i));
                std::fs::create_dir_all(&dir).unwrap();
                unique_dirs.push(dir);
            }

            // Build the entries list with at least two entries pointing to the same canonicalized path
            let mut entries: Vec<CreateAdditionalWorkspaceEntry> = Vec::new();

            // First entry: the duplicate path directly
            entries.push(CreateAdditionalWorkspaceEntry {
                path: dup_dir.clone(),
                read_only: *read_only_flags.get(0).unwrap_or(&false),
                label: labels.get(0).cloned().unwrap_or(None),
            });

            // Add unique entries in between (to test that duplicates are detected regardless of position)
            for (i, unique_path) in unique_dirs.iter().enumerate() {
                entries.push(CreateAdditionalWorkspaceEntry {
                    path: unique_path.clone(),
                    read_only: *read_only_flags.get(i + 1).unwrap_or(&false),
                    label: labels.get(i + 1).cloned().unwrap_or(None),
                });
            }

            // Second entry: same path but potentially via `..` to test canonicalization
            let duplicate_path = if use_dot_dot_variant {
                // Create a child dir so we can use `..` to resolve back to dup_dir
                let child = dup_dir.join("child");
                std::fs::create_dir_all(&child).unwrap();
                child.join("..")
            } else {
                // Same path repeated directly
                dup_dir.clone()
            };

            let last_idx = entries.len();
            entries.push(CreateAdditionalWorkspaceEntry {
                path: duplicate_path,
                read_only: *read_only_flags.get(last_idx).unwrap_or(&false),
                label: labels.get(last_idx).cloned().unwrap_or(None),
            });

            // Call validate_additional_workspaces
            let result = validate_additional_workspaces(&entries, &primary_dir);

            // Validation must return an error
            prop_assert!(result.is_err(), "Expected validation error for duplicate paths, but got Ok");

            // The error message must contain "Duplicate additional workspace path"
            match result {
                Err(OrchestratorError::Validation(msg)) => {
                    prop_assert!(
                        msg.contains("Duplicate additional workspace path"),
                        "Error message should contain 'Duplicate additional workspace path', got: {}",
                        msg
                    );
                }
                Err(other) => {
                    prop_assert!(false, "Expected Validation error, got: {:?}", other);
                }
                Ok(_) => unreachable!(),
            }
        }
    }

    // Feature: multi-workspace-mounts, Property 5: Update replaces all additional workspaces
    // **Validates: Requirements 3.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_update_replaces_all_additional_workspaces(
            agent in arb_agent_type(),
            persona_name in arb_name(),
            initial_count in 0usize..10,
            update_count in 0usize..10,
            initial_read_only in prop::collection::vec(any::<bool>(), 0..10),
            update_read_only in prop::collection::vec(any::<bool>(), 0..10),
            initial_labels in prop::collection::vec(arb_label(), 0..10),
            update_labels in prop::collection::vec(arb_label(), 0..10),
        ) {
            use crate::persona_manager::PersonaManager;
            use std::sync::Arc;

            // Ensure N != M so we can verify the replacement actually happened
            prop_assume!(initial_count != update_count);

            // Create temp directories for primary workspace and all additional workspaces
            let primary_dir = tempfile::tempdir().unwrap();

            // Create N initial workspace directories
            let mut initial_dirs: Vec<tempfile::TempDir> = Vec::new();
            for _ in 0..initial_count {
                initial_dirs.push(tempfile::tempdir().unwrap());
            }

            // Create M update workspace directories (distinct from initial ones)
            let mut update_dirs: Vec<tempfile::TempDir> = Vec::new();
            for _ in 0..update_count {
                update_dirs.push(tempfile::tempdir().unwrap());
            }

            // Set up in-memory database and PersonaManager
            let db = Arc::new(Database::open_in_memory().unwrap());
            let pm = PersonaManager::new(db.clone());

            // Insert the agent type
            db.with_conn(|conn| {
                insert_agent_type(conn, &agent)
            }).unwrap();

            // Build initial additional workspaces entries
            let n = initial_count.min(initial_read_only.len()).min(initial_labels.len());
            let initial_entries: Vec<CreateAdditionalWorkspaceEntry> = (0..n)
                .map(|i| CreateAdditionalWorkspaceEntry {
                    path: initial_dirs[i].path().to_path_buf(),
                    read_only: initial_read_only[i],
                    label: initial_labels[i].clone(),
                })
                .collect();

            // Create persona with N additional workspaces
            let create_req = CreatePersonaRequest {
                name: persona_name,
                agent_type_id: agent.id.clone(),
                workspace_path: primary_dir.path().to_path_buf(),
                memory_enabled: None,
                agent_cli_args: None,
                mcp_servers: None,
                additional_workspaces: Some(initial_entries),
            };

            let persona = pm.create(create_req).unwrap();

            // Verify initial state: N workspaces stored
            prop_assert_eq!(
                persona.additional_workspaces.len(), n,
                "After create, should have {} additional workspaces", n
            );

            // Build update additional workspaces entries
            let m = update_count.min(update_read_only.len()).min(update_labels.len());
            let update_entries: Vec<CreateAdditionalWorkspaceEntry> = (0..m)
                .map(|i| CreateAdditionalWorkspaceEntry {
                    path: update_dirs[i].path().to_path_buf(),
                    read_only: update_read_only[i],
                    label: update_labels[i].clone(),
                })
                .collect();

            // Update persona with M additional workspaces (replace-all semantics)
            let update_req = UpdatePersonaRequest {
                name: None,
                agent_type_id: None,
                workspace_path: None,
                memory_enabled: None,
                agent_cli_args: None,
                mcp_servers: None,
                additional_workspaces: Some(update_entries),
            };

            let update_result = pm.update(&persona.id, update_req).unwrap();

            // Extract the updated persona from the result
            let updated_persona = match update_result {
                UpdateResult::Applied { persona } => persona,
                UpdateResult::RequiresRestart { persona, .. } => persona,
            };

            // Verify: exactly M workspace records exist after update
            prop_assert_eq!(
                updated_persona.additional_workspaces.len(), m,
                "After update, should have exactly {} additional workspaces, got {}",
                m, updated_persona.additional_workspaces.len()
            );

            // Verify: the old N records are completely gone by checking paths
            // All stored paths should be from the update set, not the initial set
            let initial_canonical_paths: Vec<PathBuf> = initial_dirs.iter()
                .map(|d| std::fs::canonicalize(d.path()).unwrap())
                .collect();

            for ws in &updated_persona.additional_workspaces {
                prop_assert!(
                    !initial_canonical_paths.contains(&ws.path),
                    "Found an old workspace path {:?} that should have been replaced", ws.path
                );
            }

            // Also verify via direct DB query
            let db_workspaces = db.with_conn(|conn| {
                list_additional_workspaces(conn, &persona.id)
            }).unwrap();

            prop_assert_eq!(
                db_workspaces.len(), m,
                "DB should have exactly {} workspace records, got {}",
                m, db_workspaces.len()
            );
        }
    }

    // Feature: multi-workspace-mounts, Property 8: Additional workspace matching primary is rejected
    // **Validates: Requirements 7.2**

    /// Strategy to generate a path variation index (0-3) representing different ways
    /// to express the same path that all resolve to the same canonical location.
    fn arb_path_variation_index() -> impl Strategy<Value = usize> {
        0usize..4
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_additional_workspace_matching_primary_is_rejected(
            variation_index in arb_path_variation_index(),
            read_only in any::<bool>(),
            label in arb_label(),
        ) {
            // Create a real temp directory to use as both primary and additional workspace
            let tmp_dir = tempfile::tempdir().unwrap();
            let primary_path = tmp_dir.path().to_path_buf();

            // Create a subdirectory for `..` segment variations
            let sub_dir = primary_path.join("subdir");
            std::fs::create_dir_all(&sub_dir).unwrap();

            // Generate a path variation that resolves to the same directory as primary
            let additional_path = match variation_index {
                0 => {
                    // Exact same path
                    primary_path.clone()
                }
                1 => {
                    // Path with trailing slash
                    PathBuf::from(format!("{}/", primary_path.display()))
                }
                2 => {
                    // Path with `/.` appended
                    primary_path.join(".")
                }
                3 => {
                    // Path with `subdir/..` appended — resolves back to primary
                    primary_path.join("subdir").join("..")
                }
                _ => unreachable!(),
            };

            let entries = vec![CreateAdditionalWorkspaceEntry {
                path: additional_path.clone(),
                read_only,
                label,
            }];

            let result = validate_additional_workspaces(&entries, &primary_path);

            // Validation must return an error
            prop_assert!(
                result.is_err(),
                "Expected validation error when additional workspace matches primary. \
                 Primary: {:?}, Additional (variation {}): {:?}",
                primary_path, variation_index, additional_path
            );

            // The error message must contain "Additional workspace path matches primary workspace"
            match result {
                Err(OrchestratorError::Validation(msg)) => {
                    prop_assert!(
                        msg.contains("Additional workspace path matches primary workspace"),
                        "Error message should contain 'Additional workspace path matches primary workspace', got: {}",
                        msg
                    );
                }
                Err(other) => {
                    prop_assert!(
                        false,
                        "Expected Validation error, got: {:?}",
                        other
                    );
                }
                Ok(_) => {
                    prop_assert!(
                        false,
                        "Expected error when additional workspace matches primary, but got Ok. \
                         Primary: {:?}, Additional (variation {}): {:?}",
                        primary_path, variation_index, additional_path
                    );
                }
            }
        }
    }

    /// Generate a non-builtin AgentType for export/import tests (non-builtin agents get imported)
    fn arb_non_builtin_agent_type() -> impl Strategy<Value = AgentType> {
        arb_name().prop_map(|name| {
            let now = Utc::now();
            AgentType {
                id: AgentTypeId::new(),
                name,
                sbx_agent: Some("claude".to_string()),
                kit_ref: None,
                is_builtin: false,
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

    // Feature: multi-workspace-mounts, Property 9: Export/import round trip preserves additional workspaces
    // **Validates: Requirements 8.1, 8.2**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(20))]
        #[test]
        fn prop_export_import_round_trip_preserves_additional_workspaces(
            agent in arb_non_builtin_agent_type(),
            persona_name in arb_name(),
            workspace_count in 0usize..10,
            read_only_flags in prop::collection::vec(any::<bool>(), 0..10),
            labels in prop::collection::vec(arb_label(), 0..10),
        ) {
            use crate::export_import_manager::*;
            use std::sync::Arc;
            use std::collections::HashMap;

            // Create temp directories for primary workspace and additional workspaces
            let primary_dir = tempfile::tempdir().unwrap();
            let mut additional_dirs: Vec<tempfile::TempDir> = Vec::new();
            for _ in 0..workspace_count {
                additional_dirs.push(tempfile::tempdir().unwrap());
            }

            let n = workspace_count.min(read_only_flags.len()).min(labels.len()).min(additional_dirs.len());

            // Set up source database
            let source_db = Arc::new(Database::open_in_memory().unwrap());

            // Insert agent type (non-builtin so it gets imported into target DB)
            source_db.with_conn(|conn| {
                insert_agent_type(conn, &agent)
            }).unwrap();

            // Insert persona
            let now = Utc::now();
            let persona_id = PersonaId::new();
            let persona = Persona {
                id: persona_id.clone(),
                name: persona_name.clone(),
                agent_type_id: agent.id.clone(),
                workspace_path: primary_dir.path().to_path_buf(),
                memory_enabled: false,
                agent_cli_args: vec![],
                mcp_servers: vec![],
                additional_workspaces: vec![],
                created_at: now,
                updated_at: now,
            };
            source_db.with_conn(|conn| {
                insert_persona(conn, &persona)
            }).unwrap();

            // Insert N additional workspaces
            let mut original_workspaces: Vec<(PathBuf, bool, i32, Option<String>)> = Vec::new();
            source_db.with_conn(|conn| {
                for i in 0..n {
                    let canonical_path = std::fs::canonicalize(additional_dirs[i].path()).unwrap();
                    let ws = AdditionalWorkspace {
                        id: uuid::Uuid::new_v4().to_string(),
                        persona_id: persona_id.clone(),
                        path: canonical_path.clone(),
                        read_only: read_only_flags[i],
                        position: i as i32,
                        label: labels[i].clone(),
                        created_at: now,
                    };
                    insert_additional_workspace(conn, &ws)?;
                    original_workspaces.push((canonical_path, read_only_flags[i], i as i32, labels[i].clone()));
                }
                Ok(())
            }).unwrap();

            // Export from source database
            let export_manager = ExportImportManager::new(source_db.clone());
            let password = "test-export-password";
            let exported_data = export_manager.export(password).unwrap();

            // Import into a fresh target database (no conflicts)
            let target_db = Arc::new(Database::open_in_memory().unwrap());
            let import_manager = ExportImportManager::new(target_db.clone());

            // Empty resolutions since there should be no conflicts on a fresh DB
            let resolutions = ConflictResolutions {
                persona_resolutions: HashMap::new(),
            };

            let summary = import_manager.import(&exported_data, password, &resolutions).unwrap();
            prop_assert_eq!(summary.personas_imported, 1, "Should import exactly 1 persona");
            prop_assert_eq!(summary.personas_skipped, 0, "Should skip 0 personas");

            // Retrieve the imported persona's additional workspaces from the target DB
            let imported_personas = target_db.with_conn(|conn| {
                list_personas(conn)
            }).unwrap();

            prop_assert_eq!(imported_personas.len(), 1, "Target DB should have exactly 1 persona");
            let imported_persona = &imported_personas[0];

            // Verify the imported persona has the same number of additional workspaces
            prop_assert_eq!(
                imported_persona.additional_workspaces.len(), n,
                "Imported persona should have {} additional workspaces, got {}",
                n, imported_persona.additional_workspaces.len()
            );

            // Verify each workspace matches the original (path, read_only, position, label)
            for (i, imported_ws) in imported_persona.additional_workspaces.iter().enumerate() {
                let (ref orig_path, orig_read_only, orig_position, ref orig_label) = original_workspaces[i];

                prop_assert_eq!(
                    &imported_ws.path, orig_path,
                    "Workspace {} path mismatch: expected {:?}, got {:?}",
                    i, orig_path, imported_ws.path
                );
                prop_assert_eq!(
                    imported_ws.read_only, orig_read_only,
                    "Workspace {} read_only mismatch: expected {}, got {}",
                    i, orig_read_only, imported_ws.read_only
                );
                prop_assert_eq!(
                    imported_ws.position, orig_position,
                    "Workspace {} position mismatch: expected {}, got {}",
                    i, orig_position, imported_ws.position
                );
                prop_assert_eq!(
                    &imported_ws.label, orig_label,
                    "Workspace {} label mismatch: expected {:?}, got {:?}",
                    i, orig_label, imported_ws.label
                );
            }
        }
    }

    // Feature: multi-workspace-mounts, Property 12: Workspace changes with active sessions return RequiresRestart
    // **Validates: Requirements 12.1**
    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_workspace_changes_with_active_sessions_return_requires_restart(
            agent in arb_agent_type(),
            persona_name in arb_name(),
            initial_count in 0usize..5,
            update_count in 0usize..5,
            initial_read_only in prop::collection::vec(any::<bool>(), 0..5),
            update_read_only in prop::collection::vec(any::<bool>(), 0..5),
            initial_labels in prop::collection::vec(arb_label(), 0..5),
            update_labels in prop::collection::vec(arb_label(), 0..5),
        ) {
            use crate::persona_manager::PersonaManager;
            use std::sync::Arc;

            // Create temp directories for primary workspace and all additional workspaces
            let primary_dir = tempfile::tempdir().unwrap();

            // Create initial workspace directories
            let mut initial_dirs: Vec<tempfile::TempDir> = Vec::new();
            for _ in 0..initial_count {
                initial_dirs.push(tempfile::tempdir().unwrap());
            }

            // Create update workspace directories (distinct from initial ones)
            let mut update_dirs: Vec<tempfile::TempDir> = Vec::new();
            for _ in 0..update_count {
                update_dirs.push(tempfile::tempdir().unwrap());
            }

            // Set up in-memory database and PersonaManager
            let db = Arc::new(Database::open_in_memory().unwrap());
            let pm = PersonaManager::new(db.clone());

            // Insert the agent type
            db.with_conn(|conn| {
                insert_agent_type(conn, &agent)
            }).unwrap();

            // Build initial additional workspaces entries
            let n = initial_count.min(initial_read_only.len()).min(initial_labels.len()).min(initial_dirs.len());
            let initial_entries: Vec<CreateAdditionalWorkspaceEntry> = (0..n)
                .map(|i| CreateAdditionalWorkspaceEntry {
                    path: initial_dirs[i].path().to_path_buf(),
                    read_only: initial_read_only[i],
                    label: initial_labels[i].clone(),
                })
                .collect();

            // Create persona with initial additional workspaces (or none)
            let create_req = CreatePersonaRequest {
                name: persona_name,
                agent_type_id: agent.id.clone(),
                workspace_path: primary_dir.path().to_path_buf(),
                memory_enabled: None,
                agent_cli_args: None,
                mcp_servers: None,
                additional_workspaces: if n > 0 { Some(initial_entries) } else { None },
            };

            let persona = pm.create(create_req).unwrap();

            // Insert an active session (status = "running") for this persona
            db.with_conn(|conn| {
                conn.execute(
                    "INSERT INTO sessions (id, persona_id, status, created_at, updated_at) \
                     VALUES (?1, ?2, 'running', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
                    rusqlite::params![
                        uuid::Uuid::new_v4().to_string(),
                        persona.id.0,
                    ],
                ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
                Ok(())
            }).unwrap();

            // Build update additional workspaces entries (different from initial)
            let m = update_count.min(update_read_only.len()).min(update_labels.len()).min(update_dirs.len());
            let update_entries: Vec<CreateAdditionalWorkspaceEntry> = (0..m)
                .map(|i| CreateAdditionalWorkspaceEntry {
                    path: update_dirs[i].path().to_path_buf(),
                    read_only: update_read_only[i],
                    label: update_labels[i].clone(),
                })
                .collect();

            // Update persona with different additional workspaces while session is active
            let update_req = UpdatePersonaRequest {
                name: None,
                agent_type_id: None,
                workspace_path: None,
                memory_enabled: None,
                agent_cli_args: None,
                mcp_servers: None,
                additional_workspaces: Some(update_entries),
            };

            let result = pm.update(&persona.id, update_req).unwrap();

            // Verify the result is RequiresRestart
            match result {
                UpdateResult::RequiresRestart { reason, persona: updated } => {
                    // Verify the reason mentions workspace changes
                    prop_assert!(
                        reason.contains("Workspace") || reason.contains("workspace"),
                        "RequiresRestart reason should mention workspace changes, got: {}",
                        reason
                    );

                    // Verify the update was still saved (changes applied to DB immediately)
                    prop_assert_eq!(
                        updated.additional_workspaces.len(), m,
                        "Updated persona should have {} additional workspaces, got {}",
                        m, updated.additional_workspaces.len()
                    );
                }
                UpdateResult::Applied { .. } => {
                    prop_assert!(
                        false,
                        "Expected RequiresRestart when updating workspaces with active session, got Applied"
                    );
                }
            }
        }
    }

    // Feature: multi-workspace-mounts, Property 6: sbx create argument construction
    // **Validates: Requirements 4.2, 4.3, 4.4, 4.5, 4.6, 13.2**

    /// Generate a random additional workspace arg with a path and read_only flag
    fn arb_additional_workspace_arg() -> impl Strategy<Value = AdditionalWorkspaceArg> {
        (arb_workspace_path(), any::<bool>()).prop_map(|(path, read_only)| {
            AdditionalWorkspaceArg { path, read_only }
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(100))]
        #[test]
        fn prop_sbx_create_argument_construction(
            agent_name in arb_name(),
            primary_workspace in arb_workspace_path(),
            additional_workspaces in prop::collection::vec(arb_additional_workspace_arg(), 0..10),
            kit_count in 0usize..3,
            kit_paths in prop::collection::vec(arb_workspace_path(), 0..3),
            use_name in any::<bool>(),
            sandbox_name in arb_name(),
            use_template in any::<bool>(),
            template_name in arb_name(),
        ) {
            use crate::sbx::{build_create_args, SbxCreateArgs};

            let kit_paths_clamped: Vec<PathBuf> = kit_paths.into_iter().take(kit_count).collect();

            let args = SbxCreateArgs {
                agent: agent_name.clone(),
                kit_paths: kit_paths_clamped.clone(),
                workspace: primary_workspace.clone(),
                name: if use_name { Some(sandbox_name.clone()) } else { None },
                template: if use_template { Some(template_name.clone()) } else { None },
                additional_workspaces: additional_workspaces.clone(),
            };

            let cmd_args = build_create_args(&args);

            // Find the index of the primary workspace in the args vector.
            // It comes after agent, -q, optional --kit pairs, optional --name pair, optional -t pair.
            let primary_ws_str = primary_workspace.to_string_lossy().to_string();

            // The primary workspace must appear in the args
            let primary_idx = cmd_args.iter().position(|a| *a == primary_ws_str);
            prop_assert!(
                primary_idx.is_some(),
                "Primary workspace path '{}' must appear in the args vector. Args: {:?}",
                primary_ws_str, cmd_args
            );
            let primary_idx = primary_idx.unwrap();

            // Everything after the primary workspace index should be the additional workspaces
            let after_primary = &cmd_args[primary_idx + 1..];
            prop_assert_eq!(
                after_primary.len(),
                additional_workspaces.len(),
                "Number of args after primary workspace should equal number of additional workspaces. \
                 Expected {}, got {}. After primary: {:?}",
                additional_workspaces.len(), after_primary.len(), after_primary
            );

            // Verify each additional workspace in position order
            for (i, ws) in additional_workspaces.iter().enumerate() {
                let expected_path_str = ws.path.to_string_lossy().to_string();
                let expected_arg = if ws.read_only {
                    format!("{}:ro", expected_path_str)
                } else {
                    expected_path_str.clone()
                };

                prop_assert_eq!(
                    &after_primary[i], &expected_arg,
                    "Additional workspace at position {} should be '{}', got '{}'. read_only={}",
                    i, expected_arg, after_primary[i], ws.read_only
                );

                // Verify each workspace path is a separate element (not concatenated with others)
                // This is inherently true since each is a separate element in the Vec,
                // but let's verify no element contains multiple paths separated by spaces
                prop_assert!(
                    !after_primary[i].contains(' '),
                    "Workspace arg at position {} should not contain spaces (no concatenation): '{}'",
                    i, after_primary[i]
                );
            }

            // When N=0, only the primary workspace appears (no additional args after it)
            if additional_workspaces.is_empty() {
                prop_assert_eq!(
                    after_primary.len(), 0,
                    "When no additional workspaces, nothing should follow the primary workspace. Got: {:?}",
                    after_primary
                );
            }

            // Verify the primary workspace is the first positional argument
            // (i.e., it's not preceded by another positional workspace path)
            // All elements before primary_idx should be flags/options or their values
            prop_assert_eq!(
                &cmd_args[0], &agent_name,
                "First arg should be the agent name"
            );
            prop_assert_eq!(
                &cmd_args[1], "-q",
                "Second arg should be -q (quiet mode)"
            );
        }
    }

}
