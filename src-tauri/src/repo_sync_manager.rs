//! Repo Sync Manager: business logic for git remote synchronization.
//!
//! Manages the two-directory architecture where the agent works in a remote-free
//! workspace and a host-side mirror holds remotes and credentials. All sync
//! operations are user-initiated and run on the host via the git CLI.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::Utc;
use tokio::task::JoinHandle;

use crate::db::Database;
use crate::db_ops;
use crate::error::OrchestratorError;
use crate::git_cli::GitCli;
use crate::secret_scanner::SecretScanner;
use crate::types::ManagedRepo;

/// Manages git remote synchronization using a two-directory architecture.
///
/// The agent works in a remote-free workspace; a host-side mirror holds remotes
/// and credentials. All sync is user-initiated, all git operations run on the
/// host via CLI.
pub struct RepoSyncManager {
    pub db: Arc<Database>,
    pub git: Arc<GitCli>,
    pub mirrors_dir: PathBuf,
    pub scanner: SecretScanner,
    /// Background task handle for periodic commit checks.
    pub check_handle: Option<JoinHandle<()>>,
}

impl RepoSyncManager {
    /// Create a new RepoSyncManager.
    ///
    /// # Arguments
    /// - `db`: Shared database connection.
    /// - `git`: Shared git CLI wrapper.
    /// - `mirrors_dir`: Root directory where mirror repositories are stored.
    pub fn new(db: Arc<Database>, git: Arc<GitCli>, mirrors_dir: PathBuf) -> Self {
        Self {
            db,
            git,
            mirrors_dir,
            scanner: SecretScanner::new(),
            check_handle: None,
        }
    }

