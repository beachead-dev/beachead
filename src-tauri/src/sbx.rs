use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::time::timeout;

use crate::error::OrchestratorError;
use crate::types::AdditionalWorkspaceArg;

/// Commands that involve secrets — stderr from these is redacted before logging.
const SECRET_COMMANDS: &[&str] = &["secret"];

/// Default timeout for sbx CLI commands. Prevents hangs when the daemon is stopped.
const SBX_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// Extended timeout for sbx create/run — these can pull images on first use.
const SBX_CREATE_TIMEOUT: Duration = Duration::from_secs(90);

/// Result of executing an sbx CLI command.
#[derive(Debug, Clone)]
pub struct SbxOutput {
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

/// sbx version information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SbxVersion {
    pub version: String,
}

/// Result of `sbx diagnose`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnoseResult {
    pub raw_output: String,
    pub json: Option<serde_json::Value>,
}

/// Information about a running sandbox from `sbx ls --json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SandboxInfo {
    pub name: Option<String>,
    pub id: Option<String>,
    pub status: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Value,
}

/// Arguments for `sbx run`.
#[derive(Debug, Clone)]
pub struct SbxRunArgs {
    pub agent: String,
    pub kit_paths: Vec<PathBuf>,
    pub workspace: PathBuf,
    pub name: Option<String>,
    pub template: Option<String>,
    pub agent_args: Vec<String>,
}

/// Arguments for `sbx create`.
#[derive(Debug, Clone)]
pub struct SbxCreateArgs {
    pub agent: String,
    pub kit_paths: Vec<PathBuf>,
    pub workspace: PathBuf,
    pub name: Option<String>,
    pub template: Option<String>,
    pub additional_workspaces: Vec<AdditionalWorkspaceArg>,
}

/// Port mapping from `sbx ports`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortMapping {
    pub host_ip: String,
    pub host_port: u16,
    pub sandbox_port: u16,
    pub protocol: String,
}

/// Policy state from `sbx policy`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyState {
    pub default_policy: String,
    pub rules: Vec<PolicyRule>,
}

/// A single policy rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    pub id: Option<String>,
    pub action: String,
    pub target: String,
    /// APPLIES_TO: "all" for global rules or "sandbox:<name>" for per-sandbox rules.
    #[serde(default)]
    pub origin: Option<String>,
    /// PROVENANCE: "local" for user-added rules, "kit" for kit-originated rules.
    #[serde(default)]
    pub provenance: Option<String>,
    /// TYPE: rule type (e.g. "network").
    #[serde(default)]
    pub rule_type: Option<String>,
    /// STATUS: "active" or "inactive".
    #[serde(default)]
    pub status: Option<String>,
}

/// Internal deserialization target for the real sbx 0.35.0 `sbx policy ls --json`
/// output. Private to `sbx.rs` — this is NOT the frontend-facing type; rules are
/// flattened/mapped into [`PolicyState`]/[`PolicyRule`] before leaving this module.
///
/// The 0.35.0 top-level object contains only `rules` (there is no `default_policy`
/// / mode field — the default mode is set via `sbx policy init` and is not read
/// back by `sbx policy ls`).
#[derive(Deserialize)]
struct SbxPolicyLs {
    #[serde(default)]
    rules: Vec<SbxPolicyLsRule>,
}

/// A single rule as emitted by `sbx policy ls --json` on sbx 0.35.0.
///
/// Verified against real `sbx version: v0.35.0` output. Only `decision` is
/// guaranteed present; every other field is optional (or defaulted) so that
/// deserialization is resilient to rules that omit fields.
///
/// Only the fields Beachead actually consumes are declared; serde ignores any
/// other keys in the JSON (e.g. `name`, `policy_id`, `scope`, `sandbox_id`).
/// Per-sandbox scope for removal is derived from `applies_to`, so `sandbox_id`
/// is not needed here.
#[derive(Deserialize, Debug)]
struct SbxPolicyLsRule {
    #[serde(default)]
    id: Option<String>,
    /// "all" | "sandbox:<name>"
    #[serde(default)]
    applies_to: Option<String>,
    /// "network" | "filesystem" | ...
    #[serde(default)]
    resource_type: Option<String>,
    /// "allow" | "deny" — required.
    decision: String,
    /// A single rule can carry many resources.
    #[serde(default)]
    resources: Vec<String>,
    /// "local" (global) | "scoped" (per-sandbox) — a scope indicator, not a
    /// source/provenance field.
    #[serde(default)]
    origin: Option<String>,
    #[serde(default)]
    status: Option<String>,
}

/// Default policy mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDefault {
    Allow,
    Deny,
    Balanced,
}

impl std::fmt::Display for PolicyDefault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Allow => write!(f, "allow-all"),
            Self::Deny => write!(f, "deny-all"),
            Self::Balanced => write!(f, "balanced"),
        }
    }
}

/// Policy log entry from `sbx policy log`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyLogEntry {
    pub timestamp: Option<String>,
    pub sandbox: Option<String>,
    pub host: Option<String>,
    pub action: Option<String>,
    pub proxy: Option<String>,
    pub rule: Option<String>,
    pub reason: Option<String>,
}

/// Template information from `sbx template ls`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateInfo {
    pub tag: String,
    pub size: Option<String>,
    pub created: Option<String>,
}

/// Kit validation result from `sbx kit validate`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KitValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
}

/// Secret status from `sbx secret ls`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SbxSecretStatus {
    pub service: String,
    pub configured: bool,
}

/// The sbx CLI wrapper. All Docker Sandbox operations go through this struct.
///
/// SECURITY: All commands are constructed using `Command::arg()` for each argument.
/// Shell string interpolation is never used to prevent command injection.
pub struct SbxCli {
    sbx_path: PathBuf,
}

/// Build the argument vector for `sbx create` from the given args.
/// This is extracted as a standalone function for testability.
pub fn build_create_args(args: &SbxCreateArgs) -> Vec<String> {
    let mut cmd_args: Vec<String> = Vec::new();

    // Flags must come before positional args per new CLI syntax:
    // sbx create [flags] AGENT PATH [PATH...]
    //
    // NOTE: We intentionally do NOT pass `-q/--quiet`. As of sbx 0.35.0, `-q`
    // suppresses ALL output (including the sandbox name), leaving stdout empty
    // so we can't recover the created sandbox's name. Without `-q`, stdout
    // contains a parseable `Created sandbox '<name>'` line (plus a
    // `sbx run --name <name>` hint), which `extract_sandbox_name` handles.

    for kit_path in &args.kit_paths {
        cmd_args.push("--kit".to_string());
        cmd_args.push(kit_path.to_string_lossy().to_string());
    }

    if let Some(name) = &args.name {
        cmd_args.push("--name".to_string());
        cmd_args.push(name.clone());
    }

    if let Some(template) = &args.template {
        cmd_args.push("-t".to_string());
        cmd_args.push(template.clone());
    }

    // Agent (positional)
    cmd_args.push(args.agent.clone());

    // Workspace path as positional argument
    cmd_args.push(args.workspace.to_string_lossy().to_string());

    // Additional workspace paths as separate positional arguments
    for ws in &args.additional_workspaces {
        let path_str = ws.path.to_string_lossy().to_string();
        if ws.read_only {
            cmd_args.push(format!("{}:ro", path_str));
        } else {
            cmd_args.push(path_str);
        }
    }

    cmd_args
}

impl SbxCli {
    /// Create a new SbxCli instance by resolving the `sbx` binary from PATH.
    pub fn new() -> Result<Self, OrchestratorError> {
        let sbx_path = Self::resolve_from_path()?;
        Ok(Self { sbx_path })
    }

    /// Create an SbxCli with an explicit path to the sbx binary (useful for testing).
    #[cfg(test)]
    pub fn with_path(sbx_path: PathBuf) -> Self {
        Self { sbx_path }
    }

    /// Get the path to the sbx binary.
    pub fn path(&self) -> &Path {
        &self.sbx_path
    }

    /// Resolve the `sbx` binary location from the system PATH.
    /// Works across Linux, macOS, and Windows.
    pub fn resolve_from_path() -> Result<PathBuf, OrchestratorError> {
        let binary_name = if cfg!(target_os = "windows") {
            "sbx.exe"
        } else {
            "sbx"
        };

        which_binary(binary_name).ok_or_else(|| {
            OrchestratorError::SbxError(
                "sbx CLI not found on system PATH. \
                 Please install Docker Sandboxes: https://docs.docker.com/ai/sandboxes/get-started/"
                    .to_string(),
            )
        })
    }

