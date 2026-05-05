//! System Manager: wraps `sbx` CLI commands for system diagnostics and auth management.
//!
//! Provides a thin abstraction over SbxCli diagnostic/auth methods and adds
//! Docker availability checking via `docker --version`.

use std::sync::Arc;

use crate::error::OrchestratorError;
use crate::sbx::{DiagnoseResult, SbxCli, SbxVersion};
use crate::types::DependencyStatus;

/// Manages system diagnostics and Docker auth via the `sbx` CLI.
///
/// This struct delegates to `SbxCli` methods for version, diagnose, login,
/// and logout operations. It also checks Docker availability directly.
pub struct SystemManager {
    sbx: Arc<SbxCli>,
}

impl SystemManager {
    /// Create a new SystemManager wrapping the given SbxCli instance.
    pub fn new(sbx: Arc<SbxCli>) -> Self {
        Self { sbx }
    }

    /// Check whether the user is authenticated with Docker/sbx.
    ///
    /// Attempts `sbx version` as a lightweight probe. If it fails with
    /// auth-related errors (e.g., "not logged in", "unauthorized", "auth"),
    /// returns `false`. Returns `true` if the command succeeds.
    pub async fn check_auth_status(&self) -> Result<bool, OrchestratorError> {
        match self.sbx.version().await {
            Ok(_) => Ok(true),
            Err(OrchestratorError::SbxError(msg)) => {
                let lower = msg.to_lowercase();
                if lower.contains("not logged in")
                    || lower.contains("unauthorized")
                    || lower.contains("auth")
                    || lower.contains("login required")
                {
                    Ok(false)
                } else {
                    // Non-auth error — propagate
                    Err(OrchestratorError::SbxError(msg))
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Initiate Docker login (opens browser for OAuth).
    ///
    /// Delegates to `sbx login`.
    pub async fn login(&self) -> Result<(), OrchestratorError> {
        self.sbx.login().await
    }

    /// Sign out of Docker.
    ///
    /// Delegates to `sbx logout`.
    pub async fn logout(&self) -> Result<(), OrchestratorError> {
        self.sbx.logout().await
    }

    /// Get the sbx CLI version string.
    ///
    /// Delegates to `sbx version`.
    pub async fn get_version(&self) -> Result<SbxVersion, OrchestratorError> {
        self.sbx.version().await
    }

    /// Run system diagnostics.
    ///
    /// Delegates to `sbx diagnose`.
    pub async fn diagnose(&self) -> Result<DiagnoseResult, OrchestratorError> {
        self.sbx.diagnose().await
    }

    /// Check availability of required dependencies (sbx CLI and Docker).
    ///
    /// Runs `sbx version` and `docker --version`, returning a summary
    /// of what is available and their version strings.
    pub async fn dependency_check(&self) -> Result<DependencyStatus, OrchestratorError> {
        // Check sbx availability
        let (sbx_available, sbx_version) = match self.sbx.version().await {
            Ok(v) => (true, Some(v.version)),
            Err(_) => (false, None),
        };

        // Check Docker availability via `docker --version`
        let (docker_available, docker_version) = match Self::check_docker_version().await {
            Ok(version) => (true, Some(version)),
            Err(_) => (false, None),
        };

        Ok(DependencyStatus {
            sbx_available,
            sbx_version,
            docker_available,
            docker_version,
        })
    }

    /// Run `docker --version` and extract the version string.
    async fn check_docker_version() -> Result<String, OrchestratorError> {
        use tokio::process::Command;

        let binary = if cfg!(target_os = "windows") {
            "docker.exe"
        } else {
            "docker"
        };

        let output = Command::new(binary)
            .arg("--version")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| {
                OrchestratorError::DockerError(format!(
                    "Failed to execute docker --version: {}",
                    e
                ))
            })?;

        if !output.status.success() {
            return Err(OrchestratorError::DockerError(
                "docker --version returned non-zero exit code".to_string(),
            ));
        }

        let version_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(version_str)
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a SystemManager with a mock sbx binary (a shell script).
    fn create_test_manager(script_content: &str) -> (SystemManager, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("sbx");

        #[cfg(unix)]
        {
            use std::fs;
            use std::os::unix::fs::PermissionsExt;
            fs::write(&script_path, script_content).unwrap();
            fs::set_permissions(&script_path, fs::Permissions::from_mode(0o755)).unwrap();
        }

        #[cfg(windows)]
        {
            let script_path = dir.path().join("sbx.bat");
            std::fs::write(&script_path, script_content).unwrap();
        }

        let sbx = Arc::new(SbxCli::with_path(script_path));
        let manager = SystemManager::new(sbx);
        (manager, dir)
    }

    #[tokio::test]
    async fn test_check_auth_status_authenticated() {
        let script = r#"#!/bin/sh
if [ "$1" = "version" ]; then
    echo "sbx version 0.5.0"
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.check_auth_status().await.unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn test_check_auth_status_not_logged_in() {
        let script = r#"#!/bin/sh
if [ "$1" = "version" ]; then
    echo "error: not logged in" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.check_auth_status().await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_check_auth_status_unauthorized() {
        let script = r#"#!/bin/sh
if [ "$1" = "version" ]; then
    echo "error: unauthorized access" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.check_auth_status().await.unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn test_check_auth_status_non_auth_error() {
        let script = r#"#!/bin/sh
if [ "$1" = "version" ]; then
    echo "error: network timeout" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.check_auth_status().await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_login_success() {
        let script = r#"#!/bin/sh
if [ "$1" = "login" ]; then
    echo "Login successful"
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.login().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_login_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "login" ]; then
    echo "error: browser failed to open" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.login().await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_logout_success() {
        let script = r#"#!/bin/sh
if [ "$1" = "logout" ]; then
    echo "Logged out"
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.logout().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_logout_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "logout" ]; then
    echo "error: logout failed" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.logout().await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_get_version_success() {
        let script = r#"#!/bin/sh
if [ "$1" = "version" ]; then
    echo "sbx version 0.5.0"
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let version = mgr.get_version().await.unwrap();
        assert_eq!(version.version, "sbx version 0.5.0");
    }

    #[tokio::test]
    async fn test_get_version_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "version" ]; then
    echo "error: sbx not configured" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.get_version().await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_diagnose_success() {
        let script = r#"#!/bin/sh
if [ "$1" = "diagnose" ]; then
    echo '{"status":"ok","docker":"running"}'
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.diagnose().await.unwrap();
        assert!(result.json.is_some());
        assert!(result.raw_output.contains("status"));
    }

    #[tokio::test]
    async fn test_diagnose_non_json_output() {
        let script = r#"#!/bin/sh
if [ "$1" = "diagnose" ]; then
    echo "Docker: running"
    echo "sbx: ok"
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.diagnose().await.unwrap();
        assert!(result.json.is_none());
        assert!(result.raw_output.contains("Docker: running"));
    }

    #[tokio::test]
    async fn test_diagnose_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "diagnose" ]; then
    echo "error: diagnose failed" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.diagnose().await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_dependency_check_sbx_available() {
        let script = r#"#!/bin/sh
if [ "$1" = "version" ]; then
    echo "sbx version 0.5.0"
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let status = mgr.dependency_check().await.unwrap();
        assert!(status.sbx_available);
        assert_eq!(status.sbx_version, Some("sbx version 0.5.0".to_string()));
        // Docker availability depends on the test environment
    }

    #[tokio::test]
    async fn test_dependency_check_sbx_unavailable() {
        let script = r#"#!/bin/sh
if [ "$1" = "version" ]; then
    echo "error: something went wrong" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let status = mgr.dependency_check().await.unwrap();
        assert!(!status.sbx_available);
        assert_eq!(status.sbx_version, None);
    }
}