    /// Returns the platform-specific default mirrors directory.
    ///
    /// - Linux: `~/.local/share/beachead/mirrors`
    /// - macOS: `~/Library/Application Support/beachead/mirrors`
    /// - Windows: `%APPDATA%\beachead\mirrors`
    pub fn default_mirrors_dir() -> PathBuf {
        #[cfg(target_os = "linux")]
        {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("beachead/mirrors")
        }

        #[cfg(target_os = "macos")]
        {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("beachead/mirrors")
        }

        #[cfg(target_os = "windows")]
        {
            dirs::data_dir()
                .unwrap_or_else(|| PathBuf::from("C:\\Temp"))
                .join("beachead\\mirrors")
        }

        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        {
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("/tmp"))
                .join("beachead/mirrors")
        }
    }

    /// Resolve a branch name from a pattern, with deduplication suffix.
    ///
    /// Takes a managed repo and a pattern string (e.g., `ai/<persona-name>/<date>`)
    /// and resolves it to a concrete branch name:
    /// 1. Replaces `<persona-name>` with the actual persona name
    /// 2. Replaces `<date>` with current date in YYYY-MM-DD format
    /// 3. If the resulting branch name already exists on the remote, appends `-2`, `-3`, etc.
    ///
    /// # Arguments
    /// - `repo`: The managed repo record (used to look up persona name and mirror path).
    /// - `pattern`: The branch name pattern to resolve.
    ///
    /// # Returns
    /// A unique branch name that does not exist on the remote.
    pub async fn resolve_branch_name(
        &self,
        repo: &ManagedRepo,
        pattern: &str,
    ) -> Result<String, OrchestratorError> {
        // Look up persona name from DB
        let persona_name = self.db.with_conn(|conn| {
            let persona = db_ops::get_persona(conn, &repo.persona_id)?;
            Ok(persona.name)
        })?;

        // Replace placeholders in pattern
        let today = Utc::now().format("%Y-%m-%d").to_string();
        let base_name = pattern
            .replace("<persona-name>", &persona_name)
            .replace("<date>", &today);

        // Get list of remote branches to check for conflicts
        let mirror_path = Path::new(&repo.mirror_path);
        let remote_branches = self.list_remote_branch_names(mirror_path).await?;

        // If the base name doesn't conflict, use it directly
        if !remote_branches.contains(&base_name) {
            return Ok(base_name);
        }

        // Append incrementing suffix until unique
        let mut suffix = 2u32;
        loop {
            let candidate = format!("{}-{}", base_name, suffix);
            if !remote_branches.contains(&candidate) {
                return Ok(candidate);
            }
            suffix += 1;
            // Safety valve to prevent infinite loop
            if suffix > 10000 {
                return Err(OrchestratorError::Validation(
                    "Could not find a unique branch name after 10000 attempts".to_string(),
                ));
            }
        }
    }

    /// List remote branch names (without the `origin/` prefix) from the mirror.
    ///
    /// Uses `git branch -r` to list remote-tracking branches, then strips the
    /// `origin/` prefix from each.
    async fn list_remote_branch_names(
        &self,
        mirror_path: &Path,
    ) -> Result<Vec<String>, OrchestratorError> {
        let output = self
            .git
            .exec(mirror_path, &["branch", "-r"], None, false)
            .await;

        match output {
            Ok(git_output) => {
                let branches = git_output
                    .stdout
                    .lines()
                    .map(|line| line.trim())
                    .filter(|line| !line.is_empty() && !line.contains("->"))
                    .map(|line| {
                        // Strip "origin/" prefix if present
                        line.strip_prefix("origin/")
                            .unwrap_or(line)
                            .to_string()
                    })
                    .collect();
                Ok(branches)
            }
            Err(_) => {
                // If we can't list branches (e.g., no remote configured), return empty
                Ok(vec![])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_mirrors_dir_is_absolute() {
        let dir = RepoSyncManager::default_mirrors_dir();
        assert!(
            dir.is_absolute(),
            "default_mirrors_dir should return an absolute path, got: {:?}",
            dir
        );
    }

    #[test]
    fn test_default_mirrors_dir_contains_beachead() {
        let dir = RepoSyncManager::default_mirrors_dir();
        let path_str = dir.to_string_lossy();
        assert!(
            path_str.contains("beachead"),
            "default_mirrors_dir should contain 'beachead', got: {:?}",
            dir
        );
    }

    #[test]
    fn test_default_mirrors_dir_ends_with_mirrors() {
        let dir = RepoSyncManager::default_mirrors_dir();
        let path_str = dir.to_string_lossy();
        assert!(
            path_str.ends_with("mirrors"),
            "default_mirrors_dir should end with 'mirrors', got: {:?}",
            dir
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn test_default_mirrors_dir_linux() {
        let dir = RepoSyncManager::default_mirrors_dir();
        let path_str = dir.to_string_lossy();
        // On Linux, should use data_local_dir (~/.local/share)
        assert!(
            path_str.contains(".local/share") || path_str.contains("/tmp"),
            "Linux default should use data_local_dir, got: {:?}",
            dir
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn test_default_mirrors_dir_macos() {
        let dir = RepoSyncManager::default_mirrors_dir();
        let path_str = dir.to_string_lossy();
        // On macOS, should use data_dir (~/Library/Application Support)
        assert!(
            path_str.contains("Library/Application Support") || path_str.contains("/tmp"),
            "macOS default should use data_dir, got: {:?}",
            dir
        );
    }

    #[tokio::test]
    async fn test_resolve_branch_name_basic_substitution() {
        // Create an in-memory DB with a persona
        let db = Arc::new(Database::open_in_memory().unwrap());
        db.with_conn(|conn| {
            conn.execute_batch(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');
                 INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('p1', 'my-agent', 'a1', '/tmp/workspace', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');",
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        }).unwrap();

        // Create a temp git repo to act as mirror
        let mirror_dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::fs::write(mirror_dir.path().join("README.md"), "# Test").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();

        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(
            db,
            git,
            PathBuf::from("/tmp/mirrors"),
        );

        let repo = ManagedRepo {
            id: crate::types::ManagedRepoId("r1".to_string()),
            persona_id: crate::types::PersonaId("p1".to_string()),
            workspace_path: "/tmp/workspace".to_string(),
            mirror_path: mirror_dir.path().to_string_lossy().to_string(),
            remote_url: None,
            remote_provider: None,
            branch_strategy: crate::types::BranchStrategy::FeatureBranch,
            branch_pattern: Some("ai/<persona-name>/<date>".to_string()),
            attribution_mode: crate::types::AttributionMode::KeepAgent,
            sync_mode: crate::types::SyncMode::Remote,
            secret_scan_mode: crate::types::SecretScanMode::Block,
            check_interval_seconds: 300,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let branch = manager
            .resolve_branch_name(&repo, "ai/<persona-name>/<date>")
            .await
            .unwrap();

        let today = Utc::now().format("%Y-%m-%d").to_string();
        assert_eq!(branch, format!("ai/my-agent/{}", today));
    }

    #[tokio::test]
    async fn test_resolve_branch_name_no_placeholders() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        db.with_conn(|conn| {
            conn.execute_batch(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');
                 INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('p1', 'my-agent', 'a1', '/tmp/workspace', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');",
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        }).unwrap();

        let mirror_dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::fs::write(mirror_dir.path().join("README.md"), "# Test").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();

        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(
            db,
            git,
            PathBuf::from("/tmp/mirrors"),
        );

        let repo = ManagedRepo {
            id: crate::types::ManagedRepoId("r1".to_string()),
            persona_id: crate::types::PersonaId("p1".to_string()),
            workspace_path: "/tmp/workspace".to_string(),
            mirror_path: mirror_dir.path().to_string_lossy().to_string(),
            remote_url: None,
            remote_provider: None,
            branch_strategy: crate::types::BranchStrategy::FeatureBranch,
            branch_pattern: Some("feature/custom-branch".to_string()),
            attribution_mode: crate::types::AttributionMode::KeepAgent,
            sync_mode: crate::types::SyncMode::Remote,
            secret_scan_mode: crate::types::SecretScanMode::Block,
            check_interval_seconds: 300,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let branch = manager
            .resolve_branch_name(&repo, "feature/custom-branch")
            .await
            .unwrap();

        assert_eq!(branch, "feature/custom-branch");
    }
}