    /// Internal helper to execute an sbx command and capture output.
    ///
    /// SECURITY:
    /// - Uses `Command::arg()` for each argument (no shell interpolation).
    /// - Redacts stderr for secret-related commands before any logging.
    async fn exec_command(
        &self,
        subcommand: &str,
        args: &[&str],
    ) -> Result<SbxOutput, OrchestratorError> {
        let mut cmd = Command::new(&self.sbx_path);
        cmd.arg(subcommand);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = timeout(SBX_COMMAND_TIMEOUT, cmd.output())
            .await
            .map_err(|_| OrchestratorError::SbxTimeout(format!("sbx {} timed out", subcommand)))?
            .map_err(|e| {
                OrchestratorError::SbxError(format!("Failed to execute sbx {}: {}", subcommand, e))
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Redact stderr for secret-related commands before logging
        let is_secret_cmd = SECRET_COMMANDS.iter().any(|s| subcommand.contains(s));
        if !is_secret_cmd && !stderr.is_empty() && !output.status.success() {
            eprintln!("sbx {} stderr: {}", subcommand, stderr.trim());
        }

        Ok(SbxOutput {
            stdout,
            stderr,
            success: output.status.success(),
        })
    }

    /// Execute an sbx command with owned String args.
    /// Convenience wrapper around exec_command for dynamic arg lists.
    async fn exec_command_owned(
        &self,
        subcommand: &str,
        args: &[String],
    ) -> Result<SbxOutput, OrchestratorError> {
        self.exec_command_owned_with_timeout(subcommand, args, SBX_COMMAND_TIMEOUT)
            .await
    }

    /// Execute an sbx command with owned String args and a custom timeout.
    async fn exec_command_owned_with_timeout(
        &self,
        subcommand: &str,
        args: &[String],
        cmd_timeout: Duration,
    ) -> Result<SbxOutput, OrchestratorError> {
        let mut cmd = Command::new(&self.sbx_path);
        cmd.arg(subcommand);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = timeout(cmd_timeout, cmd.output())
            .await
            .map_err(|_| OrchestratorError::SbxTimeout(format!("sbx {} timed out", subcommand)))?
            .map_err(|e| {
                OrchestratorError::SbxError(format!("Failed to execute sbx {}: {}", subcommand, e))
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        let is_secret_cmd = SECRET_COMMANDS.iter().any(|s| subcommand.contains(s));
        if !is_secret_cmd && !stderr.is_empty() && !output.status.success() {
            eprintln!("sbx {} stderr: {}", subcommand, stderr.trim());
        }

        Ok(SbxOutput {
            stdout,
            stderr,
            success: output.status.success(),
        })
    }

    /// Execute a multi-part sbx command (e.g., "policy allow network").
    /// The first element of `subcommands` is the primary subcommand,
    /// subsequent elements are sub-subcommands placed before `args`.
    async fn exec_multi_command(
        &self,
        subcommands: &[&str],
        args: &[&str],
    ) -> Result<SbxOutput, OrchestratorError> {
        let mut cmd = Command::new(&self.sbx_path);
        for sub in subcommands {
            cmd.arg(sub);
        }
        for arg in args {
            cmd.arg(arg);
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let label = subcommands.join(" ");
        let output = timeout(SBX_COMMAND_TIMEOUT, cmd.output())
            .await
            .map_err(|_| OrchestratorError::SbxTimeout(format!("sbx {} timed out", label)))?
            .map_err(|e| {
                OrchestratorError::SbxError(format!("Failed to execute sbx {}: {}", label, e))
            })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        let is_secret_cmd = subcommands.iter().any(|s| SECRET_COMMANDS.contains(s));
        if !is_secret_cmd && !stderr.is_empty() && !output.status.success() {
            eprintln!("sbx {} stderr: {}", label, stderr.trim());
        }

        Ok(SbxOutput {
            stdout,
            stderr,
            success: output.status.success(),
        })
    }

    // ─── Diagnostics (Task 3.1) ───────────────────────────────────────────

    /// Get the sbx CLI version.
    pub async fn version(&self) -> Result<SbxVersion, OrchestratorError> {
        let output = self.exec_command("version", &[]).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx version failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(SbxVersion {
            version: output.stdout.trim().to_string(),
        })
    }

    /// Run `sbx diagnose` and return the raw output plus parsed JSON if available.
    pub async fn diagnose(&self) -> Result<DiagnoseResult, OrchestratorError> {
        let output = self.exec_command("diagnose", &[]).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx diagnose failed: {}",
                output.stderr.trim()
            )));
        }
        let json = serde_json::from_str(&output.stdout).ok();
        Ok(DiagnoseResult {
            raw_output: output.stdout.clone(),
            json,
        })
    }

    // ─── Sandbox Lifecycle (Task 3.2) ─────────────────────────────────────

    /// Run a new sandbox: `sbx run <agent> --kit <path> -v <workspace> ...`
    ///
    /// Constructs the command with support for:
    /// - Multiple `--kit` flags
    /// - `-t` template flag
    /// - workspace path as positional argument
    /// - `--` separator followed by agent CLI args
    pub async fn run(&self, args: &SbxRunArgs) -> Result<String, OrchestratorError> {
        let mut cmd_args: Vec<String> = Vec::new();

        // Flags must come before positional args per new CLI syntax:
        // sbx run [flags] AGENT [PATH...] [-- AGENT_ARGS...]

        // Kit paths
        for kit_path in &args.kit_paths {
            cmd_args.push("--kit".to_string());
            cmd_args.push(kit_path.to_string_lossy().to_string());
        }

        // Optional name
        if let Some(name) = &args.name {
            cmd_args.push("--name".to_string());
            cmd_args.push(name.clone());
        }

        // Optional template
        if let Some(template) = &args.template {
            cmd_args.push("-t".to_string());
            cmd_args.push(template.clone());
        }

        // Agent identifier (positional)
        cmd_args.push(args.agent.clone());

        // Workspace path (positional argument after agent)
        cmd_args.push(args.workspace.to_string_lossy().to_string());

        // Agent args after separator
        if !args.agent_args.is_empty() {
            cmd_args.push("--".to_string());
            cmd_args.extend(args.agent_args.clone());
        }

        let output = self.exec_command_owned("run", &cmd_args).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx run failed: {}",
                output.stderr.trim()
            )));
        }

        // sbx run typically outputs the sandbox ID on stdout
        Ok(output.stdout.trim().to_string())
    }

    /// Re-attach to an existing sandbox by name: `sbx run --name <name>`
    ///
    /// Unlike `run()`, this does not pass agent, workspace, or kit args.
    /// Used when the sandbox already exists and just needs to be started/re-entered.
    pub async fn reattach(&self, name: &str) -> Result<String, OrchestratorError> {
        let cmd_args = vec!["--name".to_string(), name.to_string()];
        let output = self.exec_command_owned("run", &cmd_args).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx run --name failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(output.stdout.trim().to_string())
    }

    /// Create a sandbox without starting it: `sbx create <agent> --kit <path> -v <workspace>`
    ///
    /// Uses an extended timeout (90s) since first-time creation may pull images.
    pub async fn create(&self, args: &SbxCreateArgs) -> Result<String, OrchestratorError> {
        let cmd_args = build_create_args(args);

        let output = self
            .exec_command_owned_with_timeout("create", &cmd_args, SBX_CREATE_TIMEOUT)
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx create failed: {}",
                output.stderr.trim()
            )));
        }

        // stdout contains image-pull progress plus a `Created sandbox 'NAME'`
        // line (and a `sbx run --name NAME` hint); extract_sandbox_name parses it.
        let sandbox_name = extract_sandbox_name(&output.stdout);
        if sandbox_name.is_empty() {
            return Err(OrchestratorError::SbxError(
                "sbx create did not return a sandbox name".to_string(),
            ));
        }

        Ok(sandbox_name)
    }

    /// Stop a sandbox: `sbx stop <sandbox_id>`
    pub async fn stop(&self, sandbox_id: &str) -> Result<(), OrchestratorError> {
        let output = self.exec_command("stop", &[sandbox_id]).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx stop failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    /// Remove a sandbox: `sbx rm <sandbox_id>`
    pub async fn rm(&self, sandbox_id: &str) -> Result<(), OrchestratorError> {
        let output = self.exec_command("rm", &["--force", sandbox_id]).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx rm failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    /// List sandboxes as JSON: `sbx ls --json`
    pub async fn ls_json(&self) -> Result<Vec<SandboxInfo>, OrchestratorError> {
        let output = self.exec_command("ls", &["--json"]).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx ls failed: {}",
                output.stderr.trim()
            )));
        }

        // sbx ls --json may return either:
        // - A plain array: [{"name": "...", ...}, ...]
        // - A wrapper object: {"sandboxes": [...]}
        if let Ok(sandboxes) = serde_json::from_str::<Vec<SandboxInfo>>(&output.stdout) {
            return Ok(sandboxes);
        }

        // Try wrapper object format
        #[derive(serde::Deserialize)]
        struct SbxLsWrapper {
            sandboxes: Vec<SandboxInfo>,
        }
        if let Ok(wrapper) = serde_json::from_str::<SbxLsWrapper>(&output.stdout) {
            return Ok(wrapper.sandboxes);
        }

        Err(OrchestratorError::SbxError(format!(
            "Failed to parse sbx ls JSON output: {}",
            output.stdout.chars().take(200).collect::<String>()
        )))
    }

    /// Copy files to/from a sandbox: `sbx cp <src> <dst>`
    /// One side must use the format `SANDBOX:PATH`.
    pub async fn cp(&self, src: &str, dst: &str) -> Result<(), OrchestratorError> {
        let output = self.exec_command("cp", &[src, dst]).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx cp failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    // ─── Kit Management (Task 3.3) ───────────────────────────────────────

    /// Validate a kit: `sbx kit validate <kit_path>`
    pub async fn kit_validate(
        &self,
        kit_path: &Path,
    ) -> Result<KitValidationResult, OrchestratorError> {
        let path_str = kit_path.to_string_lossy();
        let output = self
            .exec_multi_command(&["kit", "validate"], &[&path_str])
            .await?;

        if output.success {
            Ok(KitValidationResult {
                valid: true,
                errors: Vec::new(),
            })
        } else {
            // Parse validation errors from stderr/stdout
            let errors: Vec<String> = output
                .stderr
                .lines()
                .chain(output.stdout.lines())
                .filter(|l| !l.is_empty())
                .map(|l| l.to_string())
                .collect();

            Ok(KitValidationResult {
                valid: false,
                errors,
            })
        }
    }

    // ─── Port Management (Task 3.4) ──────────────────────────────────────

    /// List published ports for a sandbox: `sbx ports <sandbox_id>`
    pub async fn ports_list(
        &self,
        sandbox_id: &str,
    ) -> Result<Vec<PortMapping>, OrchestratorError> {
        let output = self.exec_command("ports", &[sandbox_id]).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx ports failed: {}",
                output.stderr.trim()
            )));
        }

        // Parse port listing output. Expected format per line:
        // <host_ip>:<host_port> -> <sandbox_port>/<protocol>
        let mappings = parse_port_output(&output.stdout);
        Ok(mappings)
    }

    /// Publish a port for a sandbox: `sbx ports <sandbox_id> --publish <port_spec>`
    pub async fn ports_publish(
        &self,
        sandbox_id: &str,
        port_spec: &str,
    ) -> Result<PortMapping, OrchestratorError> {
        let output = self
            .exec_command("ports", &[sandbox_id, "--publish", port_spec])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx ports --publish failed: {}",
                output.stderr.trim()
            )));
        }

        // Parse the single port mapping from output
        let mappings = parse_port_output(&output.stdout);
        mappings.into_iter().next().ok_or_else(|| {
            OrchestratorError::SbxError("sbx ports --publish returned no port mapping".to_string())
        })
    }

    /// Unpublish a port for a sandbox: `sbx ports <sandbox_id> --unpublish <port_spec>`
    pub async fn ports_unpublish(
        &self,
        sandbox_id: &str,
        port_spec: &str,
    ) -> Result<(), OrchestratorError> {
        let output = self
            .exec_command("ports", &[sandbox_id, "--unpublish", port_spec])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx ports --unpublish failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    // ─── Policy Management (Task 3.5) ────────────────────────────────────

    /// List current policy state via `sbx policy ls --json` (sbx 0.35.0+).
    ///
    /// The 0.35.0 JSON shape is `{ "rules": [ { decision, resources[],
    /// applies_to, resource_type, origin, status, sandbox_id? } ] }` with no
    /// top-level `default_policy` field. Each JSON rule can carry multiple
    /// `resources`, so we flatten to one [`PolicyRule`] per resource, preserving
    /// the frontend "one row per resource" contract:
    /// - `id`         ← rule `id`
    /// - `action`     ← `decision`
    /// - `target`     ← each `resources` element
    /// - `origin`     ← `applies_to` ("all" | "sandbox:<name>")
    /// - `provenance` ← JSON `origin` ("local" | "scoped" scope indicator)
    /// - `rule_type`  ← `resource_type`
    /// - `status`     ← `status`
    ///
    /// JSON is authoritative: a parse failure returns an `SbxError` describing
    /// the failure (no text fallback).
    pub async fn policy_ls(&self) -> Result<PolicyState, OrchestratorError> {
        let output = self
            .exec_multi_command(&["policy", "ls"], &["--json"])
            .await?;
        if !output.success {
            let stderr = output.stderr.trim();
            // Version-compat: an sbx older than 0.35.0 does not support the
            // `--json` flag and fails with an unknown-flag / usage style error.
            // Surface a clear, actionable minimum-version message instead of an
            // opaque failure (Requirement 4.2).
            if Self::is_json_flag_unsupported(stderr) {
                return Err(OrchestratorError::SbxError(
                    "sbx policy ls --json is unavailable; Beachead requires sbx 0.35.0 or later"
                        .to_string(),
                ));
            }
            return Err(OrchestratorError::SbxError(format!(
                "sbx policy ls failed: {}",
                stderr
            )));
        }

        // JSON is authoritative — no text fallback. A parse failure is a loud error.
        let parsed: SbxPolicyLs = serde_json::from_str(&output.stdout).map_err(|e| {
            OrchestratorError::SbxError(format!(
                "failed to parse sbx policy ls --json output: {}",
                e
            ))
        })?;

        // Flatten one PolicyRule per resource in each JSON rule.
        let rules = Self::flatten_policy_rules(&parsed.rules);

        // Derive `default_policy` best-effort from the JSON rules.
        //
        // NOTE: this value is INFERRED, not read. sbx 0.35.0's `sbx policy ls`
        // (and its `--json`) no longer exposes the global default-policy mode —
        // that mode is set via `sbx policy init <allow-all|balanced|deny-all>`
        // and is not reported back by the listing. So we infer a best-effort
        // label from the rules that ARE present, using the following order:
        //   1. Any built-in rule (id starts with "default-") → "balanced"
        //      (the balanced preset ships as the `default-*` rule set).
        //   2. A global (applies_to == "all") network allow rule targeting an
        //      "all traffic" resource ("*" / "0.0.0.0/0") → "allow-all".
        //   3. No global network allow rules at all → "deny-all".
        //   4. Otherwise → "balanced" (fallback).
        let default_policy = Self::derive_default_policy(&parsed.rules);

        Ok(PolicyState {
            default_policy,
            rules,
        })
    }

    /// Flatten the JSON rules from `sbx policy ls --json` into the frontend-facing
    /// [`PolicyRule`] list: one `PolicyRule` per resource in each JSON rule.
    ///
    /// This is the pure mapping at the heart of `policy_ls()`, extracted so it can
    /// be exercised directly (including by property tests). The field mapping is:
    /// - `id`         ← rule `id` (shared by every row from a multi-resource rule)
    /// - `action`     ← `decision`
    /// - `target`     ← each individual `resources` element
    /// - `origin`     ← `applies_to` ("all" | "sandbox:<name>")
    /// - `provenance` ← JSON `origin` ("local" | "scoped")
    /// - `rule_type`  ← `resource_type`
    /// - `status`     ← `status`
    ///
    /// A rule with an empty `resources` array produces zero rows.
    fn flatten_policy_rules(parsed_rules: &[SbxPolicyLsRule]) -> Vec<PolicyRule> {
        let mut rules: Vec<PolicyRule> = Vec::new();
        for rule in parsed_rules {
            for resource in &rule.resources {
                rules.push(PolicyRule {
                    id: rule.id.clone(),
                    action: rule.decision.clone(),
                    target: resource.clone(),
                    origin: rule.applies_to.clone(),
                    provenance: rule.origin.clone(),
                    rule_type: rule.resource_type.clone(),
                    status: rule.status.clone(),
                });
            }
        }
        rules
    }

    /// Detect whether a non-zero `sbx policy ls --json` failure is caused by the
    /// installed sbx not supporting the `--json` flag (i.e. an sbx older than
    /// 0.35.0), as opposed to some other runtime failure.
    ///
    /// The sbx CLI (built on a cobra-style argument parser) prints an
    /// "unknown flag" / usage-style error that references the offending flag.
    /// Phrasing varies across CLI frameworks and versions, so we match
    /// reasonably robustly: the message must reference `--json` AND look like a
    /// flag/usage complaint. This keeps genuine runtime failures (network,
    /// daemon, permissions) on the generic error path.
    fn is_json_flag_unsupported(stderr: &str) -> bool {
        let lower = stderr.to_lowercase();
        if !lower.contains("--json") {
            return false;
        }
        lower.contains("unknown flag")
            || lower.contains("unknown option")
            || lower.contains("unknown shorthand flag")
            || lower.contains("unrecognized flag")
            || lower.contains("unrecognized option")
            || lower.contains("flag provided but not defined")
            || lower.contains("invalid flag")
            || lower.contains("usage")
    }

    /// Best-effort inference of the global default-policy mode from the rules
    /// reported by `sbx policy ls --json`.
    ///
    /// sbx 0.35.0 removed the default-mode field from the policy listing (it is
    /// set via `sbx policy init` and not read back), so this label is INFERRED
    /// from the present rules rather than read from the tool. Returns one of
    /// `"balanced"`, `"allow-all"`, or `"deny-all"` (matching the casing used by
    /// `PolicyDefault`'s `Display`).
    fn derive_default_policy(rules: &[SbxPolicyLsRule]) -> String {
        // 1. Built-in balanced rule set present.
        if rules
            .iter()
            .any(|r| r.id.as_deref().is_some_and(|id| id.starts_with("default-")))
        {
            return "balanced".to_string();
        }

        let is_global = |r: &SbxPolicyLsRule| r.applies_to.as_deref() == Some("all");
        let is_network_allow = |r: &SbxPolicyLsRule| {
            r.decision == "allow" && r.resource_type.as_deref() == Some("network")
        };

        // 2. A global network allow rule targeting an "all traffic" resource.
        let allows_all_traffic = rules.iter().any(|r| {
            is_global(r)
                && is_network_allow(r)
                && r.resources
                    .iter()
                    .any(|res| res == "*" || res == "0.0.0.0/0")
        });
        if allows_all_traffic {
            return "allow-all".to_string();
        }

        // 3. No global network allow rules at all → deny-all.
        let has_global_network_allow = rules.iter().any(|r| is_global(r) && is_network_allow(r));
        if !has_global_network_allow {
            return "deny-all".to_string();
        }

        // 4. Fallback.
        "balanced".to_string()
    }

    /// Set the global default policy: `sbx policy init <mode>`.
    ///
    /// The CLI command is `init` (renamed from `set-default` in sbx 0.34.0; the
    /// old name is a deprecated alias slated for removal). The Rust method name
    /// is kept for API stability. Accepted modes: `allow-all` / `balanced` /
    /// `deny-all` (from `PolicyDefault`'s `Display`).
    pub async fn policy_set_default(&self, mode: PolicyDefault) -> Result<(), OrchestratorError> {
        let mode_str = mode.to_string();
        let output = self
            .exec_multi_command(&["policy", "init"], &[&mode_str])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx policy init failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    /// Allow network access globally (all sandboxes): `sbx policy allow network "<target>"`
    ///
    /// As of sbx 0.32.0, global scope is the default (no flag needed).
    pub async fn policy_allow_network(&self, target: &str) -> Result<(), OrchestratorError> {
        let output = self
            .exec_multi_command(&["policy", "allow", "network"], &[target])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx policy allow network failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    /// Allow network access scoped to a specific sandbox:
    /// `sbx policy allow network --sandbox <name> "<target>"`
    ///
    /// The rule is automatically cleaned up when the sandbox is removed.
    pub async fn policy_allow_network_for_sandbox(
        &self,
        sandbox_name: &str,
        target: &str,
    ) -> Result<(), OrchestratorError> {
        let output = self
            .exec_multi_command(
                &["policy", "allow", "network"],
                &["--sandbox", sandbox_name, target],
            )
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx policy allow network (sandbox '{}') failed: {}",
                sandbox_name,
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    /// Deny network access globally (all sandboxes): `sbx policy deny network "<target>"`
    ///
    /// As of sbx 0.32.0, global scope is the default (no flag needed).
    pub async fn policy_deny_network(&self, target: &str) -> Result<(), OrchestratorError> {
        let output = self
            .exec_multi_command(&["policy", "deny", "network"], &[target])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx policy deny network failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    /// Derive the removal scope for a rule from its `origin` (which maps from the
    /// JSON `applies_to`): per-sandbox rules (`origin == "sandbox:<name>"`) yield
    /// `Some("<name>")` to be passed as `--sandbox <name>`; global rules
    /// (`origin == "all"` or any non-`sandbox:` value, including `None`) yield
    /// `None` (no sandbox flag — global is the default scope).
    ///
    /// Extracted as a pure function so scope derivation can be tested directly.
    fn derive_removal_scope(origin: Option<&str>) -> Option<String> {
        match origin {
            Some(origin) if origin.starts_with("sandbox:") => {
                Some(origin.strip_prefix("sandbox:").unwrap().to_string())
            }
            _ => None,
        }
    }

    /// Remove a policy rule by the `id` surfaced in `policy_ls()`.
    ///
    /// Resolves the rule from the JSON-parsed `policy_ls()` result (sbx 0.35.0
    /// `sbx policy ls --json`) and issues a correctly-scoped
    /// `sbx policy rm network [--sandbox <name>] --id <rule_id>`.
    ///
    /// Removal keys on `--id`, NOT `--resource`:
    /// - DEVIATION (Requirement 2.1): the requirement is phrased around resolving
    ///   the rule's `origin` and `target`. Both ARE resolved from the JSON — `origin`
    ///   drives sandbox scoping and `target` provides error/log context — but the
    ///   actual removal keys on `--id`. In sbx 0.35.0 the JSON `id` is a stable,
    ///   unique rule identifier accepted by `sbx policy rm network --id` (verified
    ///   via `sbx policy rm network --help`). This supersedes the older comment that
    ///   claimed the displayed value was a policy-scoped name rather than the UUID;
    ///   in 0.35.0 the `id` IS the stable unique identifier. For every single-resource
    ///   rule created through Beachead's UI, `--id` removal is exactly equivalent to
    ///   per-resource removal, and it avoids the ambiguity of a multi-resource rule
    ///   that flattens into several UI rows sharing one `id`.
    /// - KNOWN LIMITATION: per-resource removal within a multi-resource rule (e.g. a
    ///   kit-originated scoped rule carrying several resources) is not selectable
    ///   through the id-only API — removing by `--id` drops the whole rule. This is
    ///   documented and out of scope for this fix.
    ///
    /// Scoping:
    /// - Global rules (origin != "sandbox:<name>"): no scope flag (global is default)
    /// - Per-sandbox rules (origin == "sandbox:<name>"): --sandbox <name>
    pub async fn policy_remove_rule(&self, rule_id: &str) -> Result<(), OrchestratorError> {
        // Resolve the rule from the JSON-parsed policy state to determine scope.
        let state = self.policy_ls().await?;
        let rule = state
            .rules
            .iter()
            .find(|r| r.id.as_deref() == Some(rule_id));

        let rule = match rule {
            Some(r) => r.clone(),
            None => {
                return Err(OrchestratorError::NotFound(format!(
                    "Policy rule '{}' not found",
                    rule_id
                )));
            }
        };

        // Determine scope from the resolved rule's origin (applies_to):
        // strip the "sandbox:" prefix for the --sandbox value; global rules omit it.
        let sandbox_name = Self::derive_removal_scope(rule.origin.as_deref());

        // Build args entirely via Command::arg() (through exec_multi_command) —
        // no shell interpolation (Requirement 2.5).
        let mut args: Vec<&str> = Vec::new();
        let sandbox_ref;
        if let Some(ref name) = sandbox_name {
            sandbox_ref = name.as_str();
            args.extend_from_slice(&["--sandbox", sandbox_ref]);
        }
        // Remove by stable, unique id (see DEVIATION note above).
        args.extend_from_slice(&["--id", rule_id]);

        let output = self
            .exec_multi_command(&["policy", "rm", "network"], &args)
            .await?;
        if !output.success {
            Err(OrchestratorError::SbxError(format!(
                "sbx policy rm failed: {}",
                output.stderr.trim()
            )))
        } else {
            Ok(())
        }
    }

    /// Get policy traffic log: `sbx policy log [SANDBOX] [--limit <n>]`
    pub async fn policy_log(
        &self,
        sandbox_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<PolicyLogEntry>, OrchestratorError> {
        let mut args: Vec<String> = Vec::new();
        // Sandbox is a positional argument (comes before flags)
        if let Some(id) = sandbox_id {
            args.push(id.to_string());
        }
        if let Some(n) = limit {
            args.push("--limit".to_string());
            args.push(n.to_string());
        }

        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let output = self
            .exec_multi_command(&["policy", "log"], &arg_refs)
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx policy log failed: {}",
                output.stderr.trim()
            )));
        }

        // Attempt JSON parse
        let entries: Vec<PolicyLogEntry> = serde_json::from_str(&output.stdout).unwrap_or_default();
        Ok(entries)
    }

    /// Reset all policy rules: `sbx policy reset --force`
    pub async fn policy_reset(&self) -> Result<(), OrchestratorError> {
        let output = self
            .exec_multi_command(&["policy", "reset"], &["--force"])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx policy reset failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    // ─── Secret Management (Task 3.6) ────────────────────────────────────
    //
    // SECURITY: Secret values are passed via Command::arg() and never logged.
    // stderr from secret commands is redacted (see SECRET_COMMANDS filter).
    // Values are zeroized after being passed to the CLI.

    /// List configured secrets: `sbx secret ls`
    pub async fn secret_ls(&self) -> Result<Vec<SbxSecretStatus>, OrchestratorError> {
        let output = self.exec_multi_command(&["secret", "ls"], &[]).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(
                "sbx secret ls failed".to_string(),
            ));
        }

        // Attempt JSON parse first
        if let Ok(secrets) = serde_json::from_str::<Vec<SbxSecretStatus>>(&output.stdout) {
            return Ok(secrets);
        }

        // Fallback: parse text output line by line
        Ok(parse_secret_ls_text(&output.stdout))
    }

    /// Set a global secret: `sbx secret set -g <service> -t <value>`
    ///
    /// SECURITY: The value is passed as a single arg and never logged.
    pub async fn secret_set(&self, service: &str, value: &str) -> Result<(), OrchestratorError> {
        let output = self
            .exec_multi_command(&["secret", "set"], &["-g", service, "-t", value])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(
                "sbx secret set failed".to_string(),
            ));
        }
        Ok(())
    }

    /// Initiate OAuth flow for a service: `sbx secret set -g <service> --oauth`
    pub async fn secret_set_oauth(&self, service: &str) -> Result<(), OrchestratorError> {
        let output = self
            .exec_multi_command(&["secret", "set"], &["-g", service, "--oauth"])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(
                "sbx secret set --oauth failed".to_string(),
            ));
        }
        Ok(())
    }

    /// Remove a secret: `sbx secret rm -g <service> -f`
    pub async fn secret_rm(&self, service: &str) -> Result<(), OrchestratorError> {
        let output = self
            .exec_multi_command(&["secret", "rm"], &["-g", service, "-f"])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(
                "sbx secret rm failed".to_string(),
            ));
        }
        Ok(())
    }

    // ─── Template Management (Task 3.7) ──────────────────────────────────

    /// List saved templates: `sbx template ls`
    pub async fn template_ls(&self) -> Result<Vec<TemplateInfo>, OrchestratorError> {
        let output = self.exec_multi_command(&["template", "ls"], &[]).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx template ls failed: {}",
                output.stderr.trim()
            )));
        }

        // Attempt JSON parse, fall back to text parsing
        if let Ok(templates) = serde_json::from_str::<Vec<TemplateInfo>>(&output.stdout) {
            return Ok(templates);
        }

        Ok(parse_template_ls_text(&output.stdout))
    }

    /// Save a sandbox as a template: `sbx template save <sandbox_id> <tag> [--output <file.tar>]`
    pub async fn template_save(
        &self,
        sandbox_id: &str,
        tag: &str,
        output_tar: Option<&Path>,
    ) -> Result<(), OrchestratorError> {
        let mut args = vec![sandbox_id, tag];
        let tar_str;
        if let Some(tar_path) = output_tar {
            tar_str = tar_path.to_string_lossy().to_string();
            args.push("--output");
            args.push(&tar_str);
        }

        let output = self
            .exec_multi_command(&["template", "save"], &args)
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx template save failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    /// Load a template from a tar file: `sbx template load <file.tar>`
    pub async fn template_load(&self, tar_path: &Path) -> Result<(), OrchestratorError> {
        let path_str = tar_path.to_string_lossy();
        let output = self
            .exec_multi_command(&["template", "load"], &[&path_str])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx template load failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    /// Remove a template: `sbx template rm <tag>`
    pub async fn template_rm(&self, tag: &str) -> Result<(), OrchestratorError> {
        let output = self.exec_multi_command(&["template", "rm"], &[tag]).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx template rm failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    // ─── Auth Management (Task 3.8) ──────────────────────────────────────

    /// Initiate Docker login (opens browser for OAuth): `sbx login`
    pub async fn login(&self) -> Result<(), OrchestratorError> {
        let output = self.exec_command("login", &[]).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx login failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    /// Sign out of Docker: `sbx logout`
    pub async fn logout(&self) -> Result<(), OrchestratorError> {
        // --yes skips the interactive confirmation prompt (stdin is piped, not a TTY)
        let output = self.exec_command("logout", &["--yes"]).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx logout failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    /// Build the argument list for `sbx run` without executing.
    /// Useful for testing command construction (Property 7).
    pub fn build_run_args(args: &SbxRunArgs) -> Vec<String> {
        let mut cmd_args: Vec<String> = Vec::new();

        cmd_args.push("run".to_string());
        cmd_args.push(args.agent.clone());

        for kit_path in &args.kit_paths {
            cmd_args.push("--kit".to_string());
            cmd_args.push(kit_path.to_string_lossy().to_string());
        }

        if let Some(name) = &args.name {
            cmd_args.push("--name".to_string());
            cmd_args.push(name.clone());
        }

        if let Some(template) = &args.template {
            cmd_args.push("-t".to_string());
            cmd_args.push(template.clone());
        }

        // Workspace as positional argument
        cmd_args.push(args.workspace.to_string_lossy().to_string());

        if !args.agent_args.is_empty() {
            cmd_args.push("--".to_string());
            cmd_args.extend(args.agent_args.clone());
        }

        cmd_args
    }
}

// ─── Helper Functions ─────────────────────────────────────────────────────

/// Locate a binary on the system PATH.
fn which_binary(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let full_path = dir.join(name);
            if full_path.is_file() {
                Some(full_path)
            } else {
                None
            }
        })
    })
}

