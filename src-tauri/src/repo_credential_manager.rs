//! Repo Credential Manager: keyring-based credential storage for Repo Sync.
//!
//! This module provides functions to store, retrieve, and delete git credentials
//! in the OS keyring for use with the `beachead-askpass` GIT_ASKPASS helper.
//!
//! Keyring service name pattern: `beachead-repo-sync-<repo-id>`
//! Each repo has two keyring entries:
//!   - `<service>-username` — the git username
//!   - `<service>-secret` — the token or password
//!
//! SECURITY:
//! - Credential values are zeroized from memory after storage.
//! - Credential values are never logged or included in error messages.
//! - The `resolve_askpass_path()` function locates the `beachead-askpass` binary
//!   relative to the current executable.

use std::path::PathBuf;

use keyring::Entry;
use zeroize::Zeroize;

use crate::error::OrchestratorError;
use crate::git_cli::CredentialEnv;

/// The keyring user field used for all beachead repo sync entries.
const KEYRING_USER: &str = "beachead";

/// Constructs the keyring service name for a given repo ID.
///
/// Pattern: `beachead-repo-sync-<repo-id>`
fn service_name(repo_id: &str) -> String {
    format!("beachead-repo-sync-{}", repo_id)
}

/// Store credentials (username + secret) in the OS keyring for a managed repo.
///
/// Creates two keyring entries:
/// - `beachead-repo-sync-<repo-id>-username` with the username value
/// - `beachead-repo-sync-<repo-id>-secret` with the token/password value
///
/// If entries already exist, they are overwritten.
///
/// # Errors
/// Returns `OrchestratorError::Internal` if the keyring is unavailable or locked.
pub fn store_credentials(
    repo_id: &str,
    mut username: String,
    mut secret: String,
) -> Result<(), OrchestratorError> {
    let service = service_name(repo_id);

    let username_key = format!("{}-username", service);
    let secret_key = format!("{}-secret", service);

    let username_entry = Entry::new(&username_key, KEYRING_USER)
        .map_err(|e| OrchestratorError::Internal(format!("keyring access failed: {}", e)))?;

    let secret_entry = Entry::new(&secret_key, KEYRING_USER)
        .map_err(|e| OrchestratorError::Internal(format!("keyring access failed: {}", e)))?;

    username_entry
        .set_password(&username)
        .map_err(|e| OrchestratorError::Internal(format!("failed to store username in keyring: {}", e)))?;

    secret_entry
        .set_password(&secret)
        .map_err(|e| {
            // Best-effort cleanup: remove the username entry if secret storage fails
            let _ = username_entry.delete_credential();
            OrchestratorError::Internal(format!("failed to store secret in keyring: {}", e))
        })?;

    // Zeroize credential values from memory
    username.zeroize();
    secret.zeroize();

    Ok(())
}

/// Delete credentials from the OS keyring for a managed repo.
///
/// Removes both the username and secret entries. If either entry does not exist,
/// the deletion is considered successful (idempotent).
///
/// # Errors
/// Returns `OrchestratorError::Internal` if the keyring is unavailable or locked
/// (distinct from "entry not found" which is silently ignored).
pub fn delete_credentials(repo_id: &str) -> Result<(), OrchestratorError> {
    let service = service_name(repo_id);

    let username_key = format!("{}-username", service);
    let secret_key = format!("{}-secret", service);

    let username_entry = Entry::new(&username_key, KEYRING_USER)
        .map_err(|e| OrchestratorError::Internal(format!("keyring access failed: {}", e)))?;

    let secret_entry = Entry::new(&secret_key, KEYRING_USER)
        .map_err(|e| OrchestratorError::Internal(format!("keyring access failed: {}", e)))?;

    // Delete both entries, ignoring "not found" errors (NoEntry)
    if let Err(e) = username_entry.delete_credential() {
        if !is_no_entry_error(&e) {
            return Err(OrchestratorError::Internal(format!(
                "failed to delete username from keyring: {}",
                e
            )));
        }
    }

    if let Err(e) = secret_entry.delete_credential() {
        if !is_no_entry_error(&e) {
            return Err(OrchestratorError::Internal(format!(
                "failed to delete secret from keyring: {}",
                e
            )));
        }
    }

    Ok(())
}

