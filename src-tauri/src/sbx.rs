use std::path::{Path, PathBuf};
use std::process::Stdio;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

use crate::error::OrchestratorError;

/// Commands that involve secrets — stderr from these is redacted before logging.
const SECRET_COMMANDS: &[&str] = &["secret"];

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
            Self::Allow => write!(f, "allow"),
            Self::Deny => write!(f, "deny"),
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

/// Kit inspect result from `sbx kit inspect`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KitInspectResult {
    pub raw_output: String,
    pub json: Option<serde_json::Value>,
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

        let output = cmd.output().await.map_err(|e| {
            OrchestratorError::SbxError(format!(
                "Failed to execute sbx {}: {}",
                subcommand, e
            ))
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
        let mut cmd = Command::new(&self.sbx_path);
        cmd.arg(subcommand);
        for arg in args {
            cmd.arg(arg);
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = cmd.output().await.map_err(|e| {
            OrchestratorError::SbxError(format!(
                "Failed to execute sbx {}: {}",
                subcommand, e
            ))
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
        let output = cmd.output().await.map_err(|e| {
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
    /// - `-v` workspace mount
    /// - `--` separator followed by agent CLI args
    pub async fn run(&self, args: &SbxRunArgs) -> Result<String, OrchestratorError> {
        let mut cmd_args: Vec<String> = Vec::new();

        // Agent identifier
        cmd_args.push(args.agent.clone());

        // Kit paths
        for kit_path in &args.kit_paths {
            cmd_args.push("--kit".to_string());
            cmd_args.push(kit_path.to_string_lossy().to_string());
        }

        // Workspace mount
        cmd_args.push("-v".to_string());
        cmd_args.push(args.workspace.to_string_lossy().to_string());

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

    /// Create a sandbox without starting it: `sbx create <agent> --kit <path> -v <workspace>`
    pub async fn create(&self, args: &SbxCreateArgs) -> Result<String, OrchestratorError> {
        let mut cmd_args: Vec<String> = Vec::new();

        cmd_args.push(args.agent.clone());

        for kit_path in &args.kit_paths {
            cmd_args.push("--kit".to_string());
            cmd_args.push(kit_path.to_string_lossy().to_string());
        }

        cmd_args.push("-v".to_string());
        cmd_args.push(args.workspace.to_string_lossy().to_string());

        if let Some(name) = &args.name {
            cmd_args.push("--name".to_string());
            cmd_args.push(name.clone());
        }

        if let Some(template) = &args.template {
            cmd_args.push("-t".to_string());
            cmd_args.push(template.clone());
        }

        let output = self.exec_command_owned("create", &cmd_args).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx create failed: {}",
                output.stderr.trim()
            )));
        }

        Ok(output.stdout.trim().to_string())
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
        let output = self.exec_command("rm", &[sandbox_id]).await?;
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

        let sandboxes: Vec<SandboxInfo> = serde_json::from_str(&output.stdout)
            .map_err(|e| {
                OrchestratorError::SbxError(format!(
                    "Failed to parse sbx ls JSON output: {}",
                    e
                ))
            })?;

        Ok(sandboxes)
    }

    /// Execute an interactive command in a sandbox: `sbx exec -it <sandbox_id>`
    /// Returns the child process for PTY attachment.
    pub async fn exec_it(
        &self,
        sandbox_id: &str,
    ) -> Result<tokio::process::Child, OrchestratorError> {
        let child = Command::new(&self.sbx_path)
            .arg("exec")
            .arg("-it")
            .arg(sandbox_id)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| {
                OrchestratorError::SbxError(format!(
                    "Failed to spawn sbx exec -it: {}",
                    e
                ))
            })?;

        Ok(child)
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

    /// Add a kit to a running sandbox: `sbx kit add <sandbox_id> <kit_path>`
    pub async fn kit_add(
        &self,
        sandbox_id: &str,
        kit_path: &Path,
    ) -> Result<(), OrchestratorError> {
        let path_str = kit_path.to_string_lossy();
        let output = self
            .exec_multi_command(&["kit", "add"], &[sandbox_id, &path_str])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx kit add failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

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

    /// Inspect a kit: `sbx kit inspect <kit_path>`
    pub async fn kit_inspect(
        &self,
        kit_path: &Path,
    ) -> Result<KitInspectResult, OrchestratorError> {
        let path_str = kit_path.to_string_lossy();
        let output = self
            .exec_multi_command(&["kit", "inspect"], &[&path_str])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx kit inspect failed: {}",
                output.stderr.trim()
            )));
        }

        let json = serde_json::from_str(&output.stdout).ok();
        Ok(KitInspectResult {
            raw_output: output.stdout.clone(),
            json,
        })
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

    /// Publish a port for a sandbox: `sbx ports --publish <port_spec> <sandbox_id>`
    pub async fn ports_publish(
        &self,
        sandbox_id: &str,
        port_spec: &str,
    ) -> Result<PortMapping, OrchestratorError> {
        let output = self
            .exec_command("ports", &["--publish", port_spec, sandbox_id])
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
            OrchestratorError::SbxError(
                "sbx ports --publish returned no port mapping".to_string(),
            )
        })
    }

    /// Unpublish a port for a sandbox: `sbx ports --unpublish <port_spec> <sandbox_id>`
    pub async fn ports_unpublish(
        &self,
        sandbox_id: &str,
        port_spec: &str,
    ) -> Result<(), OrchestratorError> {
        let output = self
            .exec_command("ports", &["--unpublish", port_spec, sandbox_id])
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

    /// List current policy state: `sbx policy ls`
    pub async fn policy_ls(&self) -> Result<PolicyState, OrchestratorError> {
        let output = self.exec_multi_command(&["policy", "ls"], &[]).await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx policy ls failed: {}",
                output.stderr.trim()
            )));
        }

        // Attempt JSON parse first, fall back to text parsing
        if let Ok(state) = serde_json::from_str::<PolicyState>(&output.stdout) {
            return Ok(state);
        }

        // Fallback: parse text output
        Ok(parse_policy_text(&output.stdout))
    }

    /// Set the default policy: `sbx policy set-default <mode>`
    pub async fn policy_set_default(
        &self,
        mode: PolicyDefault,
    ) -> Result<(), OrchestratorError> {
        let mode_str = mode.to_string();
        let output = self
            .exec_multi_command(&["policy", "set-default"], &[&mode_str])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx policy set-default failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    /// Allow network access: `sbx policy allow network "<target>"`
    pub async fn policy_allow_network(
        &self,
        target: &str,
    ) -> Result<(), OrchestratorError> {
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

    /// Deny network access: `sbx policy deny network "<target>"`
    pub async fn policy_deny_network(
        &self,
        target: &str,
    ) -> Result<(), OrchestratorError> {
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

    /// Remove a policy rule: `sbx policy remove <rule_id>`
    pub async fn policy_remove_rule(
        &self,
        rule_id: &str,
    ) -> Result<(), OrchestratorError> {
        let output = self
            .exec_multi_command(&["policy", "remove"], &[rule_id])
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(format!(
                "sbx policy remove failed: {}",
                output.stderr.trim()
            )));
        }
        Ok(())
    }

    /// Get policy traffic log: `sbx policy log [--sandbox <id>] [--limit <n>]`
    pub async fn policy_log(
        &self,
        sandbox_id: Option<&str>,
        limit: Option<u32>,
    ) -> Result<Vec<PolicyLogEntry>, OrchestratorError> {
        let mut args: Vec<String> = Vec::new();
        if let Some(id) = sandbox_id {
            args.push("--sandbox".to_string());
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
        let entries: Vec<PolicyLogEntry> =
            serde_json::from_str(&output.stdout).unwrap_or_default();
        Ok(entries)
    }

    /// Reset all policy rules: `sbx policy reset`
    pub async fn policy_reset(&self) -> Result<(), OrchestratorError> {
        let output = self
            .exec_multi_command(&["policy", "reset"], &[])
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
    pub async fn secret_set(
        &self,
        service: &str,
        value: &str,
    ) -> Result<(), OrchestratorError> {
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

    /// Set a scoped secret for a specific sandbox: `sbx secret set <sandbox_id> -g <service> -t <value>`
    pub async fn secret_set_scoped(
        &self,
        sandbox_id: &str,
        service: &str,
        value: &str,
    ) -> Result<(), OrchestratorError> {
        let output = self
            .exec_multi_command(
                &["secret", "set"],
                &[sandbox_id, "-g", service, "-t", value],
            )
            .await?;
        if !output.success {
            return Err(OrchestratorError::SbxError(
                "sbx secret set (scoped) failed".to_string(),
            ));
        }
        Ok(())
    }

    /// Initiate OAuth flow for a service: `sbx secret set -g <service> --oauth`
    pub async fn secret_set_oauth(
        &self,
        service: &str,
    ) -> Result<(), OrchestratorError> {
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
        let output = self
            .exec_multi_command(&["template", "ls"], &[])
            .await?;
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
    pub async fn template_load(
        &self,
        tar_path: &Path,
    ) -> Result<(), OrchestratorError> {
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
        let output = self
            .exec_multi_command(&["template", "rm"], &[tag])
            .await?;
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
        let output = self.exec_command("logout", &[]).await?;
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

        cmd_args.push("-v".to_string());
        cmd_args.push(args.workspace.to_string_lossy().to_string());

        if let Some(name) = &args.name {
            cmd_args.push("--name".to_string());
            cmd_args.push(name.clone());
        }

        if let Some(template) = &args.template {
            cmd_args.push("-t".to_string());
            cmd_args.push(template.clone());
        }

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
                Some((port, proto)) => {
                    (port.trim().parse::<u16>().unwrap_or(0), proto.trim().to_string())
                }
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

/// Parse `sbx policy ls` text output into a PolicyState.
fn parse_policy_text(output: &str) -> PolicyState {
    let mut default_policy = "balanced".to_string();
    let mut rules = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // Look for default policy indicator
        if line.to_lowercase().contains("default") {
            if line.to_lowercase().contains("allow") {
                default_policy = "allow".to_string();
            } else if line.to_lowercase().contains("deny") {
                default_policy = "deny".to_string();
            } else if line.to_lowercase().contains("balanced") {
                default_policy = "balanced".to_string();
            }
        }
        // Look for rule lines (heuristic: lines with "allow" or "deny" and a target)
        if (line.starts_with("allow") || line.starts_with("deny")) && line.contains(' ') {
            let parts: Vec<&str> = line.splitn(3, ' ').collect();
            if parts.len() >= 2 {
                rules.push(PolicyRule {
                    id: None,
                    action: parts[0].to_string(),
                    target: parts[1..].join(" "),
                });
            }
        }
    }

    PolicyState {
        default_policy,
        rules,
    }
}

/// Parse `sbx secret ls` text output into secret status entries.
fn parse_secret_ls_text(output: &str) -> Vec<SbxSecretStatus> {
    let mut secrets = Vec::new();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("SERVICE") || line.contains("---") {
            continue;
        }
        // Expected format: "<service>  <status>" or "<service> configured/not configured"
        let parts: Vec<&str> = line.split_whitespace().collect();
        if let Some(service) = parts.first() {
            let configured = parts
                .iter()
                .any(|p| *p == "configured" || *p == "yes" || *p == "✓");
            secrets.push(SbxSecretStatus {
                service: service.to_string(),
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
        if line.is_empty() || line.starts_with("TAG") || line.contains("---") {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if let Some(tag) = parts.first() {
            templates.push(TemplateInfo {
                tag: tag.to_string(),
                size: parts.get(1).map(|s| s.to_string()),
                created: parts.get(2..).map(|s| s.join(" ")),
            });
        }
    }
    templates
}