/// Parse `sbx ports` text output into PortMapping structs.
/// Expected format: `<host_ip>:<host_port> -> <sandbox_port>/<protocol>`
fn parse_port_output(output: &str) -> Vec<PortMapping> {
    let mut mappings = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Try to parse: "0.0.0.0:8080 -> 8080/tcp"
        if let Some((left, right)) = line.split_once("->") {
            let left = left.trim();
            let right = right.trim();

            let (host_ip, host_port) = match left.rsplit_once(':') {
                Some((ip, port)) => (ip.to_string(), port.parse::<u16>().unwrap_or(0)),
                None => continue,
            };

            let (sandbox_port, protocol) = match right.split_once('/') {
                Some((port, proto)) => (
                    port.trim().parse::<u16>().unwrap_or(0),
                    proto.trim().to_string(),
                ),
                None => (right.parse::<u16>().unwrap_or(0), "tcp".to_string()),
            };

            if host_port > 0 && sandbox_port > 0 {
                mappings.push(PortMapping {
                    host_ip,
                    host_port,
                    sandbox_port,
                    protocol,
                });
            }
        }
    }
    mappings
}

/// Parse `sbx secret ls` text output into secret status entries.
///
/// Format: `SCOPE  TYPE  NAME  SECRET` (service name is column 2).
/// SECRET column may be "(oauth configured)", "configured", etc.
fn parse_secret_ls_text(output: &str) -> Vec<SbxSecretStatus> {
    let mut secrets = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.contains("---") {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();

        // Skip header line.
        if parts.first() == Some(&"SCOPE") {
            continue;
        }

        // SCOPE TYPE NAME SECRET — service name is column 2.
        // e.g.: (global)   service   anthropic   (oauth configured)
        if parts.len() >= 3 {
            let service = parts[2].to_string();
            let configured = parts[3..].iter().any(|p| p.contains("configured"));
            secrets.push(SbxSecretStatus {
                service,
                configured,
            });
        }
    }
    secrets
}

