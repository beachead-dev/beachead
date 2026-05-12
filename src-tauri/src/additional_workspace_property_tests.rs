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

}
