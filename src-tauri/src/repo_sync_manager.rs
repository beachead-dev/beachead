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
use crate::git_cli::{GitCli, GitError};
use crate::secret_scanner::SecretScanner;
use crate::types::{
    AttributionMode, BranchStrategy, ManagedRepo, ManagedRepoId, PersonaId, RemoteProvider,
    SecretScanMode, SyncMode, SyncResult,
};

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

    /// Validate that a URL is a valid git remote URL format.
    ///
    /// Accepts:
    /// - HTTPS: `https://host/path` (must start with `https://`)
    /// - SSH: `git@host:path` (must match `git@<host>:<path>`)
    ///
    /// Returns `Ok(())` if valid, or an error describing the format requirement.
    pub fn validate_remote_url(url: &str) -> Result<(), OrchestratorError> {
        if url.is_empty() {
            return Err(OrchestratorError::Validation(
                "Remote URL cannot be empty".to_string(),
            ));
        }
        if url.len() > 2048 {
            return Err(OrchestratorError::Validation(
                "Remote URL exceeds maximum length of 2048 characters".to_string(),
            ));
        }

        let is_https = url.starts_with("https://") && url.len() > "https://".len();
        let is_ssh = {
            // SSH format: git@host:path (e.g., git@github.com:user/repo.git)
            url.starts_with("git@") && url.contains(':') && {
                let after_at = &url["git@".len()..];
                let colon_pos = after_at.find(':');
                matches!(colon_pos, Some(pos) if pos > 0 && pos < after_at.len() - 1)
            }
        };

        if !is_https && !is_ssh {
            return Err(OrchestratorError::Validation(
                "Invalid remote URL format. Must be HTTPS (https://host/path) or SSH (git@host:path)".to_string(),
            ));
        }

        Ok(())
    }

    /// Detect the remote provider from a URL.
    ///
    /// Returns `Some(RemoteProvider)` if the URL matches a known provider,
    /// or `None` for unrecognized hosts.
    fn detect_remote_provider(url: &str) -> Option<RemoteProvider> {
        let url_lower = url.to_lowercase();
        if url_lower.contains("github.com") {
            Some(RemoteProvider::Github)
        } else if url_lower.contains("gitlab.com") || url_lower.contains("gitlab.") {
            Some(RemoteProvider::Gitlab)
        } else if url_lower.contains("bitbucket.org") || url_lower.contains("bitbucket.") {
            Some(RemoteProvider::Bitbucket)
        } else {
            None
        }
    }

    /// Enable repo sync for an existing repository that has remotes configured.
    ///
    /// This is the primary enable flow for repos the user already has cloned with
    /// remotes. It:
    /// 1. Reads remotes from the workspace
    /// 2. Computes the mirror path
    /// 3. Clones the workspace to the mirror (preserving remotes)
    /// 4. Strips all remotes from the workspace
    /// 5. Stores a ManagedRepo record with defaults
    ///
    /// On clone failure: deletes partial mirror dir, does NOT strip remotes.
    /// Rejects if mirror path already exists.
    ///
    /// # Arguments
    /// - `persona_id`: The persona that owns this workspace.
    /// - `workspace_path`: Path to the workspace git repository.
    ///
    /// # Returns
    /// The created `ManagedRepo` record.
    pub async fn enable(
        &self,
        persona_id: &PersonaId,
        workspace_path: &Path,
    ) -> Result<ManagedRepo, OrchestratorError> {
        // 1. Look up persona name from DB
        let persona_name = self.db.with_conn(|conn| {
            let persona = db_ops::get_persona(conn, persona_id)?;
            Ok(persona.name)
        })?;

        // 2. Read remotes from workspace
        let remote_names = self
            .git
            .list_remote_names(workspace_path)
            .await
            .map_err(|e| {
                OrchestratorError::Internal(format!("Failed to read remotes from workspace: {}", e))
            })?;

        if remote_names.is_empty() {
            return Err(OrchestratorError::Validation(
                "Workspace has no remotes configured. Use 'Link to remote' or 'Keep local only' instead.".to_string(),
            ));
        }

        // 3. Parse the origin URL (or first remote if no origin)
        let primary_remote = if remote_names.contains(&"origin".to_string()) {
            "origin"
        } else {
            &remote_names[0]
        };

        let remote_url = self
            .git
            .get_remote_url(workspace_path, primary_remote)
            .await
            .map_err(|e| {
                OrchestratorError::Internal(format!("Failed to get remote URL: {}", e))
            })?;

        // 4. Compute mirror path: <mirrors_dir>/<persona_name>/<project_folder_name>/
        let project_name = workspace_path
            .file_name()
            .ok_or_else(|| {
                OrchestratorError::Validation("Workspace path has no folder name".to_string())
            })?
            .to_string_lossy()
            .to_string();
        let mirror_path = self.mirrors_dir.join(&persona_name).join(&project_name);

        // 5. Check if mirror path already exists → error
        if mirror_path.exists() {
            return Err(OrchestratorError::Validation(
                "Mirror directory already exists".to_string(),
            ));
        }

        // 6. Clone workspace to mirror (preserves remotes)
        // Ensure parent directory exists
        if let Some(parent) = mirror_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                OrchestratorError::Internal(format!(
                    "Failed to create mirror parent directory: {}",
                    e
                ))
            })?;
        }

        let workspace_str = workspace_path.to_string_lossy().to_string();
        let mirror_str = mirror_path.to_string_lossy().to_string();

        let clone_result = self
            .git
            .exec_in_dir(
                mirror_path.parent().unwrap(),
                &["clone", &workspace_str, &mirror_str],
                false,
            )
            .await;

        // On clone failure: delete partial mirror dir, don't strip remotes
        if let Err(e) = clone_result {
            if mirror_path.exists() {
                let _ = std::fs::remove_dir_all(&mirror_path);
            }
            return Err(OrchestratorError::Internal(format!(
                "Failed to clone workspace to mirror: {}",
                e
            )));
        }

        // 7. Set the mirror's origin to point to the actual remote (not the workspace)
        // The clone creates an origin pointing to the workspace path; replace it with
        // the real remote URL so the mirror can push/fetch from the actual remote.
        if let Some(ref url) = remote_url {
            let _ = self
                .git
                .exec(&mirror_path, &["remote", "set-url", "origin", url], None, false)
                .await;
        }

        // Also copy any other remotes from the workspace to the mirror
        // (clone only copies origin; we need all remotes preserved)
        for name in &remote_names {
            if name == primary_remote {
                continue; // Already handled above
            }
            if let Ok(Some(url)) = self.git.get_remote_url(workspace_path, name).await {
                // Add the remote to the mirror if it doesn't already exist
                let _ = self
                    .git
                    .exec(&mirror_path, &["remote", "add", name, &url], None, false)
                    .await;
            }
        }

        // 8. Strip all remotes from workspace
        for name in &remote_names {
            let _ = self
                .git
                .exec(workspace_path, &["remote", "remove", name], None, false)
                .await;
        }

        // 9. Detect remote provider from URL
        let remote_provider = remote_url.as_deref().and_then(Self::detect_remote_provider);

        // 10. Store ManagedRepo record with defaults
        let now = Utc::now();
        let repo = ManagedRepo {
            id: ManagedRepoId::new(),
            persona_id: persona_id.clone(),
            workspace_path: workspace_str,
            mirror_path: mirror_str,
            remote_url,
            remote_provider,
            branch_strategy: BranchStrategy::Direct,
            branch_pattern: Some("ai/<persona-name>/<date>".to_string()),
            attribution_mode: AttributionMode::KeepAgent,
            sync_mode: SyncMode::Remote,
            secret_scan_mode: SecretScanMode::Block,
            check_interval_seconds: 300,
            created_at: now,
            updated_at: now,
        };

        self.db.with_conn(|conn| {
            db_ops::insert_managed_repo(conn, &repo)
        })?;

        Ok(repo)
    }

    /// Enable repo sync for an agent-created repository (no existing remotes).
    ///
    /// This handles two cases:
    /// - "Link to remote": user provides a remote URL → mirror gets origin, sync_mode=Remote
    /// - "Keep local only": no remote URL → mirror has no remote, sync_mode=LocalOnly
    ///
    /// In both cases, all remotes are stripped from the workspace.
    ///
    /// # Arguments
    /// - `persona_id`: The persona that owns this workspace.
    /// - `workspace_path`: Path to the workspace git repository.
    /// - `remote_url`: Optional remote URL to add as origin to the mirror.
    ///
    /// # Returns
    /// The created `ManagedRepo` record.
    pub async fn enable_agent_created(
        &self,
        persona_id: &PersonaId,
        workspace_path: &Path,
        remote_url: Option<&str>,
    ) -> Result<ManagedRepo, OrchestratorError> {
        // 1. Validate remote URL format if provided
        if let Some(url) = remote_url {
            Self::validate_remote_url(url)?;
        }

        // 2. Look up persona name from DB
        let persona_name = self.db.with_conn(|conn| {
            let persona = db_ops::get_persona(conn, persona_id)?;
            Ok(persona.name)
        })?;

        // 3. Compute mirror path: <mirrors_dir>/<persona_name>/<project_folder_name>/
        let project_name = workspace_path
            .file_name()
            .ok_or_else(|| {
                OrchestratorError::Validation(
                    "Workspace path has no folder name".to_string(),
                )
            })?
            .to_string_lossy()
            .to_string();
        let mirror_path = self.mirrors_dir.join(&persona_name).join(&project_name);

        // 4. Check if mirror path already exists → error
        if mirror_path.exists() {
            return Err(OrchestratorError::Validation(
                "Mirror directory already exists".to_string(),
            ));
        }

        // 5. Clone workspace to mirror
        // Ensure parent directory exists
        if let Some(parent) = mirror_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                OrchestratorError::Internal(format!(
                    "Failed to create mirror parent directory: {}",
                    e
                ))
            })?;
        }

        let workspace_str = workspace_path.to_string_lossy().to_string();
        let mirror_str = mirror_path.to_string_lossy().to_string();

        let clone_result = self
            .git
            .exec_in_dir(
                mirror_path.parent().unwrap(),
                &["clone", &workspace_str, &mirror_str],
                false,
            )
            .await;

        // On clone failure: delete partial mirror dir, don't strip remotes, don't store record
        if let Err(e) = clone_result {
            // Clean up partial mirror directory if it was created
            if mirror_path.exists() {
                let _ = std::fs::remove_dir_all(&mirror_path);
            }
            return Err(OrchestratorError::Internal(format!(
                "Failed to clone workspace to mirror: {}",
                e
            )));
        }

        // 6. If remote URL provided: add it as origin to the mirror
        //    The clone creates a remote pointing to the workspace; we need to replace it.
        if let Some(url) = remote_url {
            // Remove the workspace-pointing origin that clone created
            let _ = self
                .git
                .exec(&mirror_path, &["remote", "remove", "origin"], None, false)
                .await;
            // Add the user-provided remote URL as origin
            let add_result = self
                .git
                .exec(
                    &mirror_path,
                    &["remote", "add", "origin", url],
                    None,
                    false,
                )
                .await;
            if let Err(e) = add_result {
                // Clean up on failure
                let _ = std::fs::remove_dir_all(&mirror_path);
                return Err(OrchestratorError::Internal(format!(
                    "Failed to add remote origin to mirror: {}",
                    e
                )));
            }
        } else {
            // No remote URL: remove the workspace-pointing origin that clone created
            let _ = self
                .git
                .exec(&mirror_path, &["remote", "remove", "origin"], None, false)
                .await;
        }

        // 7. Strip all remotes from workspace
        let remote_names = self.git.list_remote_names(workspace_path).await.unwrap_or_default();
        for name in &remote_names {
            let _ = self
                .git
                .exec(workspace_path, &["remote", "remove", name], None, false)
                .await;
        }

        // 8. Determine sync mode and store record
        let sync_mode = if remote_url.is_some() {
            SyncMode::Remote
        } else {
            SyncMode::LocalOnly
        };

        let now = Utc::now();
        let repo = ManagedRepo {
            id: ManagedRepoId::new(),
            persona_id: persona_id.clone(),
            workspace_path: workspace_str,
            mirror_path: mirror_str,
            remote_url: remote_url.map(|s| s.to_string()),
            remote_provider: None,
            branch_strategy: BranchStrategy::Direct,
            branch_pattern: Some("ai/<persona-name>/<date>".to_string()),
            attribution_mode: AttributionMode::KeepAgent,
            sync_mode,
            secret_scan_mode: SecretScanMode::Block,
            check_interval_seconds: 300,
            created_at: now,
            updated_at: now,
        };

        self.db.with_conn(|conn| {
            db_ops::insert_managed_repo(conn, &repo)
        })?;

        Ok(repo)
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

    /// Pull commits from the agent's workspace into the mirror.
    ///
    /// Fetches from the workspace path into the mirror, then attempts a fast-forward
    /// merge. If fast-forward is not possible, falls back to a regular merge commit.
    ///
    /// # Returns
    /// - `Ok(SyncResult { commits })` with the number of new commits pulled.
    /// - `Err(OrchestratorError::Validation(...))` if a merge conflict occurs,
    ///   including the conflicting file paths.
    pub async fn pull_from_agent(&self, repo_id: &ManagedRepoId) -> Result<SyncResult, OrchestratorError> {
        let repo = self.db.with_conn(|conn| {
            db_ops::get_managed_repo(conn, repo_id)
        })?;

        let mirror = Path::new(&repo.mirror_path);
        let workspace = Path::new(&repo.workspace_path);

        let workspace_str = workspace.to_str().ok_or_else(|| {
            OrchestratorError::Validation("Workspace path contains invalid UTF-8".to_string())
        })?;

        // Record the HEAD commit before merge so we can count new commits afterward
        let head_before = self
            .git
            .exec(mirror, &["rev-parse", "HEAD"], None, false)
            .await
            .map(|o| o.stdout.trim().to_string())
            .unwrap_or_default();

        // Fetch from workspace into mirror
        self.git
            .exec(mirror, &["fetch", workspace_str], None, false)
            .await
            .map_err(|e| OrchestratorError::Internal(format!("Failed to fetch from workspace: {}", e)))?;

        // Get current branch in mirror
        let branch = self.git.get_current_branch(mirror).await.map_err(|e| {
            OrchestratorError::Internal(format!("Failed to get current branch: {}", e))
        })?;

        if branch.is_empty() {
            return Err(OrchestratorError::Validation(
                "Mirror is in detached HEAD state; cannot merge".to_string(),
            ));
        }

        // Attempt fast-forward merge
        let merge_result = self
            .git
            .exec(mirror, &["merge", "--ff-only", "FETCH_HEAD"], None, false)
            .await;

        match merge_result {
            Ok(_) => { /* fast-forward succeeded */ }
            Err(GitError::NonZeroExit { .. }) => {
                // Fast-forward not possible, fall back to regular merge
                let regular_merge = self
                    .git
                    .exec(mirror, &["merge", "FETCH_HEAD"], None, false)
                    .await;

                if let Err(e) = regular_merge {
                    // Merge conflict or other failure — get conflicting file paths
                    let conflict_files = self.get_conflict_files(mirror).await;
                    let file_list = if conflict_files.is_empty() {
                        e.to_string()
                    } else {
                        format!(
                            "Merge conflict in files: {}",
                            conflict_files.join(", ")
                        )
                    };
                    return Err(OrchestratorError::Validation(file_list));
                }
            }
            Err(GitError::MergeConflict { stderr }) => {
                // The ff-only itself reported a conflict (unlikely but handle it)
                let conflict_files = self.get_conflict_files(mirror).await;
                let file_list = if conflict_files.is_empty() {
                    format!("Merge conflict: {}", stderr)
                } else {
                    format!(
                        "Merge conflict in files: {}",
                        conflict_files.join(", ")
                    )
                };
                return Err(OrchestratorError::Validation(file_list));
            }
            Err(e) => {
                return Err(OrchestratorError::Internal(format!(
                    "Merge failed: {}",
                    e
                )));
            }
        }

        // Count commits pulled by comparing HEAD before and after
        let commits = if head_before.is_empty() {
            0
        } else {
            let count_output = self
                .git
                .exec(
                    mirror,
                    &["rev-list", "--count", &format!("{}..HEAD", head_before)],
                    None,
                    false,
                )
                .await;
            match count_output {
                Ok(output) => output.stdout.trim().parse::<u32>().unwrap_or(0),
                Err(_) => 0,
            }
        };

        Ok(SyncResult { commits })
    }

    /// Get the list of files with merge conflicts in the given repo.
    async fn get_conflict_files(&self, repo_path: &Path) -> Vec<String> {
        // Use `git diff --name-only --diff-filter=U` to list unmerged (conflicting) files
        let result = self
            .git
            .exec(
                repo_path,
                &["diff", "--name-only", "--diff-filter=U"],
                None,
                false,
            )
            .await;

        match result {
            Ok(output) => output
                .stdout
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect(),
            Err(_) => vec![],
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

    // --- Tests for validate_remote_url ---

    #[test]
    fn test_validate_remote_url_https_valid() {
        assert!(RepoSyncManager::validate_remote_url("https://github.com/user/repo.git").is_ok());
        assert!(RepoSyncManager::validate_remote_url("https://gitlab.com/org/project").is_ok());
    }

    #[test]
    fn test_validate_remote_url_ssh_valid() {
        assert!(RepoSyncManager::validate_remote_url("git@github.com:user/repo.git").is_ok());
        assert!(RepoSyncManager::validate_remote_url("git@gitlab.com:org/project.git").is_ok());
    }

    #[test]
    fn test_validate_remote_url_empty() {
        let result = RepoSyncManager::validate_remote_url("");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_remote_url_http_rejected() {
        // Only https is accepted, not http
        let result = RepoSyncManager::validate_remote_url("http://github.com/user/repo.git");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_remote_url_invalid_format() {
        assert!(RepoSyncManager::validate_remote_url("not-a-url").is_err());
        assert!(RepoSyncManager::validate_remote_url("ftp://example.com/repo").is_err());
        assert!(RepoSyncManager::validate_remote_url("https://").is_err());
    }

    #[test]
    fn test_validate_remote_url_ssh_incomplete() {
        // Missing path after colon
        assert!(RepoSyncManager::validate_remote_url("git@github.com:").is_err());
        // Missing host
        assert!(RepoSyncManager::validate_remote_url("git@:path").is_err());
    }

    #[test]
    fn test_validate_remote_url_too_long() {
        let long_url = format!("https://github.com/{}", "a".repeat(2048));
        assert!(RepoSyncManager::validate_remote_url(&long_url).is_err());
    }

    // --- Tests for enable_agent_created ---

    /// Helper to create a temp git repo with an initial commit.
    fn create_test_workspace() -> tempfile::TempDir {
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
        std::fs::write(dir.path().join("README.md"), "# Test Project").unwrap();
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

    /// Helper to set up DB with a persona.
    fn setup_db_with_persona(persona_id: &str, persona_name: &str) -> Arc<Database> {
        let db = Arc::new(Database::open_in_memory().unwrap());
        db.with_conn(|conn| {
            conn.execute_batch(&format!(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{{}}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');
                 INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('{}', '{}', 'a1', '/tmp/workspace', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');",
                persona_id, persona_name
            )).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        }).unwrap();
        db
    }

    #[tokio::test]
    async fn test_enable_agent_created_local_only() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git.clone(), mirrors_dir.path().to_path_buf());

        let result = manager
            .enable_agent_created(
                &PersonaId("p1".to_string()),
                workspace.path(),
                None,
            )
            .await;

        assert!(result.is_ok(), "enable_agent_created failed: {:?}", result.err());
        let repo = result.unwrap();

        // Verify sync_mode is LocalOnly
        assert_eq!(repo.sync_mode, SyncMode::LocalOnly);
        assert!(repo.remote_url.is_none());

        // Verify mirror was created
        let mirror_path = Path::new(&repo.mirror_path);
        assert!(mirror_path.exists());
        assert!(mirror_path.join(".git").exists());

        // Verify workspace has no remotes
        let remotes = git.list_remote_names(workspace.path()).await.unwrap();
        assert!(remotes.is_empty(), "Workspace should have no remotes");

        // Verify mirror has no remotes (origin removed)
        let mirror_remotes = git.list_remote_names(mirror_path).await.unwrap();
        assert!(mirror_remotes.is_empty(), "Mirror should have no remotes for local-only");
    }

    #[tokio::test]
    async fn test_enable_agent_created_with_remote_url() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git.clone(), mirrors_dir.path().to_path_buf());

        let remote_url = "https://github.com/user/repo.git";
        let result = manager
            .enable_agent_created(
                &PersonaId("p1".to_string()),
                workspace.path(),
                Some(remote_url),
            )
            .await;

        assert!(result.is_ok(), "enable_agent_created failed: {:?}", result.err());
        let repo = result.unwrap();

        // Verify sync_mode is Remote
        assert_eq!(repo.sync_mode, SyncMode::Remote);
        assert_eq!(repo.remote_url.as_deref(), Some(remote_url));

        // Verify mirror has origin pointing to the provided URL
        let mirror_path = Path::new(&repo.mirror_path);
        let mirror_origin = git.get_remote_url(mirror_path, "origin").await.unwrap();
        assert_eq!(mirror_origin.as_deref(), Some(remote_url));

        // Verify workspace has no remotes
        let remotes = git.list_remote_names(workspace.path()).await.unwrap();
        assert!(remotes.is_empty(), "Workspace should have no remotes");
    }

    #[tokio::test]
    async fn test_enable_agent_created_invalid_url_rejected() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git.clone(), mirrors_dir.path().to_path_buf());

        let result = manager
            .enable_agent_created(
                &PersonaId("p1".to_string()),
                workspace.path(),
                Some("not-a-valid-url"),
            )
            .await;

        assert!(result.is_err());
        // Verify no mirror was created
        let mirror_path = mirrors_dir.path().join("my-agent");
        assert!(!mirror_path.exists(), "No mirror should be created on invalid URL");
    }

    #[tokio::test]
    async fn test_enable_agent_created_mirror_already_exists() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git.clone(), mirrors_dir.path().to_path_buf());

        // Pre-create the mirror directory
        let project_name = workspace.path().file_name().unwrap();
        let mirror_path = mirrors_dir.path().join("my-agent").join(project_name);
        std::fs::create_dir_all(&mirror_path).unwrap();

        let result = manager
            .enable_agent_created(
                &PersonaId("p1".to_string()),
                workspace.path(),
                None,
            )
            .await;

        assert!(result.is_err());
        let err_msg = format!("{:?}", result.err().unwrap());
        assert!(err_msg.contains("already exists"), "Error should mention mirror already exists: {}", err_msg);
    }

    #[tokio::test]
    async fn test_enable_agent_created_strips_existing_workspace_remotes() {
        let workspace = create_test_workspace();
        // Add a remote to the workspace (simulating agent adding one)
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://example.com/repo.git"])
            .current_dir(workspace.path())
            .output()
            .unwrap();

        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git.clone(), mirrors_dir.path().to_path_buf());

        let result = manager
            .enable_agent_created(
                &PersonaId("p1".to_string()),
                workspace.path(),
                None,
            )
            .await;

        assert!(result.is_ok());

        // Verify workspace remotes were stripped
        let remotes = git.list_remote_names(workspace.path()).await.unwrap();
        assert!(remotes.is_empty(), "All workspace remotes should be stripped");
    }

    #[tokio::test]
    async fn test_enable_agent_created_db_record_stored() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let result = manager
            .enable_agent_created(
                &PersonaId("p1".to_string()),
                workspace.path(),
                Some("git@github.com:user/repo.git"),
            )
            .await
            .unwrap();

        // Verify the record was stored in DB
        let stored = db.with_conn(|conn| {
            db_ops::get_managed_repo(conn, &result.id)
        }).unwrap();

        assert_eq!(stored.persona_id.0, "p1");
        assert_eq!(stored.sync_mode, SyncMode::Remote);
        assert_eq!(stored.remote_url.as_deref(), Some("git@github.com:user/repo.git"));
        assert_eq!(stored.branch_strategy, BranchStrategy::Direct);
        assert_eq!(stored.attribution_mode, AttributionMode::KeepAgent);
        assert_eq!(stored.secret_scan_mode, SecretScanMode::Block);
        assert_eq!(stored.check_interval_seconds, 300);
    }

    // --- Tests for enable (existing repo with remotes) ---

    #[tokio::test]
    async fn test_enable_clones_workspace_to_mirror() {
        let workspace = create_test_workspace();
        // Add a remote to the workspace
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://github.com/user/repo.git"])
            .current_dir(workspace.path())
            .output()
            .unwrap();

        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git.clone(), mirrors_dir.path().to_path_buf());

        let result = manager
            .enable(&PersonaId("p1".to_string()), workspace.path())
            .await;

        assert!(result.is_ok(), "enable failed: {:?}", result.err());
        let repo = result.unwrap();

        // Verify mirror was created and is a valid git repo
        let mirror_path = Path::new(&repo.mirror_path);
        assert!(mirror_path.exists());
        assert!(mirror_path.join(".git").exists());

        // Verify mirror has the README from the workspace
        assert!(mirror_path.join("README.md").exists());
    }

    #[tokio::test]
    async fn test_enable_strips_remotes_from_workspace() {
        let workspace = create_test_workspace();
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://github.com/user/repo.git"])
            .current_dir(workspace.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["remote", "add", "upstream", "https://github.com/org/repo.git"])
            .current_dir(workspace.path())
            .output()
            .unwrap();

        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git.clone(), mirrors_dir.path().to_path_buf());

        let result = manager
            .enable(&PersonaId("p1".to_string()), workspace.path())
            .await;

        assert!(result.is_ok());

        // Verify all remotes stripped from workspace
        let remotes = git.list_remote_names(workspace.path()).await.unwrap();
        assert!(remotes.is_empty(), "Workspace should have no remotes after enable");
    }

    #[tokio::test]
    async fn test_enable_preserves_remotes_in_mirror() {
        let workspace = create_test_workspace();
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://github.com/user/repo.git"])
            .current_dir(workspace.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["remote", "add", "upstream", "https://github.com/org/repo.git"])
            .current_dir(workspace.path())
            .output()
            .unwrap();

        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable(&PersonaId("p1".to_string()), workspace.path())
            .await
            .unwrap();

        // Verify mirror has origin pointing to the real remote URL
        let mirror_path = Path::new(&repo.mirror_path);
        let origin_url = git.get_remote_url(mirror_path, "origin").await.unwrap();
        assert_eq!(origin_url.as_deref(), Some("https://github.com/user/repo.git"));

        // Verify mirror also has the upstream remote
        let upstream_url = git.get_remote_url(mirror_path, "upstream").await.unwrap();
        assert_eq!(upstream_url.as_deref(), Some("https://github.com/org/repo.git"));
    }

    #[tokio::test]
    async fn test_enable_uses_origin_as_primary_remote() {
        let workspace = create_test_workspace();
        // Add remotes in non-origin-first order
        std::process::Command::new("git")
            .args(["remote", "add", "upstream", "https://github.com/org/repo.git"])
            .current_dir(workspace.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://github.com/user/repo.git"])
            .current_dir(workspace.path())
            .output()
            .unwrap();

        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable(&PersonaId("p1".to_string()), workspace.path())
            .await
            .unwrap();

        // The primary remote_url stored should be origin's URL
        assert_eq!(repo.remote_url.as_deref(), Some("https://github.com/user/repo.git"));
    }

    #[tokio::test]
    async fn test_enable_uses_first_remote_when_no_origin() {
        let workspace = create_test_workspace();
        // Add a remote that's not named "origin"
        std::process::Command::new("git")
            .args(["remote", "add", "upstream", "https://github.com/org/repo.git"])
            .current_dir(workspace.path())
            .output()
            .unwrap();

        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable(&PersonaId("p1".to_string()), workspace.path())
            .await
            .unwrap();

        // Should use the first (and only) remote's URL
        assert_eq!(repo.remote_url.as_deref(), Some("https://github.com/org/repo.git"));
    }

    #[tokio::test]
    async fn test_enable_rejects_if_mirror_exists() {
        let workspace = create_test_workspace();
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://github.com/user/repo.git"])
            .current_dir(workspace.path())
            .output()
            .unwrap();

        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git.clone(), mirrors_dir.path().to_path_buf());

        // Pre-create the mirror directory
        let project_name = workspace.path().file_name().unwrap();
        let mirror_path = mirrors_dir.path().join("my-agent").join(project_name);
        std::fs::create_dir_all(&mirror_path).unwrap();

        let result = manager
            .enable(&PersonaId("p1".to_string()), workspace.path())
            .await;

        assert!(result.is_err());
        let err_msg = format!("{:?}", result.err().unwrap());
        assert!(err_msg.contains("already exists"), "Error should mention mirror already exists: {}", err_msg);

        // Verify workspace remotes were NOT stripped (rollback behavior)
        let remotes = git.list_remote_names(workspace.path()).await.unwrap();
        assert!(remotes.contains(&"origin".to_string()), "Workspace remotes should be preserved on error");
    }

    #[tokio::test]
    async fn test_enable_rejects_workspace_without_remotes() {
        let workspace = create_test_workspace();
        // No remotes added

        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git.clone(), mirrors_dir.path().to_path_buf());

        let result = manager
            .enable(&PersonaId("p1".to_string()), workspace.path())
            .await;

        assert!(result.is_err());
        let err_msg = format!("{:?}", result.err().unwrap());
        assert!(err_msg.contains("no remotes"), "Error should mention no remotes: {}", err_msg);
    }

    #[tokio::test]
    async fn test_enable_stores_correct_defaults() {
        let workspace = create_test_workspace();
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://github.com/user/repo.git"])
            .current_dir(workspace.path())
            .output()
            .unwrap();

        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable(&PersonaId("p1".to_string()), workspace.path())
            .await
            .unwrap();

        // Verify defaults per requirement 5.5
        assert_eq!(repo.branch_strategy, BranchStrategy::Direct);
        assert_eq!(repo.attribution_mode, AttributionMode::KeepAgent);
        assert_eq!(repo.sync_mode, SyncMode::Remote);
        assert_eq!(repo.secret_scan_mode, SecretScanMode::Block);
        assert_eq!(repo.check_interval_seconds, 300);
        assert_eq!(repo.branch_pattern.as_deref(), Some("ai/<persona-name>/<date>"));

        // Verify DB record was stored
        let stored = db.with_conn(|conn| {
            db_ops::get_managed_repo(conn, &repo.id)
        }).unwrap();
        assert_eq!(stored.persona_id.0, "p1");
        assert_eq!(stored.workspace_path, workspace.path().to_string_lossy().to_string());
    }

    #[tokio::test]
    async fn test_enable_detects_github_provider() {
        let workspace = create_test_workspace();
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://github.com/user/repo.git"])
            .current_dir(workspace.path())
            .output()
            .unwrap();

        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable(&PersonaId("p1".to_string()), workspace.path())
            .await
            .unwrap();

        assert_eq!(repo.remote_provider, Some(RemoteProvider::Github));
    }

    #[test]
    fn test_detect_remote_provider_github() {
        assert_eq!(
            RepoSyncManager::detect_remote_provider("https://github.com/user/repo.git"),
            Some(RemoteProvider::Github)
        );
        assert_eq!(
            RepoSyncManager::detect_remote_provider("git@github.com:user/repo.git"),
            Some(RemoteProvider::Github)
        );
    }

    #[test]
    fn test_detect_remote_provider_gitlab() {
        assert_eq!(
            RepoSyncManager::detect_remote_provider("https://gitlab.com/org/project.git"),
            Some(RemoteProvider::Gitlab)
        );
    }

    #[test]
    fn test_detect_remote_provider_bitbucket() {
        assert_eq!(
            RepoSyncManager::detect_remote_provider("https://bitbucket.org/user/repo.git"),
            Some(RemoteProvider::Bitbucket)
        );
    }

    #[test]
    fn test_detect_remote_provider_unknown() {
        assert_eq!(
            RepoSyncManager::detect_remote_provider("https://custom-git.example.com/repo.git"),
            None
        );
    }

    // --- pull_from_agent tests ---

    /// Helper to set up a workspace + mirror pair for pull_from_agent tests.
    /// Returns (manager, repo_id, workspace_dir, mirror_dir).
    async fn setup_pull_test() -> (
        RepoSyncManager,
        ManagedRepoId,
        tempfile::TempDir,
        tempfile::TempDir,
    ) {
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

        // Create workspace repo
        let workspace_dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(workspace_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(workspace_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(workspace_dir.path())
            .output()
            .unwrap();
        std::fs::write(workspace_dir.path().join("README.md"), "# Test").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(workspace_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(workspace_dir.path())
            .output()
            .unwrap();

        // Clone workspace to create mirror
        let mirror_dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args([
                "clone",
                workspace_dir.path().to_str().unwrap(),
                mirror_dir.path().to_str().unwrap(),
            ])
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

        // Insert managed repo record
        let repo_id = ManagedRepoId::new();
        let now = Utc::now();
        let repo = ManagedRepo {
            id: repo_id.clone(),
            persona_id: PersonaId("p1".to_string()),
            workspace_path: workspace_dir.path().to_string_lossy().to_string(),
            mirror_path: mirror_dir.path().to_string_lossy().to_string(),
            remote_url: None,
            remote_provider: None,
            branch_strategy: BranchStrategy::Direct,
            branch_pattern: None,
            attribution_mode: AttributionMode::KeepAgent,
            sync_mode: SyncMode::LocalOnly,
            secret_scan_mode: SecretScanMode::Block,
            check_interval_seconds: 300,
            created_at: now,
            updated_at: now,
        };
        db.with_conn(|conn| db_ops::insert_managed_repo(conn, &repo)).unwrap();

        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, PathBuf::from("/tmp/mirrors"));

        (manager, repo_id, workspace_dir, mirror_dir)
    }

    #[tokio::test]
    async fn test_pull_from_agent_no_new_commits() {
        let (manager, repo_id, _workspace_dir, _mirror_dir) = setup_pull_test().await;

        let result = manager.pull_from_agent(&repo_id).await.unwrap();
        assert_eq!(result.commits, 0);
    }

    #[tokio::test]
    async fn test_pull_from_agent_fast_forward() {
        let (manager, repo_id, workspace_dir, _mirror_dir) = setup_pull_test().await;

        // Add commits to workspace
        std::fs::write(workspace_dir.path().join("file1.txt"), "content1").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(workspace_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "add file1"])
            .current_dir(workspace_dir.path())
            .output()
            .unwrap();

        std::fs::write(workspace_dir.path().join("file2.txt"), "content2").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(workspace_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "add file2"])
            .current_dir(workspace_dir.path())
            .output()
            .unwrap();

        let result = manager.pull_from_agent(&repo_id).await.unwrap();
        assert_eq!(result.commits, 2);
    }

    #[tokio::test]
    async fn test_pull_from_agent_merge_commit() {
        let (manager, repo_id, workspace_dir, mirror_dir) = setup_pull_test().await;

        // Add a commit to workspace on a different file
        std::fs::write(workspace_dir.path().join("workspace_file.txt"), "from workspace").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(workspace_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "workspace commit"])
            .current_dir(workspace_dir.path())
            .output()
            .unwrap();

        // Add a diverging commit to mirror on a different file (non-conflicting)
        std::fs::write(mirror_dir.path().join("mirror_file.txt"), "from mirror").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "mirror commit"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();

        // Pull should succeed with a merge commit (ff not possible)
        let result = manager.pull_from_agent(&repo_id).await.unwrap();
        // Should have at least 1 commit (the workspace commit + merge commit = 2)
        assert!(result.commits >= 1);
    }

    #[tokio::test]
    async fn test_pull_from_agent_merge_conflict() {
        let (manager, repo_id, workspace_dir, mirror_dir) = setup_pull_test().await;

        // Add a commit to workspace modifying README.md
        std::fs::write(workspace_dir.path().join("README.md"), "workspace version").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(workspace_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "workspace change"])
            .current_dir(workspace_dir.path())
            .output()
            .unwrap();

        // Add a conflicting commit to mirror modifying the same file
        std::fs::write(mirror_dir.path().join("README.md"), "mirror version").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "mirror change"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();

        // Pull should fail with a conflict error mentioning the file
        let err = manager.pull_from_agent(&repo_id).await.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("README.md") || err_msg.contains("conflict") || err_msg.contains("Merge"),
            "Expected conflict error mentioning file, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_pull_from_agent_repo_not_found() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        db.with_conn(|conn| {
            conn.execute_batch(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');",
            ).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        }).unwrap();

        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, PathBuf::from("/tmp/mirrors"));

        let result = manager
            .pull_from_agent(&ManagedRepoId("nonexistent".to_string()))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }
}