/// Parse `sbx template ls` text output into template info entries.
fn parse_template_ls_text(output: &str) -> Vec<TemplateInfo> {
    let mut templates = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty()
            || line.starts_with("TAG")
            || line.contains("---")
            || line.contains("No template")
            || line.contains("no template")
            || line.contains("not found")
            || line.contains("found.")
            || line.ends_with("found")
        {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if let Some(tag) = parts.first() {
            // Skip lines that look like prose messages rather than template entries
            // Template tags are typically short identifiers without spaces in the first token
            if !tag.is_empty() && !tag.contains(' ') {
                templates.push(TemplateInfo {
                    tag: tag.to_string(),
                    size: parts.get(1).map(|s| s.to_string()),
                    created: parts.get(2..).map(|s| s.join(" ")),
                });
            }
        }
    }
    templates
}

/// Extract the sandbox name from `sbx create` output.
///
/// `sbx create` output contains image pull progress and a line like:
///   "✓ Created sandbox 'kiro-bhtestworspace-1'"
///
/// This function handles both cases:
/// 1. If output is a single clean line, use it directly
/// 2. Otherwise, look for "Created sandbox 'NAME'" pattern
/// 3. Fall back to looking for "sbx run --name NAME" or legacy "sbx run NAME" pattern
pub fn extract_sandbox_name(output: &str) -> String {
    let trimmed = output.trim();

    // Case 1: single line with no spaces or special chars = likely the sandbox name
    if !trimmed.contains('\n') && !trimmed.contains(' ') && !trimmed.is_empty() {
        return trimmed.to_string();
    }

    // Case 2: look for "Created sandbox 'NAME'" pattern
    for line in output.lines() {
        if let Some(start) = line.find("Created sandbox '") {
            let after = &line[start + 17..]; // skip "Created sandbox '"
            if let Some(end) = after.find('\'') {
                return after[..end].to_string();
            }
        }
    }

    // Case 3: look for "sbx run --name NAME" or legacy "sbx run NAME" pattern at end of output
    for line in output.lines().rev() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("sbx run --name ") {
            return rest.trim().to_string();
        }
        if let Some(rest) = line.strip_prefix("sbx run ") {
            // Legacy format (deprecated in sbx 0.33.0) — skip if rest starts with a flag
            let rest = rest.trim();
            if !rest.starts_with('-') {
                return rest.to_string();
            }
        }
    }

    // Last resort: take the last non-empty line
    output
        .lines()
        .rev()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Version compatibility (Task 2.4 / Requirement 4.2) ──────────────────

    #[test]
    fn test_json_flag_unsupported_cobra_unknown_flag() {
        // Typical cobra-style error from an older sbx that lacks `--json`.
        let stderr = "Error: unknown flag: --json\nUsage:\n  sbx policy ls [flags]";
        assert!(SbxCli::is_json_flag_unsupported(stderr));
    }

    #[test]
    fn test_extract_sandbox_name_real_035_create_output() {
        // Real `sbx create` (no -q) output on sbx 0.35.0: image-pull progress
        // followed by the "✓ Created sandbox 'NAME'" line and a run hint.
        let output = "\
39cf20eca861: Already exists
Digest: sha256:39cf20eca861ec92747487af6197f6d916f774bdb98245d267dbd8dfd3debb05
Status: Image is up to date for docker/sandbox-templates:shell-docker
✓ Created sandbox 'shell-tmp.Zqe8jHNLOL'
  Workspace: /tmp/tmp.Zqe8jHNLOL (direct mount)
  Agent: shell

To connect to this sandbox, run:
  sbx run --name shell-tmp.Zqe8jHNLOL
";
        assert_eq!(extract_sandbox_name(output), "shell-tmp.Zqe8jHNLOL");
    }

    #[test]
    fn test_extract_sandbox_name_run_hint_only() {
        // If the "Created sandbox" line is absent, fall back to the run hint.
        let output = "some noise\n  sbx run --name my-sandbox-1\n";
        assert_eq!(extract_sandbox_name(output), "my-sandbox-1");
    }

    #[test]
    fn test_extract_sandbox_name_empty_output_is_empty() {
        // Regression guard: `sbx create -q` on 0.35.0 emits nothing. Empty stdout
        // must yield an empty name so create() surfaces a clear error. (We no
        // longer pass -q, but this documents the boundary.)
        assert_eq!(extract_sandbox_name(""), "");
        assert_eq!(extract_sandbox_name("   \n  \n"), "");
    }

    #[test]
    fn test_json_flag_unsupported_go_flag_phrasing() {
        let stderr = "flag provided but not defined: -json";
        // Missing the literal "--json" reference → not matched (avoids false positives).
        assert!(!SbxCli::is_json_flag_unsupported(stderr));

        let stderr2 = "flag provided but not defined: --json";
        assert!(SbxCli::is_json_flag_unsupported(stderr2));
    }

    #[test]
    fn test_json_flag_unsupported_various_phrasings() {
        assert!(SbxCli::is_json_flag_unsupported("unknown option '--json'"));
        assert!(SbxCli::is_json_flag_unsupported(
            "unrecognized flag --json\nUsage: sbx policy ls"
        ));
        assert!(SbxCli::is_json_flag_unsupported(
            "invalid flag --json provided"
        ));
        // Case-insensitive.
        assert!(SbxCli::is_json_flag_unsupported("Unknown Flag: --JSON"));
    }

    #[test]
    fn test_json_flag_unsupported_ignores_other_failures() {
        // Genuine runtime failures must NOT be misclassified as a version issue.
        assert!(!SbxCli::is_json_flag_unsupported(
            "Error: cannot connect to the sandbox daemon"
        ));
        assert!(!SbxCli::is_json_flag_unsupported(
            "permission denied while reading policy"
        ));
        // References --json but is not a flag/usage complaint (e.g. parse issue).
        assert!(!SbxCli::is_json_flag_unsupported(
            "failed to render --json report: internal error"
        ));
        assert!(!SbxCli::is_json_flag_unsupported(""));
    }

    #[test]
    fn test_parse_secret_ls_text_four_column_oauth() {
        // Four-column format (SCOPE TYPE NAME SECRET) with OAuth-configured secret
        let input = concat!(
            "SCOPE      TYPE      NAME        SECRET\n",
            "(global)   service   anthropic   (oauth configured)\n",
        );
        let secrets = parse_secret_ls_text(input);
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].service, "anthropic");
        assert!(secrets[0].configured, "oauth configured should be true");
    }

    #[test]
    fn test_parse_secret_ls_text_four_column_token() {
        // Four-column format with a plain token-configured secret
        let input = concat!(
            "SCOPE      TYPE      NAME     SECRET\n",
            "(global)   service   openai   configured\n",
        );
        let secrets = parse_secret_ls_text(input);
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0].service, "openai");
        assert!(secrets[0].configured);
    }

    #[test]
    fn test_parse_secret_ls_text_four_column_multiple() {
        // Four-column format with multiple secrets
        let input = concat!(
            "SCOPE      TYPE      NAME        SECRET\n",
            "(global)   service   anthropic   (oauth configured)\n",
            "(global)   service   openai      configured\n",
        );
        let secrets = parse_secret_ls_text(input);
        assert_eq!(secrets.len(), 2);
        let anthropic = secrets.iter().find(|s| s.service == "anthropic").unwrap();
        let openai = secrets.iter().find(|s| s.service == "openai").unwrap();
        assert!(anthropic.configured);
        assert!(openai.configured);
    }

    #[test]
    fn test_parse_secret_ls_text_four_column_header_not_treated_as_secret() {
        // SCOPE header must not produce a phantom "SCOPE" service entry
        let input = concat!(
            "SCOPE      TYPE      NAME        SECRET\n",
            "(global)   service   anthropic   (oauth configured)\n",
        );
        let secrets = parse_secret_ls_text(input);
        assert!(!secrets.iter().any(|s| s.service == "SCOPE"));
        assert!(!secrets.iter().any(|s| s.service == "(global)"));
    }

    // ─── Fix-checking: policy_ls / policy_remove_rule (Task 5.1) ─────────────
    //
    // These exercise the fixed `SbxCli::policy_ls()` and
    // `SbxCli::policy_remove_rule()` directly against a shell-script mock `sbx`
    // binary (the `create_test_manager` pattern from policy_manager.rs, applied
    // at the SbxCli level here per the design's "Fix Checking" / "Unit Tests").
    //
    // The mock scripts are written to a temp dir and made executable (0o755).

    /// Build an `SbxCli` whose `sbx` binary is a shell-script mock with the given
    /// contents. Returns the CLI plus the owning `TempDir` (kept alive by the
    /// caller for the duration of the test).
    #[cfg(unix)]
    fn mock_sbx(script_content: &str) -> (SbxCli, tempfile::TempDir) {
        use std::fs;
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("sbx");
        let mut file = fs::File::create(&script_path).unwrap();
        file.write_all(script_content.as_bytes()).unwrap();
        file.sync_all().unwrap();
        drop(file);
        // Mock scripts MUST be executable for the CLI to invoke them.
        fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        // Brief yield so the kernel releases the write lock before exec.
        std::thread::sleep(std::time::Duration::from_millis(1));

        (SbxCli::with_path(script_path), dir)
    }

    /// Representative real 0.35.0 payload: one global `applies_to:"all"` rule and
    /// one per-sandbox `applies_to:"sandbox:ktest"` rule (design.md, Requirement 3.3).
    #[cfg(unix)]
    const SBX_035_LS_JSON: &str = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    cat <<'JSON'
{
  "rules": [
    {
      "id": "b656a698-8713-442d-920c-bf95fbe979d4",
      "name": "b656a698-8713-442d-920c-bf95fbe979d4",
      "policy_id": "d8523707-740f-4d60-8385-a38e572d5639",
      "scope": "sandbox:ktest",
      "applies_to": "sandbox:ktest",
      "resource_type": "network",
      "decision": "allow",
      "resources": ["localhost:9100"],
      "origin": "scoped",
      "status": "active",
      "editable": true,
      "sandbox_id": "ktest"
    },
    {
      "id": "1e17bb98-582a-409a-aa6c-11b144c00938",
      "name": "1e17bb98-582a-409a-aa6c-11b144c00938",
      "policy_id": "local-policy",
      "scope": "global",
      "applies_to": "all",
      "resource_type": "network",
      "decision": "allow",
      "resources": ["**.kiro.dev:443"],
      "origin": "local",
      "status": "active",
      "editable": true
    }
  ]
}
JSON
    exit 0
