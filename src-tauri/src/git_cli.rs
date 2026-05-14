use dashmap::DashMap;
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;
use tokio::time::{timeout, Duration};

/// Maximum captured output size (1 MB).
pub const MAX_OUTPUT_BYTES: usize = 1_048_576;

/// Timeout for network git operations (fetch, push, pull from remote, clone from URL).
pub const NETWORK_TIMEOUT_SECS: u64 = 120;

/// Timeout for local git operations (log, status, remote, config, branch).
pub const LOCAL_TIMEOUT_SECS: u64 = 30;

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

    /// Count commits ahead/behind between two refs.
    /// Returns (ahead, behind) where ahead = local commits not in remote.
    pub async fn ahead_behind(
        &self,
        cwd: &Path,
        local_ref: &str,
        remote_ref: &str,
    ) -> Result<(u32, u32), GitError> {
        let range = format!("{}...{}", local_ref, remote_ref);
        let output = self
            .exec(
                cwd,
                &["rev-list", "--left-right", "--count", &range],
                None,
                false,
            )
            .await?;
        let parts: Vec<&str> = output.stdout.trim().split('\t').collect();
        let ahead = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(0);
        let behind = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        Ok((ahead, behind))
    }

    /// Get the current branch name via `git branch --show-current`.
    /// Returns an empty string if in detached HEAD state.
    pub async fn get_current_branch(&self, cwd: &Path) -> Result<String, GitError> {
        let output = self
            .exec(cwd, &["branch", "--show-current"], None, false)
            .await?;
        Ok(output.stdout.trim().to_string())
    }

    /// List all remote names via `git remote`.
    pub async fn list_remote_names(&self, cwd: &Path) -> Result<Vec<String>, GitError> {
        let output = self.exec(cwd, &["remote"], None, false).await?;
        let names = output
            .stdout
            .lines()
            .map(|line| line.trim().to_string())
            .filter(|line| !line.is_empty())
            .collect();
        Ok(names)
    }

    /// Get the URL for a specific remote via `git remote get-url`.
    /// Returns None if the remote does not exist.
    pub async fn get_remote_url(
        &self,
        cwd: &Path,
        remote_name: &str,
    ) -> Result<Option<String>, GitError> {
        let result = self
            .exec(cwd, &["remote", "get-url", remote_name], None, false)
            .await;
        match result {
            Ok(output) => {
                let url = output.stdout.trim().to_string();
                if url.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(url))
                }
            }
            Err(GitError::NonZeroExit { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Get the working tree status in porcelain format via `git status --porcelain`.
    pub async fn status_porcelain(&self, cwd: &Path) -> Result<String, GitError> {
        let output = self
            .exec(cwd, &["status", "--porcelain"], None, false)
            .await?;
        Ok(output.stdout.clone())
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
pub fn truncate_output(bytes: &[u8]) -> String {
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

    /// Helper to create a temporary git repo for integration tests.
    fn create_temp_git_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        // Create an initial commit so HEAD exists
        std::fs::write(dir.path().join("README.md"), "# Test").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial commit"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        dir
    }

    #[tokio::test]
    async fn test_get_current_branch() {
        let dir = create_temp_git_repo();
        let git = GitCli::new("git".to_string());
        let branch = git.get_current_branch(dir.path()).await.unwrap();
        // Default branch is typically "main" or "master"
        assert!(!branch.is_empty());
    }

    #[tokio::test]
    async fn test_list_remote_names_empty() {
        let dir = create_temp_git_repo();
        let git = GitCli::new("git".to_string());
        let remotes = git.list_remote_names(dir.path()).await.unwrap();
        assert!(remotes.is_empty());
    }

    #[tokio::test]
    async fn test_list_remote_names_with_remote() {
        let dir = create_temp_git_repo();
        // Add a remote
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://example.com/repo.git"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["remote", "add", "upstream", "https://example.com/upstream.git"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let git = GitCli::new("git".to_string());
        let remotes = git.list_remote_names(dir.path()).await.unwrap();
        assert_eq!(remotes.len(), 2);
        assert!(remotes.contains(&"origin".to_string()));
        assert!(remotes.contains(&"upstream".to_string()));
    }

    #[tokio::test]
    async fn test_get_remote_url_exists() {
        let dir = create_temp_git_repo();
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://example.com/repo.git"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        let git = GitCli::new("git".to_string());
        let url = git.get_remote_url(dir.path(), "origin").await.unwrap();
        assert_eq!(url, Some("https://example.com/repo.git".to_string()));
    }

    #[tokio::test]
    async fn test_get_remote_url_not_exists() {
        let dir = create_temp_git_repo();
        let git = GitCli::new("git".to_string());
        let url = git.get_remote_url(dir.path(), "nonexistent").await.unwrap();
        assert_eq!(url, None);
    }

    #[tokio::test]
    async fn test_status_porcelain_clean() {
        let dir = create_temp_git_repo();
        let git = GitCli::new("git".to_string());
        let status = git.status_porcelain(dir.path()).await.unwrap();
        assert!(status.trim().is_empty());
    }

    #[tokio::test]
    async fn test_status_porcelain_dirty() {
        let dir = create_temp_git_repo();
        // Create an untracked file
        std::fs::write(dir.path().join("new_file.txt"), "content").unwrap();

        let git = GitCli::new("git".to_string());
        let status = git.status_porcelain(dir.path()).await.unwrap();
        assert!(!status.trim().is_empty());
        assert!(status.contains("new_file.txt"));
    }

    #[tokio::test]
    async fn test_ahead_behind_same_ref() {
        let dir = create_temp_git_repo();
        let git = GitCli::new("git".to_string());
        let branch = git.get_current_branch(dir.path()).await.unwrap();
        // Comparing a branch to itself should yield (0, 0)
        let (ahead, behind) = git.ahead_behind(dir.path(), &branch, &branch).await.unwrap();
        assert_eq!(ahead, 0);
        assert_eq!(behind, 0);
    }

    #[tokio::test]
    async fn test_ahead_behind_with_commits() {
        let dir = create_temp_git_repo();
        let git = GitCli::new("git".to_string());
        let branch = git.get_current_branch(dir.path()).await.unwrap();

        // Create a branch at current HEAD, then add commits to main
        std::process::Command::new("git")
            .args(["branch", "base"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Add two more commits on the current branch
        std::fs::write(dir.path().join("file1.txt"), "a").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "second"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        std::fs::write(dir.path().join("file2.txt"), "b").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "third"])
            .current_dir(dir.path())
            .output()
            .unwrap();

        // Current branch is 2 ahead of "base", base is 0 behind
        let (ahead, behind) = git.ahead_behind(dir.path(), &branch, "base").await.unwrap();
        assert_eq!(ahead, 2);
        assert_eq!(behind, 0);

        // Reverse: base is 0 ahead, 2 behind current branch
        let (ahead, behind) = git.ahead_behind(dir.path(), "base", &branch).await.unwrap();
        assert_eq!(ahead, 0);
        assert_eq!(behind, 2);
    }
}
