use dashmap::DashMap;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

/// Maximum captured output size (1 MB).
const MAX_OUTPUT_BYTES: usize = 1_048_576;

/// Timeout for network git operations (fetch, push, pull from remote, clone from URL).
const NETWORK_TIMEOUT_SECS: u64 = 120;

/// Timeout for local git operations (log, status, remote, config, branch).
const LOCAL_TIMEOUT_SECS: u64 = 30;

/// Successful git command output.
#[derive(Debug, Clone)]
pub struct GitOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Errors that can occur when executing git commands.
#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git binary not found at path: {0}")]
    NotFound(String),

    #[error("git command timed out after {timeout_secs}s: git {}", args.join(" "))]
    Timeout { args: Vec<String>, timeout_secs: u64 },

    #[error("git authentication failed: {stderr}")]
    AuthFailure { stderr: String },

    #[error("git command exited with code {exit_code}: {stderr}")]
    NonZeroExit {
        exit_code: i32,
        stderr: String,
        args: Vec<String>,
    },

    #[error("invalid path {path}: {reason}")]
    InvalidPath { path: String, reason: String },

    #[error("merge conflict: {stderr}")]
    MergeConflict { stderr: String },

    #[error("non-fast-forward push rejected: {stderr}")]
    NonFastForward { stderr: String },
}

/// Environment variables for credential injection via GIT_ASKPASS.
#[derive(Debug, Clone)]
pub struct CredentialEnv {
    pub askpass_path: String,
    pub service_name: String,
}

/// Git CLI wrapper that executes git commands with proper error handling,
/// credential injection, timeouts, and per-path locking.
pub struct GitCli {
    git_path: String,
    /// Per-path locks to prevent concurrent git operations on the same repository.
    locks: DashMap<PathBuf, Arc<Mutex<()>>>,
}

impl GitCli {
    /// Create a new GitCli instance with the given git binary path.
    pub fn new(git_path: String) -> Self {
        Self {
            git_path,
            locks: DashMap::new(),
        }
    }

    /// Returns the configured git binary path.
    pub fn git_path(&self) -> &str {
        &self.git_path
    }