fi
exit 1
"#;

    /// Requirement 3.3 / 1.1 / 1.2: the verified 0.35.0 JSON (one global + one
    /// per-sandbox rule) maps into `PolicyState` with correct field mapping —
    /// decision→action, resource→target, applies_to→origin,
    /// resource_type→rule_type, json origin→provenance, status→status.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_policy_ls_maps_global_and_sandbox_rules() {
        let (cli, _dir) = mock_sbx(SBX_035_LS_JSON);
        let state = cli.policy_ls().await.unwrap();

        assert_eq!(state.rules.len(), 2, "expected exactly two flattened rules");

        // Global rule.
        let global = state
            .rules
            .iter()
            .find(|r| r.target == "**.kiro.dev:443")
            .expect("global rule **.kiro.dev:443 missing");
        assert_eq!(
            global.id.as_deref(),
            Some("1e17bb98-582a-409a-aa6c-11b144c00938")
        );
        assert_eq!(global.action, "allow"); // decision → action
        assert_eq!(global.origin.as_deref(), Some("all")); // applies_to → origin
        assert_eq!(global.provenance.as_deref(), Some("local")); // json origin → provenance
        assert_eq!(global.rule_type.as_deref(), Some("network")); // resource_type → rule_type
        assert_eq!(global.status.as_deref(), Some("active"));

        // Per-sandbox rule.
        let sandbox = state
            .rules
            .iter()
            .find(|r| r.target == "localhost:9100")
            .expect("per-sandbox rule localhost:9100 missing");
        assert_eq!(
            sandbox.id.as_deref(),
            Some("b656a698-8713-442d-920c-bf95fbe979d4")
        );
        assert_eq!(sandbox.action, "allow");
        assert_eq!(sandbox.origin.as_deref(), Some("sandbox:ktest")); // distinguishes per-sandbox
        assert_eq!(sandbox.provenance.as_deref(), Some("scoped"));
        assert_eq!(sandbox.rule_type.as_deref(), Some("network"));
    }

    /// Requirement 1.4: an empty rule set yields an empty `PolicyState`, no error.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_policy_ls_empty_rules_no_error() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    echo '{"rules":[]}'
    exit 0
