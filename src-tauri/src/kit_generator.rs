//! Kit Generator: creates dynamic mixin kit directories for personas.
//!
//! Each persona gets a generated mixin kit at session start containing:
//! - spec.yaml with schemaVersion, kind, name, description
//! - initFiles for MCP configuration (if memory enabled or additional MCP servers)
//! - network.allowedDomains for each MCP server port
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
    ) -> Result<PathBuf, OrchestratorError> {
        let dir_name = format!("{}-{}", persona.name, uuid::Uuid::new_v4());
        let kit_dir = self.kit_base_dir.join(&dir_name);
        fs::create_dir_all(&kit_dir)?;

        let spec_yaml = self.build_spec_yaml(persona, mcp_config);
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
    fn build_spec_yaml(&self, persona: &Persona, mcp_config: Option<&McpConfig>) -> String {
        let mut yaml = String::new();

        // Header
        yaml.push_str("schemaVersion: \"1\"\n");
        yaml.push_str("kind: mixin\n");
        yaml.push_str(&format!("name: persona-{}\n", persona.name));
        yaml.push_str(&format!(
            "description: Auto-generated kit for persona {}\n",
            persona.name
        ));

        // initFiles section (MCP config)
        let mcp_json = self.build_mcp_json(persona, mcp_config);
        if let Some(mcp_content) = mcp_json {
            yaml.push_str("\ninitFiles:\n");
            yaml.push_str("  - path: ${WORKDIR}/.beachead/mcp.json\n");
            yaml.push_str("    content: |\n");
            for line in mcp_content.lines() {
                yaml.push_str(&format!("      {}\n", line));
            }
        }

        // network.allowedDomains
        let domains = self.collect_allowed_domains(persona, mcp_config);
        if !domains.is_empty() {
            yaml.push_str("\nnetwork:\n");
            yaml.push_str("  allowedDomains:\n");
            for domain in &domains {
                yaml.push_str(&format!("    - \"{}\"\n", domain));
            }
        }

        // memory field (if memory enabled)
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

        // Memory MCP server (Phase 2)
        if let Some(config) = mcp_config {
            let mut headers = serde_json::Map::new();
            headers.insert(
                "Authorization".to_string(),
                serde_json::Value::String(format!("Bearer {}", config.bearer_token)),
            );

            let mut memory_server = serde_json::Map::new();
            memory_server.insert(
                "url".to_string(),
                serde_json::Value::String(config.url.clone()),
            );
            memory_server.insert(
                "headers".to_string(),
                serde_json::Value::Object(headers),
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

    /// Collect all network domains that need to be allowed for MCP servers.
    fn collect_allowed_domains(
        &self,
        persona: &Persona,
        mcp_config: Option<&McpConfig>,
    ) -> Vec<String> {
        let mut domains = Vec::new();

        // Memory MCP port
        if let Some(config) = mcp_config {
            domains.push(format!("127.0.0.1:{}", config.port));
        }

        // Additional MCP server ports
        for mcp_server in &persona.mcp_servers {
            if let Some(domain) = extract_host_port(&mcp_server.url) {
                domains.push(domain);
            }
        }

        domains
    }
}

/// Extract host:port from a URL string for network allowedDomains.
fn extract_host_port(url: &str) -> Option<String> {
    // Determine scheme and strip it
    let after_scheme = if url.starts_with("https://") {
        &url[8..]
    } else if url.starts_with("http://") {
        &url[7..]
    } else {
        return None;
    };

    // Extract authority (before first '/')
    let authority = after_scheme.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }

    // Check if there's an explicit port
    if let Some(colon_pos) = authority.rfind(':') {
        let host = &authority[..colon_pos];
        let port_str = &authority[colon_pos + 1..];
        if !host.is_empty() && port_str.parse::<u16>().is_ok() {
            return Some(format!("{}:{}", host, port_str));
        }
    }

    // No explicit port — use default for scheme
    let default_port = if url.starts_with("https://") {
        443
    } else {
        80
    };
    Some(format!("{}:{}", authority, default_port))
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

        let kit_path = generator.generate(&persona, None).unwrap();

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

        let kit_path = generator.generate(&persona, Some(&mcp_config)).unwrap();
        let content = fs::read_to_string(kit_path.join("spec.yaml")).unwrap();

        // Should have initFiles with MCP config
        assert!(content.contains("initFiles:"));
        assert!(content.contains("${WORKDIR}/.beachead/mcp.json"));
        assert!(content.contains("mcpServers"));
        assert!(content.contains("memory"));
        assert!(content.contains("host.docker.internal:9100/sse"));
        assert!(content.contains("Bearer secret-token-123"));

        // Should have network allowedDomains
        assert!(content.contains("network:"));
        assert!(content.contains("allowedDomains:"));
        assert!(content.contains("127.0.0.1:9100"));

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
        let kit_path = generator.generate(&persona, None).unwrap();
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

        // Should have network allowedDomains for both servers
        assert!(content.contains("network:"));
        assert!(content.contains("localhost:8080"));
        assert!(content.contains("localhost:9090"));
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

        let kit_path = generator.generate(&persona, Some(&mcp_config)).unwrap();
        let content = fs::read_to_string(kit_path.join("spec.yaml")).unwrap();

        // Both memory and custom-tool should be in mcpServers
        assert!(content.contains("\"memory\""));
        assert!(content.contains("\"custom-tool\""));
        assert!(content.contains("host.docker.internal:9200/sse"));
        assert!(content.contains("http://localhost:7070/api"));

        // Network should include both ports
        assert!(content.contains("127.0.0.1:9200"));
        assert!(content.contains("localhost:7070"));

        // Memory instructions present
        assert!(content.contains("memory: |"));
    }

    #[test]
    fn test_cleanup_removes_directory() {
        let tmp = TempDir::new().unwrap();
        let generator = KitGenerator::new(tmp.path().to_path_buf());
        let persona = make_persona("cleanup-test", false, vec![]);

        let kit_path = generator.generate(&persona, None).unwrap();
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

        let path1 = generator.generate(&persona, None).unwrap();
        let path2 = generator.generate(&persona, None).unwrap();

        assert_ne!(path1, path2);
        assert!(path1.exists());
        assert!(path2.exists());
    }

    #[test]
    fn test_extract_host_port_with_port() {
        assert_eq!(
            extract_host_port("http://localhost:8080/path"),
            Some("localhost:8080".to_string())
        );
    }

    #[test]
    fn test_extract_host_port_default_http() {
        assert_eq!(
            extract_host_port("http://example.com/path"),
            Some("example.com:80".to_string())
        );
    }

    #[test]
    fn test_extract_host_port_default_https() {
        assert_eq!(
            extract_host_port("https://example.com/path"),
            Some("example.com:443".to_string())
        );
    }

    #[test]
    fn test_extract_host_port_invalid_url() {
        assert_eq!(extract_host_port("not-a-url"), None);
    }

    #[test]
    fn test_spec_yaml_description() {
        let tmp = TempDir::new().unwrap();
        let generator = KitGenerator::new(tmp.path().to_path_buf());
        let persona = make_persona("desc-test", false, vec![]);

        let kit_path = generator.generate(&persona, None).unwrap();
        let content = fs::read_to_string(kit_path.join("spec.yaml")).unwrap();

        assert!(content.contains("description: Auto-generated kit for persona desc-test"));
    }

    // --- Property-based tests ---

    mod prop_tests {
        use super::*;
        use proptest::prelude::*;
        use proptest::collection::vec as prop_vec;

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

                    let kit_path = generator.generate(&persona, None).unwrap();
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
                mcp_servers in prop_vec(arb_persona_mcp_server(), 0..5),
                mcp_config in arb_mcp_config(),
            ) {
                let tmp = TempDir::new().unwrap();
                let generator = KitGenerator::new(tmp.path().to_path_buf());

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
                    .generate(&persona, mcp_config.as_ref())
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
                    prop_assert!(
                        content.contains(&format!("Bearer {}", config.bearer_token)),
                        "McpConfig bearer token not found in spec.yaml"
                    );
                }

                // --- network.allowedDomains ---
                if has_any_mcp {
                    prop_assert!(
                        content.contains("network:"),
                        "Has MCP servers but missing network section"
                    );
                    prop_assert!(
                        content.contains("allowedDomains:"),
                        "Has MCP servers but missing allowedDomains"
                    );

                    // Memory MCP port in allowedDomains
                    if let Some(ref config) = mcp_config {
                        prop_assert!(
                            content.contains(&format!("127.0.0.1:{}", config.port)),
                            "McpConfig port {} not in allowedDomains",
                            config.port
                        );
                    }

                    // Each persona MCP server host:port in allowedDomains
                    for server in &mcp_servers {
                        if let Some(domain) = extract_host_port(&server.url) {
                            prop_assert!(
                                content.contains(&domain),
                                "MCP server domain '{}' not in allowedDomains",
                                domain
                            );
                        }
                    }
                } else {
                    prop_assert!(
                        !content.contains("network:"),
                        "No MCP servers but network section present"
                    );
                }
            }
        }
    }
}
