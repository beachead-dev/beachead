//! Property-based tests for database persistence operations.
//! Uses proptest to generate arbitrary domain objects and verify round-trip persistence.

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

    /// Generate a valid URL string
    fn arb_url() -> impl Strategy<Value = String> {
        (
            prop::sample::select(vec!["http", "https"]),
            "[a-z]{3,10}",
            prop::sample::select(vec!["com", "io", "org", "net", "dev"]),
            prop::option::of(1024u16..65535u16),
            prop::option::of("[a-z/]{1,20}"),
        )
            .prop_map(|(scheme, host, tld, port, path)| {
                let mut url = format!("{}://{}.{}", scheme, host, tld);
                if let Some(p) = port {
                    url.push_str(&format!(":{}", p));
                }
                if let Some(path_str) = path {
                    url.push_str(&format!("/{}", path_str));
                }
                url
            })
    }

    /// Generate a valid workspace path
    fn arb_workspace_path() -> impl Strategy<Value = PathBuf> {
        "[a-zA-Z]{1,5}(/[a-zA-Z0-9_-]{1,10}){1,4}"
            .prop_map(|s| PathBuf::from(format!("/tmp/{}", s)))
    }

    /// Generate an arbitrary AuthMethod
    fn arb_auth_method() -> impl Strategy<Value = AuthMethod> {
        prop_oneof![
            Just(AuthMethod::ApiKey),
            Just(AuthMethod::OAuth),
            Just(AuthMethod::DeviceFlow),
        ]
    }

    /// Generate arbitrary AgentMetadata
    fn arb_agent_metadata() -> impl Strategy<Value = AgentMetadata> {
        (
            prop::collection::vec("[a-z]{3,10}", 0..4),
            prop::collection::vec(arb_auth_method(), 0..3),
            "[a-zA-Z ]{5,50}",
            any::<bool>(),
        )
            .prop_map(|(secrets, methods, desc, interactive)| AgentMetadata {
                required_secrets: secrets,
                auth_methods: methods,
                description: desc,
                supports_interactive_auth: interactive,
            })
    }

    /// Generate an arbitrary AgentType
    fn arb_agent_type() -> impl Strategy<Value = AgentType> {
        (
            arb_name(),
            prop::option::of("[a-z-]{3,15}"),
            prop::option::of("[a-z/.:]{5,30}"),
            any::<bool>(),
            arb_agent_metadata(),
        )
            .prop_map(|(name, sbx_agent, kit_ref, is_builtin, metadata)| {
                let now = Utc::now();
                AgentType {
                    id: AgentTypeId::new(),
                    name,
                    sbx_agent,
                    kit_ref,
                    is_builtin,
                    metadata,
                    created_at: now,
                    updated_at: now,
                }
            })
    }

    /// Generate arbitrary optional auth headers as JSON
    fn arb_auth_headers() -> impl Strategy<Value = Option<serde_json::Value>> {
        prop::option::of(
            prop::collection::hash_map("[A-Z][a-zA-Z-]{2,15}", "[a-zA-Z0-9]{5,20}", 1..3)
                .prop_map(|map| serde_json::to_value(map).unwrap()),
        )
    }

    // --- Property Tests ---

    // Property 1: Persona persistence round-trip
    proptest! {
        #[test]
        fn prop_persona_round_trip(
            agent in arb_agent_type(),
            persona_name in arb_name(),
            workspace in arb_workspace_path(),
            memory_enabled in any::<bool>(),
            cli_args in prop::collection::vec("[a-zA-Z0-9_-]{1,10}", 0..5),
            mcp_count in 0usize..4,
        ) {
            let db = Database::open_in_memory().unwrap();

            // Insert agent type and persona inside with_conn, collect data for assertions
            let (persona, loaded) = db.with_conn(|conn| {
                insert_agent_type(conn, &agent)?;

                let now = Utc::now();
                let persona_id = PersonaId::new();
                let mut persona = Persona {
                    id: persona_id.clone(),
                    name: persona_name,
                    agent_type_id: agent.id.clone(),
                    workspace_path: workspace,
                    memory_enabled,
                    agent_cli_args: cli_args,
                    mcp_servers: vec![],
                    created_at: now,
                    updated_at: now,
                };

                // Generate MCP servers with unique names
                for i in 0..mcp_count {
                    persona.mcp_servers.push(PersonaMcpServer {
                        id: uuid::Uuid::new_v4().to_string(),
                        persona_id: persona_id.clone(),
                        name: format!("mcp-server-{}", i),
                        url: format!("http://localhost:{}", 8000 + i),
                        description: Some(format!("Test MCP server {}", i)),
                        auth_headers: None,
                        created_at: now,
                        updated_at: now,
                    });
                }

                insert_persona(conn, &persona)?;
                let loaded = get_persona(conn, &persona.id)?;

                Ok((persona, loaded))
            }).unwrap();

            // Assert equality on all fields (outside with_conn so prop_assert works)
            prop_assert_eq!(&persona.id.0, &loaded.id.0);
            prop_assert_eq!(&persona.name, &loaded.name);
            prop_assert_eq!(&persona.agent_type_id.0, &loaded.agent_type_id.0);
            prop_assert_eq!(
                persona.workspace_path.to_string_lossy().to_string(),
                loaded.workspace_path.to_string_lossy().to_string()
            );
            prop_assert_eq!(persona.memory_enabled, loaded.memory_enabled);
            prop_assert_eq!(&persona.agent_cli_args, &loaded.agent_cli_args);
            prop_assert_eq!(persona.mcp_servers.len(), loaded.mcp_servers.len());

            // Verify MCP servers (sorted by name in both)
            let mut orig_mcps = persona.mcp_servers.clone();
            orig_mcps.sort_by(|a, b| a.name.cmp(&b.name));
            let mut loaded_mcps = loaded.mcp_servers.clone();
            loaded_mcps.sort_by(|a, b| a.name.cmp(&b.name));

            for (orig, loaded_mcp) in orig_mcps.iter().zip(loaded_mcps.iter()) {
                prop_assert_eq!(&orig.id, &loaded_mcp.id);
                prop_assert_eq!(&orig.name, &loaded_mcp.name);
                prop_assert_eq!(&orig.url, &loaded_mcp.url);
                prop_assert_eq!(&orig.description, &loaded_mcp.description);
            }
        }
    }

    // Property 2: Persona name uniqueness
    proptest! {
        #[test]
        fn prop_persona_name_uniqueness(
            agent in arb_agent_type(),
            name in arb_name(),
            ws1 in arb_workspace_path(),
            ws2 in arb_workspace_path(),
            mem1 in any::<bool>(),
            mem2 in any::<bool>(),
        ) {
            let db = Database::open_in_memory().unwrap();

            let result = db.with_conn(|conn| {
                insert_agent_type(conn, &agent)?;

                let now = Utc::now();

                // First persona
                let p1 = Persona {
                    id: PersonaId::new(),
                    name: name.clone(),
                    agent_type_id: agent.id.clone(),
                    workspace_path: ws1,
                    memory_enabled: mem1,
                    agent_cli_args: vec![],
                    mcp_servers: vec![],
                    created_at: now,
                    updated_at: now,
                };
                insert_persona(conn, &p1)?;

                // Second persona with same name but different fields
                let p2 = Persona {
                    id: PersonaId::new(),
                    name: name.clone(),
                    agent_type_id: agent.id.clone(),
                    workspace_path: ws2,
                    memory_enabled: mem2,
                    agent_cli_args: vec!["--verbose".to_string()],
                    mcp_servers: vec![],
                    created_at: now,
                    updated_at: now,
                };
                let insert_result = insert_persona(conn, &p2);
                Ok(insert_result.is_err())
            }).unwrap();

            prop_assert!(result, "Duplicate persona name should be rejected");
        }
    }

    // Property 4: Agent type persistence round-trip
    proptest! {
        #[test]
        fn prop_agent_type_round_trip(agent in arb_agent_type()) {
            let db = Database::open_in_memory().unwrap();

            let loaded = db.with_conn(|conn| {
                insert_agent_type(conn, &agent)?;
                get_agent_type(conn, &agent.id)
            }).unwrap();

            prop_assert_eq!(&agent.id.0, &loaded.id.0);
            prop_assert_eq!(&agent.name, &loaded.name);
            prop_assert_eq!(&agent.sbx_agent, &loaded.sbx_agent);
            prop_assert_eq!(&agent.kit_ref, &loaded.kit_ref);
            prop_assert_eq!(agent.is_builtin, loaded.is_builtin);
            prop_assert_eq!(&agent.metadata.required_secrets, &loaded.metadata.required_secrets);
            prop_assert_eq!(&agent.metadata.auth_methods, &loaded.metadata.auth_methods);
            prop_assert_eq!(&agent.metadata.description, &loaded.metadata.description);
            prop_assert_eq!(
                agent.metadata.supports_interactive_auth,
                loaded.metadata.supports_interactive_auth
            );
        }
    }

    // Property 11: MCP server entry persistence round-trip
    proptest! {
        #[test]
        fn prop_mcp_server_entry_round_trip(
            agent in arb_agent_type(),
            persona_name in arb_name(),
            mcp_name in arb_name(),
            mcp_url in arb_url(),
            mcp_desc in prop::option::of("[a-zA-Z ]{5,30}"),
            mcp_headers in arb_auth_headers(),
            updated_name in arb_name(),
            updated_url in arb_url(),
        ) {
            let db = Database::open_in_memory().unwrap();

            // Phase 1: Insert and read back
            let (mcp, loaded) = db.with_conn(|conn| {
                insert_agent_type(conn, &agent)?;

                let now = Utc::now();
                let persona_id = PersonaId::new();
                let persona = Persona {
                    id: persona_id.clone(),
                    name: persona_name,
                    agent_type_id: agent.id.clone(),
                    workspace_path: PathBuf::from("/tmp/test"),
                    memory_enabled: false,
                    agent_cli_args: vec![],
                    mcp_servers: vec![],
                    created_at: now,
                    updated_at: now,
                };
                insert_persona(conn, &persona)?;

                let mcp_id = uuid::Uuid::new_v4().to_string();
                let mcp = PersonaMcpServer {
                    id: mcp_id.clone(),
                    persona_id: persona_id.clone(),
                    name: mcp_name,
                    url: mcp_url,
                    description: mcp_desc,
                    auth_headers: mcp_headers,
                    created_at: now,
                    updated_at: now,
                };
                insert_persona_mcp_server(conn, &mcp)?;

                let loaded = get_persona_mcp_server(conn, &mcp_id)?;
                Ok((mcp, loaded))
            }).unwrap();

            // Assert initial read-back matches
            prop_assert_eq!(&mcp.id, &loaded.id);
            prop_assert_eq!(&mcp.name, &loaded.name);
            prop_assert_eq!(&mcp.url, &loaded.url);
            prop_assert_eq!(&mcp.description, &loaded.description);
            prop_assert_eq!(
                mcp.auth_headers.as_ref().map(|v| v.to_string()),
                loaded.auth_headers.as_ref().map(|v| v.to_string())
            );

            // Phase 2: Update and verify
            let updated_loaded = db.with_conn(|conn| {
                let updated_at = Utc::now();
                update_persona_mcp_server(
                    conn,
                    &mcp.id,
                    &updated_name,
                    &updated_url,
                    None,
                    None,
                    &updated_at,
                )?;
                get_persona_mcp_server(conn, &mcp.id)
            }).unwrap();

            prop_assert_eq!(&updated_name, &updated_loaded.name);
            prop_assert_eq!(&updated_url, &updated_loaded.url);
            prop_assert_eq!(None::<String>, updated_loaded.description);

            // Phase 3: Delete and verify gone
            let (delete_result, remaining_count) = db.with_conn(|conn| {
                delete_persona_mcp_server(conn, &mcp.id)?;
                let result = get_persona_mcp_server(conn, &mcp.id);
                let remaining = list_persona_mcp_servers(conn, &mcp.persona_id)?;
                Ok((result.is_err(), remaining.len()))
            }).unwrap();

            prop_assert!(delete_result, "Deleted MCP server should not be found");
            prop_assert_eq!(remaining_count, 0);
        }
    }

    // Property 8: Session-sandbox mapping persistence
    proptest! {
        #[test]
        fn prop_session_sandbox_mapping_persistence(
            agent in arb_agent_type(),
            persona_name in arb_name(),
            sandbox_id in prop::option::of("[a-f0-9]{12}"),
            kit_path in prop::option::of("[a-zA-Z0-9/_.-]{5,30}".prop_map(PathBuf::from)),
            status in prop::sample::select(vec![
                SessionStatus::Starting,
                SessionStatus::Running,
                SessionStatus::Stopped,
                SessionStatus::Failed,
                SessionStatus::Removed,
            ]),
            error_msg in prop::option::of("[a-zA-Z ]{5,50}"),
        ) {
            let db = Database::open_in_memory().unwrap();

            let (session, loaded) = db.with_conn(|conn| {
                insert_agent_type(conn, &agent)?;

                let now = Utc::now();
                let persona_id = PersonaId::new();
                let persona = Persona {
                    id: persona_id.clone(),
                    name: persona_name,
                    agent_type_id: agent.id.clone(),
                    workspace_path: PathBuf::from("/tmp/workspace"),
                    memory_enabled: false,
                    agent_cli_args: vec![],
                    mcp_servers: vec![],
                    created_at: now,
                    updated_at: now,
                };
                insert_persona(conn, &persona)?;

                let session = Session {
                    id: SessionId::new(),
                    persona_id: persona_id.clone(),
                    sandbox_id,
                    kit_path,
                    status,
                    error_message: error_msg,
                    created_at: now,
                    updated_at: now,
                };
                insert_session(conn, &session)?;

                let loaded = get_session(conn, &session.id)?;
                Ok((session, loaded))
            }).unwrap();

            prop_assert_eq!(&session.id.0, &loaded.id.0);
            prop_assert_eq!(&session.persona_id.0, &loaded.persona_id.0);
            prop_assert_eq!(&session.sandbox_id, &loaded.sandbox_id);
            prop_assert_eq!(
                session.kit_path.as_ref().map(|p| p.to_string_lossy().to_string()),
                loaded.kit_path.as_ref().map(|p| p.to_string_lossy().to_string())
            );
            prop_assert_eq!(&session.status, &loaded.status);
            prop_assert_eq!(&session.error_message, &loaded.error_message);
        }
    }
}