fi
exit 1
"#;
        let (cli, _dir) = mock_sbx(script);
        let state = cli.policy_ls().await.unwrap();
        assert!(
            state.rules.is_empty(),
            "expected no rules, got {:?}",
            state.rules
        );
    }

    /// Requirement 1.5: malformed / non-JSON stdout returns an `SbxError`
    /// describing the parse failure — NOT a silently-empty rule set.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_policy_ls_malformed_json_returns_parse_error() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    echo 'this is not json at all'
    exit 0
fi
exit 1
"#;
        let (cli, _dir) = mock_sbx(script);
        let result = cli.policy_ls().await;
        match result {
            Err(OrchestratorError::SbxError(msg)) => {
                assert!(
                    msg.contains("failed to parse sbx policy ls --json output"),
                    "error should describe the parse failure, got: {}",
                    msg
                );
            }
            other => panic!(
                "expected SbxError describing parse failure, got: {:?}",
                other
            ),
        }
    }

    /// A single JSON rule carrying multiple `resources` flattens into N
    /// `PolicyRule` rows that all share the rule id (design: "one row per resource").
    #[cfg(unix)]
    #[tokio::test]
    async fn test_policy_ls_multi_resource_rule_flattens_sharing_id() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    cat <<'JSON'
{"rules":[{"id":"multi-1","applies_to":"all","resource_type":"network","decision":"allow","resources":["a.com:443","b.com:443","c.com:443"],"origin":"local","status":"active"}]}
JSON
    exit 0