/// Check whether credentials are configured in the OS keyring for a managed repo.
///
/// Returns `true` if both username and secret entries exist and are readable.
/// Returns `false` if either entry is missing.
///
/// # Errors
/// Returns `OrchestratorError::Internal` if the keyring is unavailable or locked.
pub fn credentials_configured(repo_id: &str) -> Result<bool, OrchestratorError> {
    let service = service_name(repo_id);

    let username_key = format!("{}-username", service);
    let secret_key = format!("{}-secret", service);

    let username_entry = Entry::new(&username_key, KEYRING_USER)
        .map_err(|e| OrchestratorError::Internal(format!("keyring access failed: {}", e)))?;

    let secret_entry = Entry::new(&secret_key, KEYRING_USER)
        .map_err(|e| OrchestratorError::Internal(format!("keyring access failed: {}", e)))?;

    let username_exists = match username_entry.get_password() {
        Ok(mut val) => {
            val.zeroize();
            true
        }
        Err(e) if is_no_entry_error(&e) => false,
        Err(e) => {
            return Err(OrchestratorError::Internal(format!(
                "keyring unavailable: {}",
                e
            )));
        }
    };

    let secret_exists = match secret_entry.get_password() {
        Ok(mut val) => {
            val.zeroize();
            true
        }
        Err(e) if is_no_entry_error(&e) => false,
        Err(e) => {
            return Err(OrchestratorError::Internal(format!(
                "keyring unavailable: {}",
                e
            )));
        }
    };

    Ok(username_exists && secret_exists)
}

/// Resolve the path to the `beachead-askpass` binary relative to the current executable.
///
/// The askpass binary is expected to be in the same directory as the main application binary.
/// Returns the full path as a string suitable for use in `GIT_ASKPASS` environment variable.
///
/// # Errors
/// Returns `OrchestratorError::Internal` if the current executable path cannot be determined
/// or if the askpass binary does not exist at the expected location.
pub fn resolve_askpass_path() -> Result<String, OrchestratorError> {
    let current_exe = std::env::current_exe().map_err(|e| {
        OrchestratorError::Internal(format!("failed to determine current executable path: {}", e))
    })?;

    let exe_dir = current_exe.parent().ok_or_else(|| {
        OrchestratorError::Internal(
            "current executable has no parent directory".to_string(),
        )
    })?;

    let askpass_path = exe_dir.join("beachead-askpass");

    // On Windows, the binary has a .exe extension
    #[cfg(target_os = "windows")]
    let askpass_path = askpass_path.with_extension("exe");

    if !askpass_path.exists() {
        return Err(OrchestratorError::Internal(format!(
            "beachead-askpass binary not found at: {}",
            askpass_path.display()
        )));
    }

    Ok(askpass_path.to_string_lossy().to_string())
}

/// Build a `CredentialEnv` for a given repo ID, resolving the askpass path.
///
/// This is a convenience function that combines `resolve_askpass_path()` with
/// the service name construction.
pub fn build_credential_env(repo_id: &str) -> Result<CredentialEnv, OrchestratorError> {
    let askpass_path = resolve_askpass_path()?;
    Ok(CredentialEnv {
        askpass_path,
        service_name: service_name(repo_id),
    })
}

/// Returns the askpass binary path for the current platform.
/// This is useful for testing or when the binary location needs to be known
/// without checking existence.
pub fn askpass_binary_path() -> Result<PathBuf, OrchestratorError> {
    let current_exe = std::env::current_exe().map_err(|e| {
        OrchestratorError::Internal(format!("failed to determine current executable path: {}", e))
    })?;

    let exe_dir = current_exe.parent().ok_or_else(|| {
        OrchestratorError::Internal(
            "current executable has no parent directory".to_string(),
        )
    })?;

    let askpass_path = exe_dir.join("beachead-askpass");

    #[cfg(target_os = "windows")]
    let askpass_path = askpass_path.with_extension("exe");

    Ok(askpass_path)
}

/// Check if a keyring error indicates that the entry was not found.
fn is_no_entry_error(e: &keyring::Error) -> bool {
    matches!(e, keyring::Error::NoEntry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_name_format() {
        assert_eq!(
            service_name("abc-123"),
            "beachead-repo-sync-abc-123"
        );
    }

    #[test]
    fn test_service_name_with_uuid() {
        let repo_id = "550e8400-e29b-41d4-a716-446655440000";
        assert_eq!(
            service_name(repo_id),
            "beachead-repo-sync-550e8400-e29b-41d4-a716-446655440000"
        );
    }

    #[test]
    fn test_is_no_entry_error() {
        assert!(is_no_entry_error(&keyring::Error::NoEntry));
    }

    #[test]
    fn test_is_no_entry_error_other_variants() {
        // Other error variants should not match
        let other_err = keyring::Error::PlatformFailure(
            Box::new(std::io::Error::new(std::io::ErrorKind::Other, "test")),
        );
        assert!(!is_no_entry_error(&other_err));
    }

    #[test]
    fn test_askpass_binary_path_returns_path() {
        // This test verifies the function returns a path (may not exist in test env)
        let result = askpass_binary_path();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.file_name().unwrap().to_string_lossy().contains("beachead-askpass"));
    }

    #[test]
    fn test_build_credential_env_service_name() {
        // We can't fully test this without the askpass binary present,
        // but we can verify the service name construction logic
        let service = service_name("test-repo-id");
        assert_eq!(service, "beachead-repo-sync-test-repo-id");
    }
}
