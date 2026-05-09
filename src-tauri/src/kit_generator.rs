//! Kit Generator: creates dynamic mixin kit directories for personas.
//!
//! Each persona gets a generated mixin kit at session start containing:
//! - spec.yaml with schemaVersion, kind, name, description
//! - commands.initFiles for MCP configuration (placed at .mcp.json in workspace root)
//! - memory field with markdown instructions (if memory enabled)
//! - environment.variables with at minimum BEACHEAD_PERSONA

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::OrchestratorError;
use crate::types::Persona;

/// Configuration for an MCP server to include in the kit.
/// Used for Phase 2 memory MCP injection.
#[derive(Debug, Clone)]
pub struct McpConfig {
    pub url: String,
    pub bearer_token: String,
    pub port: u16,
}

/// Generates mixin kit directories for personas at session start.
pub struct KitGenerator {
    kit_base_dir: PathBuf,
}

impl KitGenerator {
    /// Create a new KitGenerator with the specified base directory for kit output.
    pub fn new(kit_base_dir: PathBuf) -> Self {
        Self { kit_base_dir }
    }

    /// Generate a mixin kit directory for the given persona.
    ///
    /// Creates a directory structure:
    /// ```text
    /// <kit_base_dir>/<persona_name>-<uuid>/
    ///   spec.yaml
    /// ```
    ///
    /// Returns the path to the generated kit directory.
    pub fn generate(
        &self,
        persona: &Persona,
        mcp_config: Option<&McpConfig>,
        mcp_config_path: Option<&str>,
    ) -> Result<PathBuf, OrchestratorError> {
        let dir_name = format!("{}-{}", persona.name, uuid::Uuid::new_v4());
        let kit_dir = self.kit_base_dir.join(&dir_name);
        fs::create_dir_all(&kit_dir)?;

        let spec_yaml = self.build_spec_yaml(persona, mcp_config, mcp_config_path);
        fs::write(kit_dir.join("spec.yaml"), spec_yaml)?;

        Ok(kit_dir)
    }

    /// Remove a generated kit directory and all its contents.
    pub fn cleanup(&self, kit_path: &Path) -> Result<(), OrchestratorError> {
        if kit_path.exists() {
            fs::remove_dir_all(kit_path)?;
        }
        Ok(())
    }

    /// Build the spec.yaml content for a persona's mixin kit.
    fn build_spec_yaml(&self, persona: &Persona, mcp_config: Option<&McpConfig>, mcp_config_path: Option<&str>) -> String {
        let mut yaml = String::new();

        // Header
        yaml.push_str("schemaVersion: \"1\"\n");
        yaml.push_str("kind: mixin\n");
        yaml.push_str(&format!("name: persona-{}\n", persona.name));
        yaml.push_str(&format!(
            "description: Auto-generated kit for persona {}\n",
            persona.name
        ));

        // commands.initFiles section (MCP config at agent-specific path)
        // Each agent reads MCP config from a different location.
        let mcp_json = self.build_mcp_json(persona, mcp_config);
        if let Some(mcp_content) = mcp_json {
            let config_path = mcp_config_path.unwrap_or(".mcp.json");
            let workspace_path = persona.workspace_path.to_string_lossy();
            yaml.push_str("\ncommands:\n");
            yaml.push_str("  initFiles:\n");
            yaml.push_str(&format!("    - path: {}/{}\n", workspace_path, config_path));
            yaml.push_str("      content: |\n");
            for line in mcp_content.lines() {
                yaml.push_str(&format!("        {}\n", line));
            }
        }

        // memory field (instructions appended to agent's AI file if supported)
        if persona.memory_enabled {
            yaml.push_str("\nmemory: |\n");
            yaml.push_str("  ## Memory Instructions\n");
            yaml.push_str(
                "  You have access to a long-term memory system via MCP tools.\n",
            );
            yaml.push_str(
                "  Use memory_store to save important context, decisions, and learnings.\n",
            );
            yaml.push_str(
                "  Use memory_query to retrieve relevant past knowledge before starting work.\n",
            );
            yaml.push_str(
                "  Use memory_list to see what's stored. Use memory_delete to remove outdated entries.\n",
            );
        }

        // environment.variables
        yaml.push_str("\nenvironment:\n");
        yaml.push_str("  variables:\n");
        yaml.push_str(&format!("    BEACHEAD_PERSONA: \"{}\"\n", persona.name));

        yaml
    }