fi
exit 1
"#;
        let (cli, _dir) = mock_sbx(script);
        let state = cli.policy_ls().await.unwrap();

        assert_eq!(state.rules.len(), 3, "3 resources → 3 rows");
        assert!(state
            .rules
            .iter()
            .all(|r| r.id.as_deref() == Some("multi-1")));
        let targets: Vec<&str> = state.rules.iter().map(|r| r.target.as_str()).collect();
        assert_eq!(targets, vec!["a.com:443", "b.com:443", "c.com:443"]);
    }

    /// Requirement 2.3: removing a global rule issues
    /// `policy rm network --id <id>` with NO `--sandbox` scope. The mock only
    /// exits 0 for that exact argument shape, so a wrongly-scoped command fails.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_policy_remove_global_rule_no_sandbox_scope() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    cat <<'JSON'
{"rules":[{"id":"1e17bb98-582a-409a-aa6c-11b144c00938","applies_to":"all","resource_type":"network","decision":"allow","resources":["**.kiro.dev:443"],"origin":"local","status":"active"}]}
JSON
    exit 0
fi
if [ "$1" = "policy" ] && [ "$2" = "rm" ] && [ "$3" = "network" ] && [ "$4" = "--id" ] && [ "$5" = "1e17bb98-582a-409a-aa6c-11b144c00938" ] && [ -z "$6" ]; then
    exit 0
fi
exit 1
"#;
        let (cli, _dir) = mock_sbx(script);
        let result = cli
            .policy_remove_rule("1e17bb98-582a-409a-aa6c-11b144c00938")
            .await;
        assert!(
            result.is_ok(),
            "global removal should issue `policy rm network --id <id>` with no scope, got: {:?}",
            result
        );
    }

    /// Requirement 2.2: removing a per-sandbox rule issues
    /// `policy rm network --sandbox <name> --id <id>`. The mock only exits 0 for
    /// that exact scoped shape.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_policy_remove_sandbox_rule_adds_sandbox_scope() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    cat <<'JSON'
{"rules":[{"id":"b656a698-8713-442d-920c-bf95fbe979d4","applies_to":"sandbox:ktest","resource_type":"network","decision":"allow","resources":["localhost:9100"],"origin":"scoped","status":"active","sandbox_id":"ktest"}]}
JSON
    exit 0
fi
if [ "$1" = "policy" ] && [ "$2" = "rm" ] && [ "$3" = "network" ] && [ "$4" = "--sandbox" ] && [ "$5" = "ktest" ] && [ "$6" = "--id" ] && [ "$7" = "b656a698-8713-442d-920c-bf95fbe979d4" ]; then
    exit 0
fi
exit 1
"#;
        let (cli, _dir) = mock_sbx(script);
        let result = cli
            .policy_remove_rule("b656a698-8713-442d-920c-bf95fbe979d4")
            .await;
        assert!(
            result.is_ok(),
            "per-sandbox removal should add `--sandbox ktest`, got: {:?}",
            result
        );
    }

    /// Requirement 2.4: a rule id absent from the JSON listing yields `NotFound`.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_policy_remove_missing_rule_returns_notfound() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    echo '{"rules":[]}'
    exit 0
fi
exit 1
"#;
        let (cli, _dir) = mock_sbx(script);
        let result = cli.policy_remove_rule("does-not-exist").await;
        match result {
            Err(OrchestratorError::NotFound(msg)) => {
                assert!(
                    msg.contains("does-not-exist"),
                    "NotFound should identify the rule, got: {}",
                    msg
                );
            }
            other => panic!("expected NotFound, got: {:?}", other),
        }
    }

    /// Requirement 4.2: when the installed sbx predates `--json`, the daemon
    /// fails with an unknown-flag error; `policy_ls()` surfaces the clear
    /// minimum-version message rather than an opaque failure.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_policy_ls_version_incompat_unknown_flag() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "ls" ] && [ "$3" = "--json" ]; then
    echo "Error: unknown flag: --json" >&2
    echo "Usage:" >&2
    echo "  sbx policy ls [flags]" >&2
    exit 1
fi
exit 1
"#;
        let (cli, _dir) = mock_sbx(script);
        let result = cli.policy_ls().await;
        match result {
            Err(OrchestratorError::SbxError(msg)) => {
                assert!(
                    msg.contains("requires sbx 0.35.0 or later"),
                    "expected clear minimum-version error, got: {}",
                    msg
                );
            }
            other => panic!("expected minimum-version SbxError, got: {:?}", other),
        }
    }

    // ─── Preservation property tests (Task 5.2 / Property 2) ─────────────────
    //
    // Property 2 (Preservation): for arbitrary JSON rule sets emitted by
    // `sbx policy ls --json`, the flatten mapping preserves the per-rule
    // invariants (one row per resource; field mapping; multi-resource rows share
    // an id), the serialized `PolicyState`/`PolicyRule` field names remain the
    // frontend contract (`id`/`action`/`target`/`origin`/`provenance`/
    // `rule_type`/`status`, plus `default_policy`/`rules`), and removal-scope
    // derivation yields `--sandbox <name>` for per-sandbox rules and no scope for
    // global rules.
    //
    // **Validates: Requirements 3.1, 3.2**
    use proptest::prelude::*;

    /// `decision`: "allow" | "deny".
    fn arb_decision() -> impl Strategy<Value = String> {
        prop_oneof![Just("allow".to_string()), Just("deny".to_string())]
    }

    /// `resource_type`: a small set of plausible types.
    fn arb_resource_type() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("network".to_string()),
            Just("filesystem".to_string()),
            Just("process".to_string()),
        ]
    }

    /// `status`: "active" | "inactive".
    fn arb_status() -> impl Strategy<Value = String> {
        prop_oneof![Just("active".to_string()), Just("inactive".to_string())]
    }

    /// JSON `origin` (scope indicator, mapped to `provenance`): "local" | "scoped".
    fn arb_json_origin() -> impl Strategy<Value = String> {
        prop_oneof![Just("local".to_string()), Just("scoped".to_string())]
    }

    /// A bare sandbox name.
    fn arb_sandbox_name() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9-]{0,14}".prop_map(|s| s)
    }

    /// `applies_to`: global ("all") or per-sandbox ("sandbox:<name>").
    fn arb_applies_to() -> impl Strategy<Value = String> {
        prop_oneof![
            Just("all".to_string()),
            arb_sandbox_name().prop_map(|n| format!("sandbox:{}", n)),
        ]
    }

    /// A single resource target string.
    fn arb_resource() -> impl Strategy<Value = String> {
        prop_oneof![
            "[a-z]{2,8}\\.(com|dev|io):[0-9]{2,4}".prop_map(|s| s),
            Just("*".to_string()),
            "localhost:[0-9]{2,4}".prop_map(|s| s),
        ]
    }

    /// Generate an arbitrary `SbxPolicyLsRule` covering the input space: varying
    /// decision, resource_type, global/scoped applies_to, single/multi/empty
    /// resources, and status.
    fn arb_policy_ls_rule() -> impl Strategy<Value = SbxPolicyLsRule> {
        (
            "[a-f0-9-]{6,20}".prop_map(|s| s),
            arb_applies_to(),
            arb_resource_type(),
            arb_decision(),
            prop::collection::vec(arb_resource(), 0..5),
            arb_json_origin(),
            arb_status(),
        )
            .prop_map(
                |(id, applies_to, resource_type, decision, resources, json_origin, status)| {
                    SbxPolicyLsRule {
                        id: Some(id),
                        applies_to: Some(applies_to),
                        resource_type: Some(resource_type),
                        decision,
                        resources,
                        origin: Some(json_origin),
                        status: Some(status),
                    }
                },
            )
    }

    proptest! {
        /// Flatten invariants: the number of produced rows equals the total number
        /// of resources across all rules (empty-resource rules produce zero rows),
        /// and each row carries its source rule's fields verbatim (action←decision,
        /// target←each resource, origin←applies_to, provenance←json origin,
        /// rule_type←resource_type, status←status, id←rule id). Because every row
        /// from a given rule copies that rule's id, all rows from a multi-resource
        /// rule share the same id.
        ///
        /// **Validates: Requirements 3.1, 3.2**
        #[test]
        fn prop_flatten_preserves_invariants(
            rules in prop::collection::vec(arb_policy_ls_rule(), 0..6)
        ) {
            let flat = SbxCli::flatten_policy_rules(&rules);

            // Row count == sum of resources across all rules.
            let expected_rows: usize = rules.iter().map(|r| r.resources.len()).sum();
            prop_assert_eq!(flat.len(), expected_rows);

            // Rows are produced in rule/resource order; verify each maps verbatim.
            let mut idx = 0usize;
            for rule in &rules {
                for resource in &rule.resources {
                    let row = &flat[idx];
                    prop_assert_eq!(&row.id, &rule.id);
                    prop_assert_eq!(&row.action, &rule.decision);
                    prop_assert_eq!(&row.target, resource);
                    prop_assert_eq!(&row.origin, &rule.applies_to);
                    prop_assert_eq!(&row.provenance, &rule.origin);
                    prop_assert_eq!(&row.rule_type, &rule.resource_type);
                    prop_assert_eq!(&row.status, &rule.status);
                    idx += 1;
                }
            }
        }
    }

    proptest! {
        /// Serialization contract: the serialized `PolicyState` has exactly the
        /// keys `default_policy`/`rules`, and every serialized `PolicyRule` has
        /// exactly the frontend-contract keys `id`/`action`/`target`/`origin`/
        /// `provenance`/`rule_type`/`status`. This protects `PoliciesPage.tsx`
        /// from silent field renames.
        ///
        /// **Validates: Requirements 3.1, 3.2**
        #[test]
        fn prop_serialization_field_names_stable(
            rules in prop::collection::vec(arb_policy_ls_rule(), 0..6)
        ) {
            let flat = SbxCli::flatten_policy_rules(&rules);
            let state = PolicyState {
                default_policy: "balanced".to_string(),
                rules: flat,
            };

            let json = serde_json::to_value(&state).unwrap();
            let obj = json.as_object().expect("PolicyState must serialize to an object");

            let mut state_keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
            state_keys.sort();
            prop_assert_eq!(state_keys, vec!["default_policy", "rules"]);

            for row in obj["rules"].as_array().expect("rules must be an array") {
                let row_obj = row.as_object().expect("each rule must be an object");
                let mut rule_keys: Vec<&str> = row_obj.keys().map(|s| s.as_str()).collect();
                rule_keys.sort();
                prop_assert_eq!(
                    rule_keys,
                    vec!["action", "id", "origin", "provenance", "rule_type", "status", "target"]
                );
            }
        }
    }

    proptest! {
        /// Scoping property: a per-sandbox rule (`applies_to == "sandbox:<name>"`)
        /// always derives a removal scope of `Some("<name>")` (→ `--sandbox <name>`),
        /// while a global rule (`applies_to == "all"`, or any non-`sandbox:` origin)
        /// never derives a scope (→ no sandbox flag).
        ///
        /// **Validates: Requirements 3.1, 3.2**
        #[test]
        fn prop_removal_scope_matches_applies_to(rule in arb_policy_ls_rule()) {
            let origin = rule.applies_to.as_deref();
            let scope = SbxCli::derive_removal_scope(origin);
            match origin {
                Some(o) if o.starts_with("sandbox:") => {
                    let expected = o.strip_prefix("sandbox:").unwrap().to_string();
                    prop_assert_eq!(scope, Some(expected));
                }
                _ => {
                    prop_assert_eq!(scope, None);
                }
            }
        }
    }

    // ─── Preservation: untouched policy commands (Task 5.3 / Property 2) ─────
    //
    // Requirement 3.1 / Design "Preservation Requirements": the policy commands
    // that this bugfix does NOT touch must keep issuing byte-identical argv.
    // Each mock below `exit 0` ONLY for the exact expected argument shape and
    // `exit 1` for anything else, so any drift in command construction (extra
    // flag, reordered positional, wrong subcommand) makes the assertion fail.

    /// `policy_allow_network(target)` → `policy allow network <target>`
    /// (global scope, no `--sandbox` flag).
    #[cfg(unix)]
    #[tokio::test]
    async fn test_preserve_policy_allow_network_command() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "allow" ] && [ "$3" = "network" ] && [ "$4" = "example.com:443" ] && [ -z "$5" ]; then
    exit 0
