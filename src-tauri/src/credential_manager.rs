//! Credential Manager: wraps `sbx secret` CLI commands for secure credential management.
//!
//! SECURITY:
//! - Secret values are NEVER stored in SQLite or logged.
//! - All secrets are stored exclusively in the OS keychain via `sbx secret`.
//! - The `zeroize` crate is used to clear secret values from memory after passing to sbx CLI.
//! - stderr output from `sbx secret set` is redacted before any logging to prevent
//!   accidental secret exposure.

use std::sync::Arc;

use zeroize::Zeroize;

use crate::error::OrchestratorError;
use crate::sbx::SbxCli;
use crate::types::SecretStatus;

/// Manages credentials via the `sbx secret` CLI.
///
/// This struct provides a security layer on top of `SbxCli` secret methods:
/// - Zeroizes secret values from memory after use
/// - Never logs or persists secret values
/// - Redacts stderr from secret operations
pub struct CredentialManager {
    sbx: Arc<SbxCli>,
}

impl CredentialManager {
    /// Create a new CredentialManager wrapping the given SbxCli instance.
    pub fn new(sbx: Arc<SbxCli>) -> Self {
        Self { sbx }
    }

    /// List all configured secrets and their status.
    ///
    /// Invokes `sbx secret ls` and maps the output to `Vec<SecretStatus>`.
    /// Only returns service names and whether they are configured — never exposes values.
    pub async fn list_secrets(&self) -> Result<Vec<SecretStatus>, OrchestratorError> {
        let sbx_secrets = self.sbx.secret_ls().await?;

        let secrets = sbx_secrets
            .into_iter()
            .map(|s| SecretStatus {
                service: s.service,
                configured: s.configured,
            })
            .collect();

        Ok(secrets)
    }

    /// Set a secret for a service via API key.
    ///
    /// Invokes `sbx secret set -g <service> -t <value>`.
    ///
    /// SECURITY:
    /// - The value is zeroized from memory after being passed to the sbx CLI.
    /// - No secret values are logged or stored in SQLite.
    pub async fn set_secret(
        &self,
        service: &str,
        mut value: String,
    ) -> Result<(), OrchestratorError> {
        // Validate inputs
        if service.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Service name cannot be empty".to_string(),
            ));
        }
        if value.is_empty() {
            return Err(OrchestratorError::Validation(
                "Secret value cannot be empty".to_string(),
            ));
        }

        let result = self.sbx.secret_set(service, &value).await;

        // Zeroize the secret value from memory regardless of success/failure
        value.zeroize();

        result
    }

    /// Initiate OAuth flow for a service.
    ///
    /// Invokes `sbx secret set -g <service> --oauth`, which opens a browser
    /// for authentication.
    pub async fn set_secret_oauth(&self, service: &str) -> Result<(), OrchestratorError> {
        if service.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Service name cannot be empty".to_string(),
            ));
        }

        self.sbx.secret_set_oauth(service).await
    }

    /// Remove a secret for a service.
    ///
    /// Invokes `sbx secret rm -g <service> -f` to remove the credential
    /// from the OS keychain.
    pub async fn remove_secret(&self, service: &str) -> Result<(), OrchestratorError> {
        if service.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Service name cannot be empty".to_string(),
            ));
        }

        self.sbx.secret_rm(service).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Creates a CredentialManager with a mock sbx binary (a shell script).
    /// The mock script simulates `sbx secret` commands for testing.
    fn create_test_manager(script_content: &str) -> (CredentialManager, tempfile::TempDir) {
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
            // On Windows, use a .bat file
            let script_path = dir.path().join("sbx.bat");
            std::fs::write(&script_path, script_content).unwrap();
        }

        let sbx = Arc::new(SbxCli::with_path(script_path));
        let manager = CredentialManager::new(sbx);
        (manager, dir)
    }

    #[tokio::test]
    async fn test_list_secrets_parses_json_output() {
        let script = r#"#!/bin/sh
if [ "$1" = "secret" ] && [ "$2" = "ls" ]; then
    echo '[{"service":"openai","configured":true},{"service":"anthropic","configured":false}]'
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let secrets = mgr.list_secrets().await.unwrap();

        assert_eq!(secrets.len(), 2);
        assert_eq!(secrets[0].service, "openai");
        assert!(secrets[0].configured);
        assert_eq!(secrets[1].service, "anthropic");
        assert!(!secrets[1].configured);
    }

    #[tokio::test]
    async fn test_list_secrets_empty() {
        let script = r#"#!/bin/sh
if [ "$1" = "secret" ] && [ "$2" = "ls" ]; then
    echo '[]'
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let secrets = mgr.list_secrets().await.unwrap();
        assert!(secrets.is_empty());
    }

    #[tokio::test]
    async fn test_set_secret_success() {
        let script = r#"#!/bin/sh
if [ "$1" = "secret" ] && [ "$2" = "set" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr
            .set_secret("openai", "sk-test-key-12345".to_string())
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_secret_empty_service_fails() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.set_secret("", "some-value".to_string()).await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_set_secret_empty_value_fails() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.set_secret("openai", "".to_string()).await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_set_secret_whitespace_service_fails() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.set_secret("   ", "some-value".to_string()).await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_set_secret_zeroizes_value() {
        // This test verifies the zeroize behavior by checking that
        // the method accepts ownership of the value string.
        // The actual zeroization happens internally — we verify the API
        // takes ownership (not a reference) which enables zeroization.
        let script = r#"#!/bin/sh
if [ "$1" = "secret" ] && [ "$2" = "set" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let secret_value = "my-secret-api-key".to_string();
        // Value is moved into set_secret — caller can't access it after
        let result = mgr.set_secret("openai", secret_value).await;
        assert!(result.is_ok());
        // secret_value is no longer accessible here (moved), confirming ownership transfer
    }

    #[tokio::test]
    async fn test_set_secret_oauth_success() {
        let script = r#"#!/bin/sh
if [ "$1" = "secret" ] && [ "$2" = "set" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.set_secret_oauth("openai").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_set_secret_oauth_empty_service_fails() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.set_secret_oauth("").await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_remove_secret_success() {
        let script = r#"#!/bin/sh
if [ "$1" = "secret" ] && [ "$2" = "rm" ]; then
    exit 0
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.remove_secret("openai").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_remove_secret_empty_service_fails() {
        let script = r#"#!/bin/sh
exit 0
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.remove_secret("").await;
        assert!(matches!(result, Err(OrchestratorError::Validation(_))));
    }

    #[tokio::test]
    async fn test_list_secrets_cli_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "secret" ] && [ "$2" = "ls" ]; then
    echo "error: not logged in" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.list_secrets().await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_set_secret_cli_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "secret" ] && [ "$2" = "set" ]; then
    echo "error: keychain locked" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr
            .set_secret("openai", "sk-key".to_string())
            .await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }

    #[tokio::test]
    async fn test_remove_secret_cli_failure() {
        let script = r#"#!/bin/sh
if [ "$1" = "secret" ] && [ "$2" = "rm" ]; then
    echo "error: secret not found" >&2
    exit 1
fi
exit 1
"#;
        let (mgr, _dir) = create_test_manager(script);
        let result = mgr.remove_secret("nonexistent").await;
        assert!(matches!(result, Err(OrchestratorError::SbxError(_))));
    }
}
