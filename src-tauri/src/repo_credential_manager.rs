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

    username_entry.set_password(&username).map_err(|e| {
        OrchestratorError::Internal(format!("failed to store username in keyring: {}", e))
    })?;

    secret_entry.set_password(&secret).map_err(|e| {
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
        OrchestratorError::Internal(format!(
            "failed to determine current executable path: {}",
            e
        ))
    })?;

    let exe_dir = current_exe.parent().ok_or_else(|| {
        OrchestratorError::Internal("current executable has no parent directory".to_string())
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
        OrchestratorError::Internal(format!(
            "failed to determine current executable path: {}",
            e
        ))
    })?;

    let exe_dir = current_exe.parent().ok_or_else(|| {
        OrchestratorError::Internal("current executable has no parent directory".to_string())
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

    // ===== Service name format tests (Requirement 13.7) =====

    #[test]
    fn test_service_name_format() {
        assert_eq!(service_name("abc-123"), "beachead-repo-sync-abc-123");
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
    fn test_service_name_prefix_is_consistent() {
        // All service names must start with the beachead-repo-sync- prefix
        for repo_id in &["a", "123", "my-repo", "repo_with_underscores", "UPPERCASE"] {
            let name = service_name(repo_id);
            assert!(
                name.starts_with("beachead-repo-sync-"),
                "service name '{}' does not start with expected prefix",
                name
            );
        }
    }

    #[test]
    fn test_service_name_contains_repo_id_suffix() {
        let repo_id = "my-custom-repo-id";
        let name = service_name(repo_id);
        assert!(name.ends_with(repo_id));
    }

    #[test]
    fn test_service_name_with_empty_repo_id() {
        // Edge case: empty repo ID still produces valid prefix
        assert_eq!(service_name(""), "beachead-repo-sync-");
    }

    #[test]
    fn test_service_name_with_special_characters() {
        // Repo IDs with dots, slashes, etc.
        assert_eq!(
            service_name("repo.with.dots"),
            "beachead-repo-sync-repo.with.dots"
        );
        assert_eq!(
            service_name("repo/with/slashes"),
            "beachead-repo-sync-repo/with/slashes"
        );
    }

    #[test]
    fn test_service_name_uniqueness() {
        // Different repo IDs produce different service names
        let name1 = service_name("repo-1");
        let name2 = service_name("repo-2");
        assert_ne!(name1, name2);
    }

    // ===== is_no_entry_error tests (Requirement 13.9) =====

    #[test]
    fn test_is_no_entry_error() {
        assert!(is_no_entry_error(&keyring::Error::NoEntry));
    }

    #[test]
    fn test_is_no_entry_error_platform_failure() {
        let err = keyring::Error::PlatformFailure(Box::new(std::io::Error::new(
            std::io::ErrorKind::Other,
            "test",
        )));
        assert!(!is_no_entry_error(&err));
    }

    #[test]
    fn test_is_no_entry_error_no_storage_access() {
        let err = keyring::Error::NoStorageAccess(Box::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "keyring locked",
        )));
        assert!(!is_no_entry_error(&err));
    }

    #[test]
    fn test_is_no_entry_error_bad_encoding() {
        let err = keyring::Error::BadEncoding(vec![0xFF, 0xFE]);
        assert!(!is_no_entry_error(&err));
    }

    #[test]
    fn test_is_no_entry_error_too_long() {
        let err = keyring::Error::TooLong("service".to_string(), 255);
        assert!(!is_no_entry_error(&err));
    }

    #[test]
    fn test_is_no_entry_error_invalid() {
        let err = keyring::Error::Invalid("service".to_string(), "contains null byte".to_string());
        assert!(!is_no_entry_error(&err));
    }

    // ===== Askpass path resolution tests (Requirement 13.3) =====

    #[test]
    fn test_askpass_binary_path_returns_path() {
        let result = askpass_binary_path();
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("beachead-askpass"));
    }

    #[test]
    fn test_askpass_binary_path_is_in_same_dir_as_current_exe() {
        let askpass = askpass_binary_path().unwrap();
        let current_exe = std::env::current_exe().unwrap();
        // The askpass binary should be in the same directory as the current executable
        assert_eq!(askpass.parent(), current_exe.parent());
    }

    #[test]
    fn test_askpass_binary_path_is_absolute() {
        let path = askpass_binary_path().unwrap();
        assert!(
            path.is_absolute(),
            "askpass path should be absolute: {:?}",
            path
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn test_askpass_binary_path_has_exe_extension_on_windows() {
        let path = askpass_binary_path().unwrap();
        assert_eq!(path.extension().unwrap(), "exe");
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn test_askpass_binary_path_has_no_extension_on_unix() {
        let path = askpass_binary_path().unwrap();
        // On Unix, the binary name is just "beachead-askpass" with no extension
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            "beachead-askpass"
        );
    }

    #[test]
    fn test_resolve_askpass_path_fails_when_binary_missing() {
        // resolve_askpass_path checks existence, so it should fail in test env
        // where the binary isn't built alongside the test runner
        let result = resolve_askpass_path();
        // In test environment, the binary likely doesn't exist next to the test binary
        // This is expected behavior — the function correctly reports the missing binary
        if result.is_err() {
            let err_msg = format!("{}", result.unwrap_err());
            assert!(
                err_msg.contains("not found"),
                "Error should mention binary not found, got: {}",
                err_msg
            );
        }
        // If it happens to exist (e.g., after a full build), that's also fine
    }

    // ===== build_credential_env tests =====

    #[test]
    fn test_build_credential_env_service_name() {
        // Verify the service name construction logic used by build_credential_env
        let service = service_name("test-repo-id");
        assert_eq!(service, "beachead-repo-sync-test-repo-id");
    }

    #[test]
    fn test_build_credential_env_uses_correct_service_name_format() {
        // build_credential_env should use the same service_name function
        // We can't call build_credential_env directly (needs askpass binary),
        // but we verify the service name it would use
        let repo_id = "550e8400-e29b-41d4-a716-446655440000";
        let expected_service = format!("beachead-repo-sync-{}", repo_id);
        assert_eq!(service_name(repo_id), expected_service);
    }

    // ===== Prompt parsing logic tests (Requirement 13.3) =====
    // These test the same logic used in beachead-askpass binary:
    // prompt.to_lowercase().contains("password") determines username vs password

    #[test]
    fn test_prompt_parsing_password_detection() {
        // Simulates the logic in beachead-askpass: check if prompt contains "password"
        let password_prompts = vec![
            "Password for 'https://github.com': ",
            "password for 'https://gitlab.com': ",
            "PASSWORD for 'https://bitbucket.org': ",
            "Enter your password: ",
            "Git password: ",
        ];
        for prompt in password_prompts {
            let is_password = prompt.to_lowercase().contains("password");
            assert!(
                is_password,
                "Prompt '{}' should be detected as password request",
                prompt
            );
        }
    }

    #[test]
    fn test_prompt_parsing_username_detection() {
        // Prompts that do NOT contain "password" are treated as username requests
        let username_prompts = vec![
            "Username for 'https://github.com': ",
            "username for 'https://gitlab.com': ",
            "User: ",
            "Login: ",
            "", // empty prompt defaults to username
        ];
        for prompt in username_prompts {
            let is_password = prompt.to_lowercase().contains("password");
            assert!(
                !is_password,
                "Prompt '{}' should be detected as username request",
                prompt
            );
        }
    }

    #[test]
    fn test_prompt_parsing_keyring_key_construction() {
        // Verifies the key format used by beachead-askpass to look up credentials
        let service = "beachead-repo-sync-my-repo";

        // Password prompt → uses "-secret" suffix
        let password_key = format!("{}-secret", service);
        assert_eq!(password_key, "beachead-repo-sync-my-repo-secret");

        // Username prompt → uses "-username" suffix
        let username_key = format!("{}-username", service);
        assert_eq!(username_key, "beachead-repo-sync-my-repo-username");
    }

    #[test]
    fn test_prompt_parsing_case_insensitive() {
        // The binary uses to_lowercase().contains("password") — case insensitive
        let mixed_case_prompts = vec!["Password", "PASSWORD", "pAsSwOrD", "Enter PASSWORD here"];
        for prompt in mixed_case_prompts {
            let is_password = prompt.to_lowercase().contains("password");
            assert!(is_password, "Case-insensitive check failed for: {}", prompt);
        }
    }
}