    /// Build the MCP JSON configuration content.
    /// Returns None if there are no MCP servers to configure.
    fn build_mcp_json(
        &self,
        persona: &Persona,
        mcp_config: Option<&McpConfig>,
    ) -> Option<String> {
        let has_memory = mcp_config.is_some();
        let has_additional = !persona.mcp_servers.is_empty();

        if !has_memory && !has_additional {
            return None;
        }

        let mut servers = serde_json::Map::new();

        // Memory MCP server
        // No auth headers needed — each persona has its own container with
        // isolated data. Network policy restricts sandbox access to localhost ports.
        if let Some(config) = mcp_config {
            let mut memory_server = serde_json::Map::new();
            memory_server.insert(
                "url".to_string(),
                serde_json::Value::String(config.url.clone()),
            );

            servers.insert("memory".to_string(), serde_json::Value::Object(memory_server));
        }

        // Additional MCP servers
        for mcp_server in &persona.mcp_servers {
            let mut server_entry = serde_json::Map::new();
            server_entry.insert(
                "url".to_string(),
                serde_json::Value::String(mcp_server.url.clone()),
            );

            if let Some(ref auth) = mcp_server.auth_headers {
                server_entry.insert("headers".to_string(), auth.clone());
            }

            servers.insert(mcp_server.name.clone(), serde_json::Value::Object(server_entry));
        }

        let mcp_json = serde_json::json!({
            "mcpServers": servers
        });

        Some(serde_json::to_string_pretty(&mcp_json).unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentTypeId, PersonaId, PersonaMcpServer};
    use chrono::Utc;
    use tempfile::TempDir;

    fn make_persona(name: &str, memory_enabled: bool, mcp_servers: Vec<PersonaMcpServer>) -> Persona {
        Persona {
            id: PersonaId("test-id".to_string()),
            name: name.to_string(),
            agent_type_id: AgentTypeId("agent-1".to_string()),
            workspace_path: PathBuf::from("/tmp/workspace"),
            memory_enabled,
            agent_cli_args: vec![],
            mcp_servers,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn make_mcp_server(name: &str, url: &str, auth: Option<serde_json::Value>) -> PersonaMcpServer {
        PersonaMcpServer {
            id: format!("mcp-{}", name),
            persona_id: PersonaId("test-id".to_string()),
            name: name.to_string(),
            url: url.to_string(),
            description: None,
            auth_headers: auth,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn test_generate_basic_kit() {
        let tmp = TempDir::new().unwrap();
        let generator = KitGenerator::new(tmp.path().to_path_buf());
        let persona = make_persona("test-agent", false, vec![]);

        let kit_path = generator.generate(&persona, None, None).unwrap();

        assert!(kit_path.exists());
        assert!(kit_path.join("spec.yaml").exists());

        let content = fs::read_to_string(kit_path.join("spec.yaml")).unwrap();
        assert!(content.contains("schemaVersion: \"1\""));
        assert!(content.contains("kind: mixin"));
        assert!(content.contains("name: persona-test-agent"));
        assert!(content.contains("BEACHEAD_PERSONA: \"test-agent\""));
        // No initFiles or network when no MCP servers
        assert!(!content.contains("initFiles:"));
        assert!(!content.contains("network:"));
        // No memory field when memory disabled
        assert!(!content.contains("memory: |"));
    }

    #[test]
    fn test_generate_kit_with_memory_enabled() {
        let tmp = TempDir::new().unwrap();
        let generator = KitGenerator::new(tmp.path().to_path_buf());
        let persona = make_persona("memory-agent", true, vec![]);

        let mcp_config = McpConfig {
            url: "http://host.docker.internal:9100/sse".to_string(),
            bearer_token: "secret-token-123".to_string(),
            port: 9100,
        };

        let kit_path = generator.generate(&persona, Some(&mcp_config), None).unwrap();
        let content = fs::read_to_string(kit_path.join("spec.yaml")).unwrap();

        // Should have initFiles with MCP config
        assert!(content.contains("initFiles:"));
        assert!(content.contains("/.mcp.json"));
        assert!(content.contains("mcpServers"));
        assert!(content.contains("memory"));
        assert!(content.contains("host.docker.internal:9100/sse"));

        // Auth headers are not included — isolation is via per-persona containers
        assert!(!content.contains("Bearer"));
        assert!(!content.contains("Authorization"));

        // Should NOT have network allowedDomains (restrictive allowlist removed)
        assert!(!content.contains("network:"));
        assert!(!content.contains("allowedDomains:"));

        // Should have memory instructions
        assert!(content.contains("memory: |"));
        assert!(content.contains("Memory Instructions"));
        assert!(content.contains("memory_store"));
        assert!(content.contains("memory_query"));
        assert!(content.contains("memory_list"));
        assert!(content.contains("memory_delete"));

        // Should have environment variable
        assert!(content.contains("BEACHEAD_PERSONA: \"memory-agent\""));
    }

    #[test]
    fn test_generate_kit_with_additional_mcp_servers() {
        let tmp = TempDir::new().unwrap();
        let generator = KitGenerator::new(tmp.path().to_path_buf());

        let auth_headers = serde_json::json!({
            "X-Api-Key": "my-api-key"
        });

        let mcp_servers = vec![
            make_mcp_server("database", "http://localhost:8080/mcp", Some(auth_headers)),
            make_mcp_server("tools", "http://localhost:9090/sse", None),
        ];

        let persona = make_persona("multi-mcp", false, mcp_servers);
        let kit_path = generator.generate(&persona, None, None).unwrap();
        let content = fs::read_to_string(kit_path.join("spec.yaml")).unwrap();

        // Should have initFiles with MCP config
        assert!(content.contains("initFiles:"));
        assert!(content.contains("mcpServers"));
        assert!(content.contains("database"));
        assert!(content.contains("http://localhost:8080/mcp"));
        assert!(content.contains("X-Api-Key"));
        assert!(content.contains("my-api-key"));
        assert!(content.contains("tools"));
        assert!(content.contains("http://localhost:9090/sse"));

        // Should NOT have network allowedDomains (restrictive allowlist removed)
        assert!(!content.contains("network:"));
        assert!(!content.contains("allowedDomains:"));
    }

    #[test]
    fn test_generate_kit_with_memory_and_additional_mcp() {
        let tmp = TempDir::new().unwrap();
        let generator = KitGenerator::new(tmp.path().to_path_buf());

        let mcp_servers = vec![
            make_mcp_server("custom-tool", "http://localhost:7070/api", None),
        ];

        let persona = make_persona("full-config", true, mcp_servers);
        let mcp_config = McpConfig {
            url: "http://host.docker.internal:9200/sse".to_string(),
            bearer_token: "mem-token".to_string(),
            port: 9200,
        };

        let kit_path = generator.generate(&persona, Some(&mcp_config), None).unwrap();
        let content = fs::read_to_string(kit_path.join("spec.yaml")).unwrap();

        // Both memory and custom-tool should be in mcpServers
        assert!(content.contains("\"memory\""));
        assert!(content.contains("\"custom-tool\""));
        assert!(content.contains("host.docker.internal:9200/sse"));
        assert!(content.contains("http://localhost:7070/api"));

        // Should NOT have network allowedDomains (restrictive allowlist removed)
        assert!(!content.contains("network:"));

        // Memory instructions present
        assert!(content.contains("memory: |"));
    }

    #[test]
    fn test_cleanup_removes_directory() {
        let tmp = TempDir::new().unwrap();
        let generator = KitGenerator::new(tmp.path().to_path_buf());
        let persona = make_persona("cleanup-test", false, vec![]);

        let kit_path = generator.generate(&persona, None, None).unwrap();
        assert!(kit_path.exists());

        generator.cleanup(&kit_path).unwrap();
        assert!(!kit_path.exists());
    }

    #[test]
    fn test_cleanup_nonexistent_path_is_ok() {
        let tmp = TempDir::new().unwrap();
        let generator = KitGenerator::new(tmp.path().to_path_buf());
        let nonexistent = tmp.path().join("does-not-exist");

        // Should not error
        generator.cleanup(&nonexistent).unwrap();
    }

    #[test]
    fn test_generate_creates_unique_directories() {
        let tmp = TempDir::new().unwrap();
        let generator = KitGenerator::new(tmp.path().to_path_buf());
        let persona = make_persona("unique-test", false, vec![]);

        let path1 = generator.generate(&persona, None, None).unwrap();
        let path2 = generator.generate(&persona, None, None).unwrap();

        assert_ne!(path1, path2);
        assert!(path1.exists());
        assert!(path2.exists());
    }

    #[test]
    fn test_spec_yaml_description() {
        let tmp = TempDir::new().unwrap();
        let generator = KitGenerator::new(tmp.path().to_path_buf());
        let persona = make_persona("desc-test", false, vec![]);

        let kit_path = generator.generate(&persona, None, None).unwrap();
        let content = fs::read_to_string(kit_path.join("spec.yaml")).unwrap();

        assert!(content.contains("description: Auto-generated kit for persona desc-test"));
    }

    // --- Property-based tests ---

    mod prop_tests {
        use super::*;
        use proptest::prelude::*;
        use proptest::collection::vec as prop_vec;
        use std::collections::HashSet;

        /// **Validates: Requirements 3.1, 10.4, 10.5, 18.1–18.7**
        ///
        /// Property 6: Kit generation completeness
        /// For any valid persona configuration (varying memory, MCP servers, McpConfig),
        /// the generated spec.yaml always contains all required sections.

        /// Strategy for generating a valid MCP server name (alphanumeric + hyphens)
        fn arb_mcp_name() -> impl Strategy<Value = String> {
            "[a-z][a-z0-9\\-]{1,10}".prop_map(|s| s)
        }

        /// Strategy for generating a valid port number
        fn arb_port() -> impl Strategy<Value = u16> {
            1024..=65000u16
        }

        /// Strategy for generating a valid MCP server URL with explicit port
        fn arb_mcp_url() -> impl Strategy<Value = String> {
            (
                prop_oneof![Just("http"), Just("https")],
                prop_oneof![
                    Just("localhost".to_string()),
                    Just("host.docker.internal".to_string()),
                    Just("127.0.0.1".to_string()),
                ],
                arb_port(),
                prop_oneof![
                    Just("/sse".to_string()),
                    Just("/mcp".to_string()),
                    Just("/api".to_string()),
                ],
            )
                .prop_map(|(scheme, host, port, path)| {
                    format!("{}://{}:{}{}", scheme, host, port, path)
                })
        }

        /// Strategy for generating optional auth headers
        fn arb_auth_headers() -> impl Strategy<Value = Option<serde_json::Value>> {
            prop_oneof![
                Just(None),
                "[a-zA-Z]{5,15}".prop_map(|token| {
                    Some(serde_json::json!({"Authorization": format!("Bearer {}", token)}))
                }),
                "[a-zA-Z0-9]{8,20}".prop_map(|key| {
                    Some(serde_json::json!({"X-Api-Key": key}))
                }),
            ]
        }

        /// Strategy for generating a PersonaMcpServer
        fn arb_persona_mcp_server() -> impl Strategy<Value = PersonaMcpServer> {
            (arb_mcp_name(), arb_mcp_url(), arb_auth_headers()).prop_map(
                |(name, url, auth_headers)| PersonaMcpServer {
                    id: format!("mcp-{}", name),
                    persona_id: PersonaId("prop-test-id".to_string()),
                    name,
                    url,
                    description: None,
                    auth_headers,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
            )
        }

        /// Strategy for generating an optional McpConfig (memory MCP)
        fn arb_mcp_config() -> impl Strategy<Value = Option<McpConfig>> {
            prop_oneof![
                Just(None),
                (arb_port(), "[a-zA-Z0-9]{10,30}").prop_map(|(port, token)| {
                    Some(McpConfig {
                        url: format!("http://host.docker.internal:{}/sse", port),
                        bearer_token: token,
                        port,
                    })
                }),
            ]
        }

        /// Strategy for generating a persona name (lowercase alpha, 3-12 chars)
        fn arb_persona_name() -> impl Strategy<Value = String> {
            "[a-z]{3,12}"
        }

        proptest! {
            /// **Validates: Requirements 3.10**
            ///
            /// Property 9: Kit directory cleanup on session removal
            /// For any session with generated kit directories, cleanup removes them
            /// from the filesystem, and cleanup is idempotent.
            #[test]
            fn prop_kit_directory_cleanup_on_removal(
                names in prop_vec(arb_persona_name(), 1..5),
                memory_enabled in any::<bool>(),
            ) {
                let tmp = TempDir::new().unwrap();
                let generator = KitGenerator::new(tmp.path().to_path_buf());

                // Generate kit directories for each persona name
                let mut kit_paths = Vec::new();
                for name in &names {
                    let persona = Persona {
                        id: PersonaId("prop-cleanup-id".to_string()),
                        name: name.clone(),
                        agent_type_id: AgentTypeId("agent-prop".to_string()),
                        workspace_path: PathBuf::from("/tmp/workspace"),
                        memory_enabled,
                        agent_cli_args: vec![],
                        mcp_servers: vec![],
                        created_at: Utc::now(),
                        updated_at: Utc::now(),
                    };

                    let kit_path = generator.generate(&persona, None, None).unwrap();
                    // Verify kit directory was created
                    prop_assert!(
                        kit_path.exists(),
                        "Kit directory should exist after generate(): {:?}",
                        kit_path
                    );
                    prop_assert!(
                        kit_path.join("spec.yaml").exists(),
                        "spec.yaml should exist in kit directory"
                    );
                    kit_paths.push(kit_path);
                }

                // Cleanup each kit directory (simulating session removal)
                for kit_path in &kit_paths {
                    generator.cleanup(kit_path).unwrap();
                }

                // Assert: all kit directories are deleted from filesystem
                for kit_path in &kit_paths {
                    prop_assert!(
                        !kit_path.exists(),
                        "Kit directory should NOT exist after cleanup: {:?}",
                        kit_path
                    );
                }

                // Assert: cleanup is idempotent — calling again doesn't error
                for kit_path in &kit_paths {
                    let result = generator.cleanup(kit_path);
                    prop_assert!(
                        result.is_ok(),
                        "Cleanup on already-cleaned path should not error: {:?}",
                        result
                    );
                }
            }

            #[test]
            fn prop_kit_generation_completeness(
                name in arb_persona_name(),
                memory_enabled in any::<bool>(),
                mcp_servers_raw in prop_vec(arb_persona_mcp_server(), 0..5),
                mcp_config in arb_mcp_config(),
            ) {
                let tmp = TempDir::new().unwrap();
                let generator = KitGenerator::new(tmp.path().to_path_buf());

                // Deduplicate MCP servers by name since JSON maps can't have duplicate keys
                let mut seen_names = HashSet::new();
                let mcp_servers: Vec<PersonaMcpServer> = mcp_servers_raw
                    .into_iter()
                    .filter(|s| seen_names.insert(s.name.clone()))
                    .collect();

                let persona = Persona {
                    id: PersonaId("prop-test-id".to_string()),
                    name: name.clone(),
                    agent_type_id: AgentTypeId("agent-prop".to_string()),
                    workspace_path: PathBuf::from("/tmp/workspace"),
                    memory_enabled,
                    agent_cli_args: vec![],
                    mcp_servers: mcp_servers.clone(),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                };

                let kit_path = generator
                    .generate(&persona, mcp_config.as_ref(), None)
                    .unwrap();

                let content = fs::read_to_string(kit_path.join("spec.yaml")).unwrap();

                // --- Always-present sections ---
                prop_assert!(
                    content.contains("schemaVersion: \"1\""),
                    "Missing schemaVersion in spec.yaml"
                );
                prop_assert!(
                    content.contains("kind: mixin"),
                    "Missing kind: mixin in spec.yaml"
                );
                prop_assert!(
                    content.contains(&format!("name: persona-{}", name)),
                    "Missing name: persona-{} in spec.yaml", name
                );
                prop_assert!(
                    content.contains(&format!("BEACHEAD_PERSONA: \"{}\"", name)),
                    "Missing BEACHEAD_PERSONA env var in spec.yaml"
                );

                // --- Memory section ---
                if memory_enabled {
                    prop_assert!(
                        content.contains("memory: |"),
                        "memory_enabled=true but missing 'memory: |' section"
                    );
                    prop_assert!(
                        content.contains("Memory Instructions"),
                        "memory_enabled=true but missing memory instructions"
                    );
                } else {
                    prop_assert!(
                        !content.contains("memory: |"),
                        "memory_enabled=false but 'memory: |' section present"
                    );
                }

                // --- initFiles section (MCP servers) ---
                let has_any_mcp = !mcp_servers.is_empty() || mcp_config.is_some();
                if has_any_mcp {
                    prop_assert!(
                        content.contains("initFiles:"),
                        "Has MCP servers but missing initFiles section"
                    );
                    // Each persona MCP server name and URL should appear
                    for server in &mcp_servers {
                        prop_assert!(
                            content.contains(&server.name),
                            "MCP server name '{}' not found in spec.yaml",
                            server.name
                        );
                        prop_assert!(
                            content.contains(&server.url),
                            "MCP server URL '{}' not found in spec.yaml",
                            server.url
                        );
                    }
                } else {
                    prop_assert!(
                        !content.contains("initFiles:"),
                        "No MCP servers but initFiles section present"
                    );
                }

                // --- McpConfig (memory MCP) in initFiles ---
                if let Some(ref config) = mcp_config {
                    prop_assert!(
                        content.contains(&config.url),
                        "McpConfig URL '{}' not found in spec.yaml",
                        config.url
                    );
                    // Memory server should not have bearer token in the config
                    prop_assert!(
                        !content.contains(&format!("Bearer {}", config.bearer_token)),
                        "Memory MCP bearer token should not be in spec.yaml"
                    );
                }

                // --- network.allowedDomains ---
                // Network section should never be emitted (restrictive allowlist removed)
                prop_assert!(
                    !content.contains("network:"),
                    "network: section should not be present in kit spec.yaml"
                );
                prop_assert!(
                    !content.contains("allowedDomains:"),
                    "allowedDomains: section should not be present in kit spec.yaml"
                );
            }
        }
    }
}