fi
exit 1
"#;
        let (cli, _dir) = mock_sbx(script);
        let result = cli.policy_allow_network("example.com:443").await;
        assert!(
            result.is_ok(),
            "expected `policy allow network example.com:443` (no scope), got: {:?}",
            result
        );
    }

    /// `policy_allow_network_for_sandbox(name, target)` →
    /// `policy allow network --sandbox <name> <target>` (flag before positional).
    #[cfg(unix)]
    #[tokio::test]
    async fn test_preserve_policy_allow_network_for_sandbox_command() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "allow" ] && [ "$3" = "network" ] && [ "$4" = "--sandbox" ] && [ "$5" = "ktest" ] && [ "$6" = "localhost:9100" ] && [ -z "$7" ]; then
    exit 0
fi
exit 1
"#;
        let (cli, _dir) = mock_sbx(script);
        let result = cli
            .policy_allow_network_for_sandbox("ktest", "localhost:9100")
            .await;
        assert!(
            result.is_ok(),
            "expected `policy allow network --sandbox ktest localhost:9100`, got: {:?}",
            result
        );
    }

    /// `policy_deny_network(target)` → `policy deny network <target>`
    /// (global scope, no `--sandbox` flag).
    #[cfg(unix)]
    #[tokio::test]
    async fn test_preserve_policy_deny_network_command() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "deny" ] && [ "$3" = "network" ] && [ "$4" = "blocked.com:80" ] && [ -z "$5" ]; then
    exit 0
fi
exit 1
"#;
        let (cli, _dir) = mock_sbx(script);
        let result = cli.policy_deny_network("blocked.com:80").await;
        assert!(
            result.is_ok(),
            "expected `policy deny network blocked.com:80` (no scope), got: {:?}",
            result
        );
    }

    /// `policy_set_default(mode)` → `policy init <mode>` (the command was renamed
    /// from `set-default` to `init` in sbx 0.34.0) where the mode string comes
    /// from `PolicyDefault`'s `Display` ("balanced" / "allow-all" / "deny-all").
    /// All three variants are checked to guard against drift in the Display
    /// mapping used to build the command.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_preserve_policy_set_default_command() {
        for (mode, expected) in [
            (PolicyDefault::Balanced, "balanced"),
            (PolicyDefault::Allow, "allow-all"),
            (PolicyDefault::Deny, "deny-all"),
        ] {
            let script = format!(
                r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "init" ] && [ "$3" = "{expected}" ] && [ -z "$4" ]; then
    exit 0
fi
exit 1
"#
            );
            let (cli, _dir) = mock_sbx(&script);
            let result = cli.policy_set_default(mode.clone()).await;
            assert!(
                result.is_ok(),
                "expected `policy init {}` for {:?}, got: {:?}",
                expected,
                mode,
                result
            );
        }
    }

    /// `policy_log(Some(sandbox), Some(limit))` →
    /// `policy log <SANDBOX> --limit <n>` — the sandbox is a POSITIONAL argument
    /// that must precede the `--limit` flag. The mock only exits 0 for that exact
    /// ordering, so a reordered/`--limit`-first construction would fail.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_preserve_policy_log_command_positional_before_limit() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "log" ] && [ "$3" = "ktest" ] && [ "$4" = "--limit" ] && [ "$5" = "50" ] && [ -z "$6" ]; then
    echo '[]'
    exit 0
fi
exit 1
"#;
        let (cli, _dir) = mock_sbx(script);
        let result = cli.policy_log(Some("ktest"), Some(50)).await;
        assert!(
            result.is_ok(),
            "expected `policy log ktest --limit 50` (positional sandbox before flag), got: {:?}",
            result
        );
    }

    /// `policy_reset()` → `policy reset --force`.
    #[cfg(unix)]
    #[tokio::test]
    async fn test_preserve_policy_reset_command() {
        let script = r#"#!/bin/sh
if [ "$1" = "policy" ] && [ "$2" = "reset" ] && [ "$3" = "--force" ] && [ -z "$4" ]; then
    exit 0
fi
exit 1
"#;
        let (cli, _dir) = mock_sbx(script);
        let result = cli.policy_reset().await;
        assert!(
            result.is_ok(),
            "expected `policy reset --force`, got: {:?}",
            result
        );
    }

    // ─── Optional real-binary verification (Task 6.3 / Requirement 4.1) ──────

    /// Verifies the `sbx policy ls --json` JSON-shape assumption against the REAL
    /// installed `sbx` binary rather than a mock, so we catch drift if a future
    /// sbx release changes the payload the parser in [`SbxCli::policy_ls`] relies
    /// on (decision/resources/applies_to/resource_type/status).
    ///
    /// This test is `#[ignore]`d so it never runs in the normal suite or CI (CI
    /// machines have no `sbx` install / login). Run it manually on a workstation
    /// that has a real **sbx 0.35.0+** installed and is logged in:
    ///
    /// ```text
    /// cargo test -- --ignored test_real_binary_policy_ls_json_shape
    /// ```
    ///
    /// It resolves `sbx` from `PATH` via [`SbxCli::new`], calls `policy_ls()`, and
    /// asserts the call succeeds and that any returned rules satisfy the structural
    /// invariants the parser depends on. Because real policy contents are
    /// environment-dependent, it asserts STRUCTURE (well-formed fields), never the
    /// presence of specific rules. A hard failure here is acceptable — it only runs
    /// when explicitly requested via `--ignored`.
    ///
    /// _Requirements: 4.1_
    #[ignore = "requires a real sbx 0.35.0+ install and login; run with `cargo test -- --ignored`"]
    #[tokio::test]
    async fn test_real_binary_policy_ls_json_shape() {
        // Resolve the real `sbx` binary from PATH.
        let cli = SbxCli::new().expect("sbx binary must be resolvable from PATH for this test");

        // The call itself must succeed against the real 0.35.0+ `--json` output.
        let state = cli
            .policy_ls()
            .await
            .expect("policy_ls() should succeed against a real sbx 0.35.0+ install");

        // `default_policy` is derived best-effort; it must be one of the known labels.
        assert!(
            matches!(
                state.default_policy.as_str(),
                "allow-all" | "balanced" | "deny-all"
            ),
            "default_policy should be a known derived label, got: {:?}",
            state.default_policy
        );

        // Assert STRUCTURAL invariants on each returned rule (not specific rules,
        // since real policy contents are environment-dependent).
        for rule in &state.rules {
            assert!(
                rule.action == "allow" || rule.action == "deny",
                "rule action must be \"allow\" or \"deny\", got: {:?}",
                rule.action
            );
            assert!(
                !rule.target.is_empty(),
                "rule target (resource) must be non-empty"
            );
            assert!(
                rule.rule_type.is_some(),
                "rule_type (resource_type) should be present, rule: {:?}",
                rule
            );
            // `origin` distinguishes global ("all") from per-sandbox ("sandbox:<name>").
            if let Some(origin) = rule.origin.as_deref() {
                assert!(
                    origin == "all" || origin.starts_with("sandbox:"),
                    "origin should be \"all\" or \"sandbox:<name>\", got: {:?}",
                    origin
                );
            }
        }
    }
}
