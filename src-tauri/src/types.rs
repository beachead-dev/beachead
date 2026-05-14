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

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ManagedRepoId(pub String);

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

impl ManagedRepoId {
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

impl std::fmt::Display for ManagedRepoId {
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
    /// Relative path from workspace root where this agent reads MCP server config.
    /// e.g., ".mcp.json", ".kiro/settings/mcp.json", ".cursor/mcp.json"
    #[serde(default)]
    pub mcp_config_path: Option<String>,
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
    pub additional_workspaces: Vec<AdditionalWorkspace>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// An additional workspace mount associated with a persona.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdditionalWorkspace {
    pub id: String,
    pub persona_id: PersonaId,
    pub path: PathBuf,
    pub read_only: bool,
    pub position: i32,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// Request entry for creating/updating an additional workspace.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateAdditionalWorkspaceEntry {
    pub path: PathBuf,
    pub read_only: bool,
    pub label: Option<String>,
}

/// A workspace path ready to be passed to sbx create as a positional arg.
#[derive(Debug, Clone)]
pub struct AdditionalWorkspaceArg {
    pub path: PathBuf,
    pub read_only: bool,
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
    pub git_available: bool,
    pub git_version: Option<String>,
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
    pub additional_workspaces: Option<Vec<CreateAdditionalWorkspaceEntry>>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePersonaRequest {
    pub name: Option<String>,
    pub agent_type_id: Option<AgentTypeId>,
    pub workspace_path: Option<PathBuf>,
    pub memory_enabled: Option<bool>,
    pub agent_cli_args: Option<Vec<String>>,
    pub mcp_servers: Option<Vec<CreateMcpServerEntry>>,
    pub additional_workspaces: Option<Vec<CreateAdditionalWorkspaceEntry>>,
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

// --- Repo Sync Domain Types ---

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    LocalOnly,
    Remote,
}

impl std::fmt::Display for SyncMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LocalOnly => write!(f, "local_only"),
            Self::Remote => write!(f, "remote"),
        }
    }
}

impl std::str::FromStr for SyncMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local_only" => Ok(Self::LocalOnly),
            "remote" => Ok(Self::Remote),
            other => Err(format!("unknown sync mode: {}", other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchStrategy {
    Direct,
    FeatureBranch,
}

impl std::fmt::Display for BranchStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Direct => write!(f, "direct"),
            Self::FeatureBranch => write!(f, "feature_branch"),
        }
    }
}

impl std::str::FromStr for BranchStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "direct" => Ok(Self::Direct),
            "feature_branch" => Ok(Self::FeatureBranch),
            other => Err(format!("unknown branch strategy: {}", other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttributionMode {
    KeepAgent,
    RewriteUser,
    CoAuthoredBy,
}

impl std::fmt::Display for AttributionMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::KeepAgent => write!(f, "keep_agent"),
            Self::RewriteUser => write!(f, "rewrite_user"),
            Self::CoAuthoredBy => write!(f, "co_authored_by"),
        }
    }
}

impl std::str::FromStr for AttributionMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "keep_agent" => Ok(Self::KeepAgent),
            "rewrite_user" => Ok(Self::RewriteUser),
            "co_authored_by" => Ok(Self::CoAuthoredBy),
            other => Err(format!("unknown attribution mode: {}", other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretScanMode {
    Block,
    WarnOnly,
}

impl std::fmt::Display for SecretScanMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Block => write!(f, "block"),
            Self::WarnOnly => write!(f, "warn_only"),
        }
    }
}

impl std::str::FromStr for SecretScanMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "block" => Ok(Self::Block),
            "warn_only" => Ok(Self::WarnOnly),
            other => Err(format!("unknown secret scan mode: {}", other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteProvider {
    Github,
    Gitlab,
    Bitbucket,
    Custom,
}

impl std::fmt::Display for RemoteProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Github => write!(f, "github"),
            Self::Gitlab => write!(f, "gitlab"),
            Self::Bitbucket => write!(f, "bitbucket"),
            Self::Custom => write!(f, "custom"),
        }
    }
}

impl std::str::FromStr for RemoteProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "github" => Ok(Self::Github),
            "gitlab" => Ok(Self::Gitlab),
            "bitbucket" => Ok(Self::Bitbucket),
            "custom" => Ok(Self::Custom),
            other => Err(format!("unknown remote provider: {}", other)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialType {
    Token,
    UsernamePassword,
}

impl std::fmt::Display for CredentialType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Token => write!(f, "token"),
            Self::UsernamePassword => write!(f, "username_password"),
        }
    }
}

impl std::str::FromStr for CredentialType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "token" => Ok(Self::Token),
            "username_password" => Ok(Self::UsernamePassword),
            other => Err(format!("unknown credential type: {}", other)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedRepo {
    pub id: ManagedRepoId,
    pub persona_id: PersonaId,
    pub workspace_path: String,
    pub mirror_path: String,
    pub remote_url: Option<String>,
    pub remote_provider: Option<RemoteProvider>,
    pub branch_strategy: BranchStrategy,
    pub branch_pattern: Option<String>,
    pub attribution_mode: AttributionMode,
    pub sync_mode: SyncMode,
    pub secret_scan_mode: SecretScanMode,
    pub check_interval_seconds: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoCredential {
    pub id: String,
    pub repo_id: ManagedRepoId,
    pub keyring_service_name: String,
    pub credential_type: CredentialType,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// --- Repo Sync Request Types ---

#[derive(Debug, Deserialize)]
pub struct EnableRepoRequest {
    pub persona_id: PersonaId,
    pub workspace_path: String,
    pub remote_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateRepoRequest {
    pub remote_url: Option<String>,
    pub remote_provider: Option<RemoteProvider>,
    pub branch_strategy: Option<BranchStrategy>,
    pub branch_pattern: Option<String>,
    pub attribution_mode: Option<AttributionMode>,
    pub sync_mode: Option<SyncMode>,
    pub secret_scan_mode: Option<SecretScanMode>,
    pub check_interval_seconds: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct SetCredentialsRequest {
    pub username: String,
    pub secret: String,
    pub credential_type: CredentialType,
}

// --- Repo Sync Response Types ---

#[derive(Debug, Clone, Serialize)]
pub struct ManagedRepoResponse {
    pub id: String,
    pub persona_id: String,
    pub persona_name: String,
    pub workspace_path: String,
    pub mirror_path: String,
    pub remote_url: Option<String>,
    pub remote_provider: Option<String>,
    pub branch_strategy: String,
    pub branch_pattern: Option<String>,
    pub attribution_mode: String,
    pub sync_mode: String,
    pub secret_scan_mode: String,
    pub check_interval_seconds: u32,
    pub sync_status: SyncStatus,
    pub credential_status: String,
    pub mirror_exists: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncStatus {
    pub workspace_ahead: u32,
    pub mirror_ahead: u32,
    pub remote_ahead: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct DetectedRepo {
    pub workspace_path: String,
    pub persona_id: String,
    pub persona_name: String,
    pub has_remotes: bool,
    pub remote_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CommitInfo {
    pub sha: String,
    pub message: String,
    pub author: String,
    pub timestamp: String,
    pub files_changed: u32,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    pub commits: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct PushResult {
    pub branch: String,
    pub commits: u32,
}