    /// Execute a git command with credential injection and timeout.
    ///
    /// - `cwd`: Working directory (must contain `.git`).
    /// - `args`: Git command arguments (e.g., `["fetch", "origin"]`).
    /// - `credential_env`: Optional credential environment for GIT_ASKPASS.
    /// - `network_op`: If true, uses 120s timeout; otherwise 30s.
    pub async fn exec(
        &self,
        cwd: &Path,
        args: &[&str],
        credential_env: Option<&CredentialEnv>,
        network_op: bool,
    ) -> Result<GitOutput, GitError> {
        // Validate path exists and contains .git
        if !cwd.exists() {
            return Err(GitError::InvalidPath {
                path: cwd.display().to_string(),
                reason: "directory does not exist".to_string(),
            });
        }
        if !cwd.join(".git").exists() {
            return Err(GitError::InvalidPath {
                path: cwd.display().to_string(),
                reason: "directory does not contain a .git folder".to_string(),
            });
        }

        // Acquire per-path lock (prevents concurrent git on same repo)
        let lock = self
            .locks
            .entry(cwd.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        let _guard = lock.lock().await;

        let timeout_duration = if network_op {
            Duration::from_secs(NETWORK_TIMEOUT_SECS)
        } else {
            Duration::from_secs(LOCAL_TIMEOUT_SECS)
        };

        let mut cmd = Command::new(&self.git_path);
        cmd.args(args)
            .current_dir(cwd)
            .env("GIT_TERMINAL_PROMPT", "0");

        // Inject credential helper env vars
        if let Some(cred) = credential_env {
            cmd.env("GIT_ASKPASS", &cred.askpass_path);
            cmd.env("BEACHEAD_KEYRING_SERVICE", &cred.service_name);
        }

        // Spawn and capture with timeout
        let args_owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();

        let result = timeout(timeout_duration, cmd.output()).await;

        match result {
            Err(_elapsed) => Err(GitError::Timeout {
                args: args_owned,
                timeout_secs: if network_op {
                    NETWORK_TIMEOUT_SECS
                } else {
                    LOCAL_TIMEOUT_SECS
                },
            }),
            Ok(Err(_io_err)) => Err(GitError::NotFound(self.git_path.clone())),
            Ok(Ok(output)) => {
                let stdout = truncate_output(&output.stdout);
                let stderr = truncate_output(&output.stderr);
                let exit_code = output.status.code().unwrap_or(-1);

                if exit_code == 0 {
                    Ok(GitOutput {
                        exit_code,
                        stdout,
                        stderr,
                    })
                } else {
                    Err(classify_git_error(&stderr, exit_code, args_owned))
                }
            }
        }
    }
}

/// Truncate output bytes to MAX_OUTPUT_BYTES, keeping the tail if exceeded.
fn truncate_output(bytes: &[u8]) -> String {
    if bytes.len() <= MAX_OUTPUT_BYTES {
        String::from_utf8_lossy(bytes).to_string()
    } else {
        let truncated = &bytes[bytes.len() - MAX_OUTPUT_BYTES..];
        format!(
            "[truncated: output exceeded 1MB, showing last 1MB]\n{}",
            String::from_utf8_lossy(truncated)
        )
    }
}

/// Classify a git error based on stderr content and exit code.
pub fn classify_git_error(stderr: &str, exit_code: i32, args: Vec<String>) -> GitError {
    let sanitized = sanitize_stderr(stderr);

    if sanitized.contains("Authentication failed")
        || sanitized.contains("could not read Username")
    {
        GitError::AuthFailure { stderr: sanitized }
    } else if sanitized.contains("CONFLICT") || sanitized.contains("Automatic merge failed") {
        GitError::MergeConflict { stderr: sanitized }
    } else if sanitized.contains("non-fast-forward") {
        GitError::NonFastForward { stderr: sanitized }
    } else {
        GitError::NonZeroExit {
            exit_code,
            stderr: sanitized,
            args,
        }
    }
}

/// Redact embedded credentials from URLs in git output.
/// Replaces anything between "://" and "@" with "***".
/// Example: "https://user:token@github.com" → "https://***@github.com"
pub fn sanitize_stderr(stderr: &str) -> String {
    let re = Regex::new(r"://[^@]+@").unwrap();
    re.replace_all(stderr, "://***@").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_stderr_redacts_credentials() {
        let input = "fatal: Authentication failed for 'https://user:token@github.com/repo.git'";
        let result = sanitize_stderr(input);
        assert_eq!(
            result,
            "fatal: Authentication failed for 'https://***@github.com/repo.git'"
        );
    }

    #[test]
    fn test_sanitize_stderr_no_credentials() {
        let input = "fatal: not a git repository";
        let result = sanitize_stderr(input);
        assert_eq!(result, "fatal: not a git repository");
    }

    #[test]
    fn test_sanitize_stderr_multiple_urls() {
        let input = "https://user1:pass1@host1.com and https://user2:pass2@host2.com";
        let result = sanitize_stderr(input);
        assert_eq!(result, "https://***@host1.com and https://***@host2.com");
    }

    #[test]
    fn test_sanitize_stderr_ssh_url_unchanged() {
        let input = "fatal: Could not read from remote repository 'git@github.com:user/repo.git'";
        let result = sanitize_stderr(input);
        // SSH URLs don't have :// so they should be unchanged
        assert_eq!(
            result,
            "fatal: Could not read from remote repository 'git@github.com:user/repo.git'"
        );
    }

    #[test]
    fn test_classify_git_error_auth_failure() {
        let err = classify_git_error(
            "fatal: Authentication failed for 'https://github.com'",
            128,
            vec!["push".to_string()],
        );
        assert!(matches!(err, GitError::AuthFailure { .. }));
    }

    #[test]
    fn test_classify_git_error_could_not_read_username() {
        let err = classify_git_error(
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
            128,
            vec!["fetch".to_string()],
        );
        assert!(matches!(err, GitError::AuthFailure { .. }));
    }

    #[test]
    fn test_classify_git_error_merge_conflict() {
        let err = classify_git_error(
            "CONFLICT (content): Merge conflict in file.txt\nAutomatic merge failed",
            1,
            vec!["merge".to_string()],
        );
        assert!(matches!(err, GitError::MergeConflict { .. }));
    }

    #[test]
    fn test_classify_git_error_non_fast_forward() {
        let err = classify_git_error(
            "error: failed to push some refs\nhint: Updates were rejected because the tip of your current branch is behind\nhint: its remote counterpart. non-fast-forward",
            1,
            vec!["push".to_string(), "origin".to_string(), "main".to_string()],
        );
        assert!(matches!(err, GitError::NonFastForward { .. }));
    }

    #[test]
    fn test_classify_git_error_generic_non_zero() {
        let err = classify_git_error(
            "fatal: bad object HEAD",
            128,
            vec!["log".to_string()],
        );
        match err {
            GitError::NonZeroExit {
                exit_code,
                stderr,
                args,
            } => {
                assert_eq!(exit_code, 128);
                assert_eq!(stderr, "fatal: bad object HEAD");
                assert_eq!(args, vec!["log"]);
            }
            _ => panic!("Expected NonZeroExit"),
        }
    }

    #[test]
    fn test_classify_git_error_sanitizes_credentials_in_output() {
        let err = classify_git_error(
            "fatal: Authentication failed for 'https://user:secret@github.com/repo.git'",
            128,
            vec!["push".to_string()],
        );
        match err {
            GitError::AuthFailure { stderr } => {
                assert!(!stderr.contains("secret"));
                assert!(stderr.contains("***"));
            }
            _ => panic!("Expected AuthFailure"),
        }
    }

    #[test]
    fn test_truncate_output_under_limit() {
        let data = b"hello world";
        let result = truncate_output(data);
        assert_eq!(result, "hello world");
    }

    #[test]
    fn test_truncate_output_over_limit() {
        // Create data larger than 1MB
        let data = vec![b'x'; MAX_OUTPUT_BYTES + 100];
        let result = truncate_output(&data);
        assert!(result.starts_with("[truncated: output exceeded 1MB, showing last 1MB]"));
        // The actual content portion should be exactly MAX_OUTPUT_BYTES of 'x'
        let content_after_header = result
            .strip_prefix("[truncated: output exceeded 1MB, showing last 1MB]\n")
            .unwrap();
        assert_eq!(content_after_header.len(), MAX_OUTPUT_BYTES);
    }

    #[test]
    fn test_truncate_output_exactly_at_limit() {
        let data = vec![b'y'; MAX_OUTPUT_BYTES];
        let result = truncate_output(&data);
        // Should not be truncated
        assert!(!result.starts_with("[truncated"));
        assert_eq!(result.len(), MAX_OUTPUT_BYTES);
    }
}
