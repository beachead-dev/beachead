use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// --- ID Types ---

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PersonaId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentTypeId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct McpContainerId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SharedMemoryId(pub String);

impl PersonaId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl AgentTypeId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl SessionId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl McpContainerId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

impl SharedMemoryId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

// --- Display implementations ---

impl std::fmt::Display for PersonaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for AgentTypeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for McpContainerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::fmt::Display for SharedMemoryId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// --- Domain Types ---

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentType {
    pub id: AgentTypeId,
    pub name: String,
    pub sbx_agent: Option<String>,
    pub kit_ref: Option<String>,
    pub is_builtin: bool,
    pub metadata: AgentMetadata,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    pub required_secrets: Vec<String>,
    pub auth_methods: Vec<AuthMethod>,
    pub description: String,
    pub supports_interactive_auth: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    ApiKey,
    OAuth,
    DeviceFlow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Persona {
    pub id: PersonaId,
    pub name: String,
    pub agent_type_id: AgentTypeId,
    pub workspace_path: PathBuf,
    pub memory_enabled: bool,
    pub agent_cli_args: Vec<String>,
    pub mcp_servers: Vec<PersonaMcpServer>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaMcpServer {
    pub id: String,
    pub persona_id: PersonaId,
    pub name: String,
    pub url: String,
    pub description: Option<String>,
    pub auth_headers: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Starting,
    Running,
    Stopped,
    Failed,
    Removed,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Starting => write!(f, "starting"),
            Self::Running => write!(f, "running"),
            Self::Stopped => write!(f, "stopped"),
            Self::Failed => write!(f, "failed"),
            Self::Removed => write!(f, "removed"),
        }
    }
}

impl std::str::FromStr for SessionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "starting" => Ok(Self::Starting),
            "running" => Ok(Self::Running),
            "stopped" => Ok(Self::Stopped),
            "failed" => Ok(Self::Failed),
            "removed" => Ok(Self::Removed),
            other => Err(format!("unknown session status: {}", other)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub persona_id: PersonaId,
    pub sandbox_id: Option<String>,
    pub kit_path: Option<PathBuf>,
    pub status: SessionStatus,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecretStatus {
    pub service: String,
    pub configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyStatus {
    pub sbx_available: bool,
    pub sbx_version: Option<String>,
    pub docker_available: bool,
    pub docker_version: Option<String>,
}

// --- API Request/Response Types ---

#[derive(Debug, Deserialize)]
pub struct CreatePersonaRequest {
    pub name: String,
    pub agent_type_id: AgentTypeId,
    pub workspace_path: PathBuf,
    pub memory_enabled: Option<bool>,
    pub agent_cli_args: Option<Vec<String>>,
    pub mcp_servers: Option<Vec<CreateMcpServerEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePersonaRequest {
    pub name: Option<String>,
    pub agent_type_id: Option<AgentTypeId>,
    pub workspace_path: Option<PathBuf>,
    pub memory_enabled: Option<bool>,
    pub agent_cli_args: Option<Vec<String>>,
    pub mcp_servers: Option<Vec<CreateMcpServerEntry>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreateMcpServerEntry {
    pub name: String,
    pub url: String,
    pub description: Option<String>,
    pub auth_headers: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct CreateAgentRequest {
    pub name: String,
    pub kit_ref: Option<String>,
    pub metadata: Option<AgentMetadata>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateAgentRequest {
    pub name: Option<String>,
    pub kit_ref: Option<String>,
    pub metadata: Option<AgentMetadata>,
}

#[derive(Debug, Deserialize)]
pub struct CreateSessionRequest {
    pub persona_id: PersonaId,
    pub name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateSessionResponse {
    pub session_id: SessionId,
    pub ws_url: String,
}

#[derive(Debug, Deserialize)]
pub struct SetSecretRequest {
    pub value: String,
}

#[derive(Debug, Deserialize)]
pub struct PublishPortRequest {
    pub port_spec: String,
}

#[derive(Debug, Deserialize)]
pub struct SetDefaultPolicyRequest {
    pub mode: String,
}

#[derive(Debug, Deserialize)]
pub struct AddPolicyRuleRequest {
    pub action: String,
    pub target: String,
}

#[derive(Debug, Serialize)]
pub struct UploadResult {
    pub sandbox_path: String,
    pub method: UploadMethod,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadMethod {
    Workspace,
    SbxCp,
}

/// Result of a persona update indicating whether changes were applied live
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
pub enum UpdateResult {
    Applied { persona: Persona },
    RequiresRestart { persona: Persona, reason: String },
}
