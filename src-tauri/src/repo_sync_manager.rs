//! Repo Sync Manager: business logic for git remote synchronization.
//!
//! Manages the two-directory architecture where the agent works in a remote-free
//! workspace and a host-side mirror holds remotes and credentials. All sync
//! operations are user-initiated and run on the host via the git CLI.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::db::Database;
use crate::db_ops;
use crate::error::OrchestratorError;
use crate::git_cli::{GitCli, GitError};
use crate::repo_credential_manager;
use crate::secret_scanner::SecretScanner;
use crate::types::{
    AttributionMode, BranchStrategy, CommitInfo, DetectedRepo, ManagedRepo, ManagedRepoId,
    PersonaId, PushResult, RemoteProvider, SecretScanMode, SyncMode, SyncResult, SyncStatus,
    UpdateRepoRequest,
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
    /// Cached sync status for all repos, keyed by repo ID.
    /// Updated by the background checker, polled by the frontend.
    pub cached_status: Arc<DashMap<String, SyncStatus>>,
    /// Tracks the last time each repo was checked, keyed by repo ID.
    last_check_times: Arc<DashMap<String, Instant>>,
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
            cached_status: Arc::new(DashMap::new()),
            last_check_times: Arc::new(DashMap::new()),
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

    /// Push commits from the mirror into the agent's workspace.
    ///
    /// Executes `git pull <mirror-path> <branch>` in the workspace using local
    /// file paths only (no network access required). Checks for dirty working tree
    /// first and returns an error if uncommitted changes exist.
    ///
    /// # Returns
    /// - `Ok(SyncResult { commits })` with the number of commits applied.
    /// - `Err(OrchestratorError::Validation(...))` if the workspace has uncommitted
    ///   changes or a merge conflict occurs (includes conflicting file paths).
    pub async fn push_to_agent(&self, repo_id: &ManagedRepoId) -> Result<SyncResult, OrchestratorError> {
        let repo = self.db.with_conn(|conn| {
            db_ops::get_managed_repo(conn, repo_id)
        })?;

        let workspace = Path::new(&repo.workspace_path);
        let mirror = Path::new(&repo.mirror_path);

        let mirror_str = mirror.to_str().ok_or_else(|| {
            OrchestratorError::Validation("Mirror path contains invalid UTF-8".to_string())
        })?;

        // Check for dirty working tree
        let status = self
            .git
            .exec(workspace, &["status", "--porcelain"], None, false)
            .await
            .map_err(|e| {
                OrchestratorError::Internal(format!("Failed to check workspace status: {}", e))
            })?;

        if !status.stdout.trim().is_empty() {
            return Err(OrchestratorError::Validation(
                "Workspace has uncommitted changes".to_string(),
            ));
        }

        // Get current branch from mirror
        let branch = self.git.get_current_branch(mirror).await.map_err(|e| {
            OrchestratorError::Internal(format!("Failed to get current branch from mirror: {}", e))
        })?;

        if branch.is_empty() {
            return Err(OrchestratorError::Validation(
                "Mirror is in detached HEAD state; cannot pull".to_string(),
            ));
        }

        // Record HEAD before pull to count commits afterward
        let head_before = self
            .git
            .exec(workspace, &["rev-parse", "HEAD"], None, false)
            .await
            .map(|o| o.stdout.trim().to_string())
            .unwrap_or_default();

        // Execute git pull --no-rebase <mirror-path> <branch> in workspace (local, no network)
        let pull_result = self
            .git
            .exec(workspace, &["pull", "--no-rebase", mirror_str, &branch], None, false)
            .await;

        match pull_result {
            Ok(_) => { /* pull succeeded */ }
            Err(GitError::MergeConflict { stderr: _ }) => {
                let conflict_files = self.get_conflict_files(workspace).await;
                let file_list = if conflict_files.is_empty() {
                    "Merge conflict in workspace".to_string()
                } else {
                    format!(
                        "Merge conflict in files: {}",
                        conflict_files.join(", ")
                    )
                };
                return Err(OrchestratorError::Validation(file_list));
            }
            Err(GitError::NonZeroExit { stderr, .. }) => {
                // Check if this is a merge conflict — git pull puts CONFLICT in stdout,
                // not stderr, so classify_git_error won't catch it. Check for unmerged files.
                let conflict_files = self.get_conflict_files(workspace).await;
                if !conflict_files.is_empty() {
                    let file_list = format!(
                        "Merge conflict in files: {}",
                        conflict_files.join(", ")
                    );
                    return Err(OrchestratorError::Validation(file_list));
                }
                // Also check stderr for conflict indicators
                if stderr.contains("CONFLICT") || stderr.contains("Automatic merge failed") {
                    return Err(OrchestratorError::Validation(
                        "Merge conflict in workspace".to_string(),
                    ));
                }
                return Err(OrchestratorError::Internal(format!(
                    "Pull failed: {}",
                    stderr
                )));
            }
            Err(e) => {
                return Err(OrchestratorError::Internal(format!(
                    "Pull failed: {}",
                    e
                )));
            }
        }

        // Count commits applied by comparing HEAD before and after
        let commits = if head_before.is_empty() {
            0
        } else {
            let count_output = self
                .git
                .exec(
                    workspace,
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

    /// Push selected commits from the mirror to the remote repository.
    ///
    /// This operation:
    /// 1. Gets the repo from DB
    /// 2. Checks credentials are configured (returns error if not)
    /// 3. Runs secret scan on the selected commits (blocks if secrets found in "block" mode)
    /// 4. Determines target branch based on branch_strategy (direct or feature branch with dedup)
    /// 5. If squash requested: creates a squash commit combining selected commits
    /// 6. Builds credential env and pushes to remote
    /// 7. Returns PushResult with branch name and commit count
    ///
    /// # Arguments
    /// - `repo_id`: The ID of the managed repo to push for.
    /// - `commit_shas`: The commit SHAs to push (selected by user in commit review).
    /// - `squash`: Whether to squash the selected commits into one.
    /// - `squash_message`: Optional message for the squash commit.
    ///
    /// # Returns
    /// - `Ok(PushResult { branch, commits })` on success.
    /// - `Err(OrchestratorError::MissingCredentials(...))` if credentials are not configured.
    /// - `Err(OrchestratorError::Validation(...))` if secrets are detected (block mode).
    /// - `Err(OrchestratorError::Internal(...))` on push failure.
    ///
    /// # Requirements: 8.3, 8.4, 8.5, 8.6, 8.9
    pub async fn push_to_remote(
        &self,
        repo_id: &ManagedRepoId,
        commit_shas: &[String],
        squash: bool,
        squash_message: Option<&str>,
    ) -> Result<PushResult, OrchestratorError> {
        // 1. Get the repo from DB
        let repo = self.db.with_conn(|conn| db_ops::get_managed_repo(conn, repo_id))?;

        let mirror = Path::new(&repo.mirror_path);

        // 2. Check credentials are configured (Requirement 8.9)
        let cred_configured =
            repo_credential_manager::credentials_configured(&repo_id.0)?;
        if !cred_configured {
            return Err(OrchestratorError::MissingCredentials(
                "Credentials must be configured before pushing to remote".to_string(),
            ));
        }

        // 3. Run secret scan on selected commits (Requirement 15.1)
        let scan_result = self
            .scanner
            .scan_commits(mirror, commit_shas, &self.git)
            .await;

        match scan_result {
            Err(findings) if repo.secret_scan_mode == SecretScanMode::Block => {
                // Block mode: reject the push with findings
                let findings_desc: Vec<String> = findings
                    .iter()
                    .map(|f| {
                        if f.file_path.is_empty() {
                            f.pattern_name.clone()
                        } else {
                            format!("{}: {}", f.file_path, f.pattern_name)
                        }
                    })
                    .collect();
                return Err(OrchestratorError::Validation(format!(
                    "Secret scan detected potential secrets: {}",
                    findings_desc.join("; ")
                )));
            }
            Err(_findings) => {
                // Warn-only mode: proceed (the frontend handles the warning UI)
                // The API layer should have already confirmed the user wants to proceed
            }
            Ok(_) => {
                // No secrets found, proceed
            }
        }

        // 4. Determine target branch based on branch_strategy (Requirement 8.4)
        let branch = match repo.branch_strategy {
            BranchStrategy::FeatureBranch => {
                let pattern = repo
                    .branch_pattern
                    .as_deref()
                    .unwrap_or("ai/<persona-name>/<date>");
                self.resolve_branch_name(&repo, pattern).await?
            }
            BranchStrategy::Direct => {
                self.git.get_current_branch(mirror).await.map_err(|e| {
                    OrchestratorError::Internal(format!(
                        "Failed to get current branch: {}",
                        e
                    ))
                })?
            }
        };

        if branch.is_empty() {
            return Err(OrchestratorError::Validation(
                "Mirror is in detached HEAD state; cannot push".to_string(),
            ));
        }

        // 5. Handle squash if requested
        if squash && commit_shas.len() > 1 {
            let message = squash_message.unwrap_or("Squashed commits");
            self.squash_commits(mirror, commit_shas, message, &branch)
                .await?;
        }

        // For feature branch strategy, create the branch locally before pushing
        if repo.branch_strategy == BranchStrategy::FeatureBranch {
            // Create the feature branch at the current HEAD (or squashed commit)
            let _ = self
                .git
                .exec(mirror, &["checkout", "-B", &branch], None, false)
                .await
                .map_err(|e| {
                    OrchestratorError::Internal(format!(
                        "Failed to create feature branch '{}': {}",
                        branch, e
                    ))
                })?;
        }

        // 6. Build credential env and push (Requirement 8.3)
        let cred_env = repo_credential_manager::build_credential_env(&repo_id.0)?;

        self.git
            .exec(mirror, &["push", "origin", &branch], Some(&cred_env), true)
            .await
            .map_err(|e| {
                OrchestratorError::Internal(format!("Push to remote failed: {}", e))
            })?;

        // 7. Return PushResult (Requirement 8.5)
        Ok(PushResult {
            branch,
            commits: commit_shas.len() as u32,
        })
    }

    /// Squash multiple commits into a single commit.
    ///
    /// Uses `git reset --soft` to the parent of the first commit, then creates
    /// a new commit with the combined changes and the provided message.
    ///
    /// # Arguments
    /// - `mirror`: Path to the mirror repository.
    /// - `commit_shas`: The commit SHAs to squash (in chronological order).
    /// - `message`: The commit message for the squashed commit.
    /// - `branch`: The target branch name (used for reference).
    async fn squash_commits(
        &self,
        mirror: &Path,
        commit_shas: &[String],
        message: &str,
        _branch: &str,
    ) -> Result<(), OrchestratorError> {
        if commit_shas.is_empty() {
            return Ok(());
        }

        // Get the parent of the first commit to reset to
        let first_sha = &commit_shas[0];
        let parent_ref = format!("{}^", first_sha);

        // Soft reset to the parent of the first commit — keeps all changes staged
        self.git
            .exec(mirror, &["reset", "--soft", &parent_ref], None, false)
            .await
            .map_err(|e| {
                OrchestratorError::Internal(format!(
                    "Failed to soft reset for squash: {}",
                    e
                ))
            })?;

        // Create a new commit with all the staged changes
        self.git
            .exec(mirror, &["commit", "-m", message], None, false)
            .await
            .map_err(|e| {
                OrchestratorError::Internal(format!(
                    "Failed to create squash commit: {}",
                    e
                ))
            })?;

        Ok(())
    }

    /// Fetch new commits from the remote into the mirror.
    ///
    /// Executes `git fetch origin` on the mirror using the credential helper for
    /// authentication, then counts how many commits the local branch is behind
    /// the remote tracking branch.
    ///
    /// # Arguments
    /// - `repo_id`: The ID of the managed repo to fetch for.
    ///
    /// # Returns
    /// - `Ok(SyncResult { commits })` with the number of commits behind the remote.
    /// - `Err(OrchestratorError::MissingCredentials(...))` if credentials are not configured.
    /// - `Err(OrchestratorError::Internal(...))` on fetch failure (network, auth, etc.).
    ///
    /// # Requirements: 9.2, 9.3, 9.4
    pub async fn fetch_from_remote(
        &self,
        repo_id: &ManagedRepoId,
    ) -> Result<SyncResult, OrchestratorError> {
        // 1. Get the repo from DB
        let repo = self.db.with_conn(|conn| {
            db_ops::get_managed_repo(conn, repo_id)
        })?;

        let mirror = Path::new(&repo.mirror_path);

        // 2. Build credential env (return error if credentials not configured)
        let cred_env = repo_credential_manager::build_credential_env(&repo_id.0)?;

        // 3. Execute `git fetch origin` with credentials and network timeout
        self.git
            .exec(mirror, &["fetch", "origin"], Some(&cred_env), true)
            .await
            .map_err(|e| {
                OrchestratorError::Internal(format!("Failed to fetch from remote: {}", e))
            })?;

        // 4. Get current branch
        let branch = self.git.get_current_branch(mirror).await.map_err(|e| {
            OrchestratorError::Internal(format!("Failed to get current branch: {}", e))
        })?;

        // 5. Count commits behind using ahead_behind
        let (_, behind) = self
            .git
            .ahead_behind(mirror, &branch, &format!("origin/{}", branch))
            .await
            .unwrap_or((0, 0));

        // 6. Return SyncResult with the behind count
        Ok(SyncResult { commits: behind })
    }

    /// Scan all persona workspaces for git repositories not yet tracked by Repo Sync.
    ///
    /// Iterates all personas from the database, checks if their workspace_path
    /// contains a `.git` directory, filters out workspaces already tracked via
    /// `managed_repo_exists`, and for detected repos checks if they have remotes
    /// configured.
    ///
    /// # Returns
    /// A list of `DetectedRepo` entries representing untracked git repositories.
    ///
    /// # Requirements: 4.1, 4.2, 4.3, 4.8, 4.9
    pub async fn scan_workspaces(&self) -> Result<Vec<DetectedRepo>, OrchestratorError> {
        // 1. List all personas from DB
        let personas = self.db.with_conn(|conn| db_ops::list_personas(conn))?;

        let mut detected: Vec<DetectedRepo> = Vec::new();

        for persona in &personas {
            let workspace_path = &persona.workspace_path;

            // 2. Check if workspace_path contains a .git directory
            if !workspace_path.join(".git").exists() {
                continue;
            }

            // 3. Filter out workspaces already tracked by Repo Sync
            let workspace_str = workspace_path.to_string_lossy().to_string();
            let already_tracked = self.db.with_conn(|conn| {
                db_ops::managed_repo_exists(conn, &persona.id, &workspace_str)
            })?;

            if already_tracked {
                continue;
            }

            // 4. Check if the repo has remotes configured
            let (has_remotes, remote_url) = match self
                .git
                .list_remote_names(workspace_path)
                .await
            {
                Ok(remotes) => {
                    if remotes.is_empty() {
                        (false, None)
                    } else {
                        // Get the origin URL (or first remote's URL)
                        let primary_remote = if remotes.contains(&"origin".to_string()) {
                            "origin"
                        } else {
                            &remotes[0]
                        };
                        let url = self
                            .git
                            .get_remote_url(workspace_path, primary_remote)
                            .await
                            .unwrap_or(None);
                        (true, url)
                    }
                }
                Err(_) => (false, None),
            };

            // 5. Build DetectedRepo entry
            detected.push(DetectedRepo {
                workspace_path: workspace_str,
                persona_id: persona.id.0.clone(),
                persona_name: persona.name.clone(),
                has_remotes,
                remote_url,
            });
        }

        Ok(detected)
    }

    /// List unpushed commits in the mirror for a managed repo.
    ///
    /// Gets commits that exist in the mirror but have not been pushed to the remote
    /// tracking branch. Uses `git log origin/<branch>..HEAD` if a remote tracking
    /// branch exists, or `git log` (all commits) if no remote is configured.
    ///
    /// Parses each commit's SHA, message, author, timestamp, and diff stats
    /// (files changed, insertions, deletions) using `--format` and `--numstat`
    /// in a single pass.
    ///
    /// # Arguments
    /// - `repo_id`: The ID of the managed repo.
    ///
    /// # Returns
    /// A list of `CommitInfo` entries (max 500), ordered newest-first.
    ///
    /// # Requirements: 18.3, 18.9
    pub async fn list_commits(
        &self,
        repo_id: &ManagedRepoId,
    ) -> Result<Vec<CommitInfo>, OrchestratorError> {
        let repo = self
            .db
            .with_conn(|conn| db_ops::get_managed_repo(conn, repo_id))?;

        let mirror = Path::new(&repo.mirror_path);

        // Get current branch
        let branch = self.git.get_current_branch(mirror).await.map_err(|e| {
            OrchestratorError::Internal(format!("Failed to get current branch: {}", e))
        })?;

        if branch.is_empty() {
            return Err(OrchestratorError::Validation(
                "Mirror is in detached HEAD state; cannot list commits".to_string(),
            ));
        }

        // Determine the commit range:
        // - If remote tracking branch exists: origin/<branch>..HEAD (unpushed only)
        // - If no remote tracking: all commits on HEAD
        let range = {
            let remote_ref = format!("origin/{}", branch);
            // Check if the remote tracking branch exists
            let check = self
                .git
                .exec(
                    mirror,
                    &["rev-parse", "--verify", &remote_ref],
                    None,
                    false,
                )
                .await;
            if check.is_ok() {
                format!("{}..HEAD", remote_ref)
            } else {
                "HEAD".to_string()
            }
        };

        // Use a custom format with a unique separator to parse fields reliably.
        // Format: SHA<SEP>message<SEP>author<SEP>timestamp
        // Then --numstat gives file stats after each commit entry.
        let separator = "---COMMIT_SEP---";
        let format_str = format!(
            "{}%H{}%s{}%an{}%aI",
            separator, separator, separator, separator
        );

        let format_arg = format!("--format={}", format_str);
        let args: Vec<&str> = vec![
            "log",
            &format_arg,
            "--numstat",
            "-n",
            "500",
            &range,
        ];

        let output = self
            .git
            .exec(mirror, &args, None, false)
            .await
            .map_err(|e| {
                OrchestratorError::Internal(format!("Failed to list commits: {}", e))
            })?;

        // Parse the output
        let commits = Self::parse_log_output(&output.stdout, separator);

        Ok(commits)
    }

    /// Parse the output of `git log --format=<sep>%H<sep>%s<sep>%an<sep>%aI --numstat`.
    ///
    /// The output format is:
    /// ```text
    /// ---COMMIT_SEP---<sha>---COMMIT_SEP---<message>---COMMIT_SEP---<author>---COMMIT_SEP---<timestamp>
    /// <added>\t<deleted>\t<filename>
    /// <added>\t<deleted>\t<filename>
    /// ...
    /// ---COMMIT_SEP---<sha>---COMMIT_SEP---...
    /// ```
    fn parse_log_output(output: &str, separator: &str) -> Vec<CommitInfo> {
        let mut commits: Vec<CommitInfo> = Vec::new();

        // Split by the separator that starts each commit entry
        let parts: Vec<&str> = output.split(separator).collect();

        // The first element is empty (before the first separator), skip it.
        // Then we process groups of 4 fields: SHA, message, author, timestamp
        // followed by numstat lines until the next separator.
        let mut i = 1; // skip the empty first element
        while i + 3 < parts.len() {
            let sha = parts[i].trim().to_string();
            let message = parts[i + 1].trim().to_string();
            let author = parts[i + 2].trim().to_string();

            // The timestamp part may contain trailing numstat lines
            let timestamp_and_stats = parts[i + 3];

            // Split timestamp from numstat lines: timestamp is on the first line
            let mut lines = timestamp_and_stats.lines();
            let timestamp = lines.next().unwrap_or("").trim().to_string();

            // Remaining lines are numstat entries (added\tdeleted\tfilename)
            let mut files_changed: u32 = 0;
            let mut insertions: u32 = 0;
            let mut deletions: u32 = 0;

            for line in lines {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let stat_parts: Vec<&str> = line.split('\t').collect();
                if stat_parts.len() >= 3 {
                    files_changed += 1;
                    // "-" means binary file (no line count)
                    if let Ok(added) = stat_parts[0].parse::<u32>() {
                        insertions += added;
                    }
                    if let Ok(deleted) = stat_parts[1].parse::<u32>() {
                        deletions += deleted;
                    }
                }
            }

            if !sha.is_empty() {
                commits.push(CommitInfo {
                    sha,
                    message,
                    author,
                    timestamp,
                    files_changed,
                    insertions,
                    deletions,
                });
            }

            i += 4;
        }

        commits
    }

    /// Spawn a background task that periodically checks for new commits in both
    /// directions (workspace→mirror and remote→mirror) for all managed repos.
    ///
    /// The checker runs every 60 seconds (base interval). For each repo, it only
    /// performs a check if enough time has passed since the last check (per-repo
    /// `check_interval_seconds`). Results are cached in `self.cached_status` for
    /// lightweight polling by the frontend.
    ///
    /// # Requirements: 16.1, 16.2, 16.3, 16.5, 16.6, 16.7
    pub fn start_background_checker(&self) -> JoinHandle<()> {
        let db = self.db.clone();
        let git = self.git.clone();
        let cached_status = self.cached_status.clone();
        let last_check_times = self.last_check_times.clone();

        tokio::spawn(async move {
            loop {
                let repos = db.with_conn(|conn| {
                    db_ops::list_managed_repos(conn)
                }).unwrap_or_default();

                for repo in &repos {
                    let repo_id = repo.id.0.clone();
                    let interval = Duration::from_secs(repo.check_interval_seconds as u64);

                    // Skip if not enough time has passed since last check
                    if let Some(last_check) = last_check_times.get(&repo_id) {
                        if last_check.elapsed() < interval {
                            continue;
                        }
                    }

                    let mut status = SyncStatus {
                        workspace_ahead: 0,
                        mirror_ahead: 0,
                        remote_ahead: 0,
                    };

                    // Check workspace → mirror (local, fast)
                    let mirror_path = Path::new(&repo.mirror_path);
                    let workspace_path = Path::new(&repo.workspace_path);

                    if mirror_path.join(".git").exists() && workspace_path.join(".git").exists() {
                        // Fetch from workspace into mirror to get latest refs
                        let workspace_str = repo.workspace_path.clone();
                        let fetch_result = git
                            .exec(mirror_path, &["fetch", &workspace_str], None, false)
                            .await;

                        if fetch_result.is_ok() {
                            // Get current branch in mirror
                            if let Ok(branch) = git.get_current_branch(mirror_path).await {
                                if !branch.is_empty() {
                                    // Count workspace commits ahead of mirror
                                    let ahead = git
                                        .ahead_behind(mirror_path, "FETCH_HEAD", &branch)
                                        .await
                                        .map(|(ahead, _)| ahead)
                                        .unwrap_or(0);
                                    status.workspace_ahead = ahead;
                                }
                            }
                        }
                    }

                    // Check remote → mirror (network, uses credentials)
                    // Skip for LocalOnly repos (Requirement 16.6)
                    if repo.sync_mode != SyncMode::LocalOnly {
                        if mirror_path.join(".git").exists() {
                            // Only attempt remote check if credentials are configured
                            if let Ok(cred_env) =
                                repo_credential_manager::build_credential_env(&repo_id)
                            {
                                let fetch_result = git
                                    .exec(
                                        mirror_path,
                                        &["fetch", "origin"],
                                        Some(&cred_env),
                                        true,
                                    )
                                    .await;

                                if fetch_result.is_ok() {
                                    if let Ok(branch) =
                                        git.get_current_branch(mirror_path).await
                                    {
                                        if !branch.is_empty() {
                                            let remote_ref =
                                                format!("origin/{}", branch);
                                            let behind = git
                                                .ahead_behind(
                                                    mirror_path,
                                                    &branch,
                                                    &remote_ref,
                                                )
                                                .await
                                                .map(|(_, behind)| behind)
                                                .unwrap_or(0);
                                            status.remote_ahead = behind;
                                        }
                                    }
                                }
                                // Requirement 16.5: If check fails, log and retry next interval
                            }
                        }
                    }

                    // Update cached sync status
                    cached_status.insert(repo_id.clone(), status);
                    last_check_times.insert(repo_id, Instant::now());
                }

                // Base interval: sleep 60 seconds between sweeps
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        })
    }

    /// Returns the cached sync status for all repos.
    ///
    /// This is a lightweight read from the in-memory cache populated by the
    /// background checker. Used by `GET /api/repo-sync/repos` to include status
    /// without performing git operations on every request.
    pub fn get_cached_status(&self) -> HashMap<String, SyncStatus> {
        self.cached_status
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Returns true if any managed repo has pending commits in either direction.
    ///
    /// Used by the `GET /api/repo-sync/status` endpoint to drive the sidebar
    /// notification badge.
    ///
    /// # Requirements: 16.2, 16.3, 16.7
    pub fn has_pending(&self) -> bool {
        self.cached_status.iter().any(|entry| {
            let status = entry.value();
            status.workspace_ahead > 0 || status.remote_ahead > 0
        })
    }

    /// Update a managed repo's configuration.
    ///
    /// Validates:
    /// - Remote URL format (if provided)
    /// - Branch pattern (max 200 chars, no invalid git branch chars)
    /// - Sync mode prerequisites: changing to Remote requires remote URL and credentials
    ///
    /// Only fields present in the request are updated; `None` fields are left unchanged.
    ///
    /// # Requirements: 12.2, 12.4, 12.5, 12.6, 18.4
    pub async fn update_repo(
        &self,
        id: &ManagedRepoId,
        req: &UpdateRepoRequest,
    ) -> Result<ManagedRepo, OrchestratorError> {
        // 1. Get existing repo from DB
        let mut repo = self.db.with_conn(|conn| db_ops::get_managed_repo(conn, id))?;

        // 2. Validate remote URL format if provided (Requirement 12.2)
        if let Some(ref url) = req.remote_url {
            Self::validate_remote_url(url)?;
        }

        // 3. Validate branch pattern if provided (Requirement 12.4)
        if let Some(ref pattern) = req.branch_pattern {
            Self::validate_branch_pattern(pattern)?;
        }

        // 4. If changing sync_mode to Remote: verify prerequisites (Requirement 12.5)
        if let Some(SyncMode::Remote) = req.sync_mode {
            // The effective remote_url after this update
            let effective_url = req.remote_url.as_ref().or(repo.remote_url.as_ref());
            if effective_url.is_none() {
                return Err(OrchestratorError::Validation(
                    "Cannot change sync mode to Remote: remote URL is not configured".to_string(),
                ));
            }

            // Check credentials are configured
            let cred_configured = repo_credential_manager::credentials_configured(&id.0)?;
            if !cred_configured {
                return Err(OrchestratorError::Validation(
                    "Cannot change sync mode to Remote: credentials are not configured"
                        .to_string(),
                ));
            }
        }

        // 5. Apply updates to the repo record
        if let Some(ref url) = req.remote_url {
            repo.remote_url = Some(url.clone());
            // Auto-detect provider from URL
            repo.remote_provider = Self::detect_remote_provider(url);
        }
        if let Some(ref provider) = req.remote_provider {
            repo.remote_provider = Some(provider.clone());
        }
        if let Some(ref strategy) = req.branch_strategy {
            repo.branch_strategy = strategy.clone();
        }
        if let Some(ref pattern) = req.branch_pattern {
            repo.branch_pattern = Some(pattern.clone());
        }
        if let Some(ref mode) = req.attribution_mode {
            repo.attribution_mode = mode.clone();
        }
        if let Some(ref mode) = req.sync_mode {
            repo.sync_mode = mode.clone();
        }
        if let Some(ref mode) = req.secret_scan_mode {
            repo.secret_scan_mode = mode.clone();
        }
        if let Some(interval) = req.check_interval_seconds {
            repo.check_interval_seconds = interval;
        }

        repo.updated_at = Utc::now();

        // 6. Save to DB
        self.db
            .with_conn(|conn| db_ops::update_managed_repo(conn, id, &repo))?;

        Ok(repo)
    }

    /// Validate a branch pattern string.
    ///
    /// Rules:
    /// - Maximum 200 characters
    /// - No characters invalid for git branch names: space, ~, ^, :, ?, *, [, \, control chars
    /// - Cannot start or end with a dot or slash
    /// - Cannot contain consecutive dots (..) or consecutive slashes (//)
    fn validate_branch_pattern(pattern: &str) -> Result<(), OrchestratorError> {
        if pattern.is_empty() {
            return Err(OrchestratorError::Validation(
                "Branch pattern cannot be empty".to_string(),
            ));
        }
        if pattern.len() > 200 {
            return Err(OrchestratorError::Validation(
                "Branch pattern exceeds maximum length of 200 characters".to_string(),
            ));
        }

        // Characters invalid in git branch names
        let invalid_chars = [' ', '~', '^', ':', '?', '*', '[', '\\'];
        for ch in pattern.chars() {
            if invalid_chars.contains(&ch) || ch.is_control() {
                return Err(OrchestratorError::Validation(format!(
                    "Branch pattern contains invalid character: {:?}",
                    ch
                )));
            }
        }

        // Cannot start or end with dot or slash
        if pattern.starts_with('.') || pattern.starts_with('/') {
            return Err(OrchestratorError::Validation(
                "Branch pattern cannot start with '.' or '/'".to_string(),
            ));
        }
        if pattern.ends_with('.') || pattern.ends_with('/') {
            return Err(OrchestratorError::Validation(
                "Branch pattern cannot end with '.' or '/'".to_string(),
            ));
        }

        // Cannot contain consecutive dots or slashes
        if pattern.contains("..") {
            return Err(OrchestratorError::Validation(
                "Branch pattern cannot contain consecutive dots '..'".to_string(),
            ));
        }
        if pattern.contains("//") {
            return Err(OrchestratorError::Validation(
                "Branch pattern cannot contain consecutive slashes '//'".to_string(),
            ));
        }

        Ok(())
    }

    /// Delete a managed repo.
    ///
    /// 1. Gets the repo from DB
    /// 2. Deletes keyring credentials (best-effort, don't fail if keyring unavailable)
    /// 3. Deletes the DB record
    /// 4. If `delete_mirror` is true: deletes the mirror directory from disk
    ///
    /// The workspace remotes are NOT restored on deletion.
    ///
    /// # Requirements: 18.10, 14.3
    pub async fn delete_repo(
        &self,
        id: &ManagedRepoId,
        delete_mirror: bool,
    ) -> Result<(), OrchestratorError> {
        // 1. Get repo from DB (validates it exists)
        let repo = self.db.with_conn(|conn| db_ops::get_managed_repo(conn, id))?;

        // 2. Delete keyring credentials (best-effort)
        // Per implementation guidance #6: deletion/cleanup failures should be best-effort
        let _ = repo_credential_manager::delete_credentials(&id.0);

        // 3. Delete DB record
        self.db
            .with_conn(|conn| db_ops::delete_managed_repo(conn, id))?;

        // 4. If delete_mirror is true: delete the mirror directory from disk
        if delete_mirror {
            let mirror_path = Path::new(&repo.mirror_path);
            if mirror_path.exists() {
                std::fs::remove_dir_all(mirror_path).map_err(|e| {
                    OrchestratorError::Internal(format!(
                        "Failed to delete mirror directory '{}': {}",
                        repo.mirror_path, e
                    ))
                })?;
            }
        }

        Ok(())
    }

    /// Update the mirrors directory path.
    ///
    /// Validates:
    /// - Path is not empty
    /// - Path is absolute
    /// - Path does not exceed 4096 characters
    ///
    /// If the directory does not exist, it is created. Updates the `mirror_path`
    /// in all affected ManagedRepo records to reflect the new base directory.
    ///
    /// # Requirements: 14.3, 14.4, 14.5, 14.6
    pub fn update_mirrors_dir(&mut self, new_path: &str) -> Result<PathBuf, OrchestratorError> {
        // 1. Validate path
        if new_path.is_empty() {
            return Err(OrchestratorError::Validation(
                "Mirrors directory path cannot be empty".to_string(),
            ));
        }
        if new_path.len() > 4096 {
            return Err(OrchestratorError::Validation(
                "Mirrors directory path exceeds maximum length of 4096 characters".to_string(),
            ));
        }

        let new_dir = PathBuf::from(new_path);
        if !new_dir.is_absolute() {
            return Err(OrchestratorError::Validation(
                "Mirrors directory path must be absolute".to_string(),
            ));
        }

        // 2. Create directory if it doesn't exist
        if !new_dir.exists() {
            std::fs::create_dir_all(&new_dir).map_err(|e| {
                OrchestratorError::Validation(format!(
                    "Cannot create mirrors directory '{}': {}",
                    new_path, e
                ))
            })?;
        }

        // 3. Update mirror_path in all affected ManagedRepo records
        let old_dir = self.mirrors_dir.clone();
        let old_dir_str = old_dir.to_string_lossy().to_string();

        self.db.with_conn(|conn| {
            let repos = db_ops::list_managed_repos(conn)?;
            for repo in &repos {
                // Only update repos whose mirror_path starts with the old mirrors_dir
                if repo.mirror_path.starts_with(&old_dir_str) {
                    let relative = &repo.mirror_path[old_dir_str.len()..];
                    // Strip leading separator if present
                    let relative = relative
                        .strip_prefix('/')
                        .or_else(|| relative.strip_prefix('\\'))
                        .unwrap_or(relative);
                    let updated_path = new_dir.join(relative);
                    let updated_path_str = updated_path.to_string_lossy().to_string();
                    let now = Utc::now().to_rfc3339();
                    db_ops::update_managed_repo_mirror_path(
                        conn,
                        &repo.id,
                        &updated_path_str,
                        &now,
                    )?;
                }
            }
            Ok(())
        })?;

        // 4. Update the manager's mirrors_dir
        self.mirrors_dir = new_dir.clone();

        Ok(new_dir)
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

    // --- Tests for push_to_agent ---

    /// Helper to set up a workspace+mirror pair for push_to_agent tests.
    /// The mirror is cloned from the workspace, so both start at the same commit.
    /// Workspace git config is set for commits.
    async fn setup_push_to_agent_test() -> (
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

        // Create mirror repo (source of truth for push_to_agent)
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

        // Clone mirror to create workspace
        let workspace_dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args([
                "clone",
                mirror_dir.path().to_str().unwrap(),
                workspace_dir.path().to_str().unwrap(),
            ])
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
        // Remove origin from workspace (simulating repo sync isolation)
        std::process::Command::new("git")
            .args(["remote", "remove", "origin"])
            .current_dir(workspace_dir.path())
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
    async fn test_push_to_agent_no_new_commits() {
        let (manager, repo_id, _workspace_dir, _mirror_dir) = setup_push_to_agent_test().await;

        let result = manager.push_to_agent(&repo_id).await.unwrap();
        assert_eq!(result.commits, 0);
    }

    #[tokio::test]
    async fn test_push_to_agent_applies_commits() {
        let (manager, repo_id, workspace_dir, mirror_dir) = setup_push_to_agent_test().await;

        // Add commits to mirror
        std::fs::write(mirror_dir.path().join("file1.txt"), "content1").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "add file1"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();

        std::fs::write(mirror_dir.path().join("file2.txt"), "content2").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "add file2"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();

        let result = manager.push_to_agent(&repo_id).await.unwrap();
        assert_eq!(result.commits, 2);

        // Verify files exist in workspace
        assert!(workspace_dir.path().join("file1.txt").exists());
        assert!(workspace_dir.path().join("file2.txt").exists());
    }

    #[tokio::test]
    async fn test_push_to_agent_dirty_workspace_rejected() {
        let (manager, repo_id, workspace_dir, _mirror_dir) = setup_push_to_agent_test().await;

        // Create uncommitted changes in workspace
        std::fs::write(workspace_dir.path().join("dirty.txt"), "uncommitted").unwrap();

        let result = manager.push_to_agent(&repo_id).await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("uncommitted changes"),
            "Expected error about uncommitted changes, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_push_to_agent_merge_conflict() {
        let (manager, repo_id, workspace_dir, mirror_dir) = setup_push_to_agent_test().await;

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

        // Push to agent should fail with a conflict error
        let err = manager.push_to_agent(&repo_id).await.unwrap_err();
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("README.md") || err_msg.contains("conflict") || err_msg.contains("Merge"),
            "Expected conflict error mentioning file, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_push_to_agent_repo_not_found() {
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
            .push_to_agent(&ManagedRepoId("nonexistent".to_string()))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    // --- Tests for push_to_remote ---

    /// Helper to set up a mirror repo for push_to_remote tests.
    /// Returns (manager, repo_id, mirror_dir) with a managed repo record.
    async fn setup_push_to_remote_test(
        branch_strategy: BranchStrategy,
        secret_scan_mode: SecretScanMode,
    ) -> (RepoSyncManager, ManagedRepoId, tempfile::TempDir) {
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

        // Create mirror repo with some commits
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

        // Insert managed repo record
        let repo_id = ManagedRepoId::new();
        let now = Utc::now();
        let repo = ManagedRepo {
            id: repo_id.clone(),
            persona_id: PersonaId("p1".to_string()),
            workspace_path: "/tmp/workspace".to_string(),
            mirror_path: mirror_dir.path().to_string_lossy().to_string(),
            remote_url: Some("https://github.com/user/repo.git".to_string()),
            remote_provider: Some(RemoteProvider::Github),
            branch_strategy,
            branch_pattern: Some("ai/<persona-name>/<date>".to_string()),
            attribution_mode: AttributionMode::KeepAgent,
            sync_mode: SyncMode::Remote,
            secret_scan_mode,
            check_interval_seconds: 300,
            created_at: now,
            updated_at: now,
        };
        db.with_conn(|conn| db_ops::insert_managed_repo(conn, &repo)).unwrap();

        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, PathBuf::from("/tmp/mirrors"));

        (manager, repo_id, mirror_dir)
    }

    #[tokio::test]
    async fn test_push_to_remote_fails_without_credentials() {
        let (manager, repo_id, _mirror_dir) =
            setup_push_to_remote_test(BranchStrategy::Direct, SecretScanMode::Block).await;

        let result = manager
            .push_to_remote(&repo_id, &["abc123".to_string()], false, None)
            .await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("Credentials") || err_msg.contains("credentials"),
            "Expected credentials error, got: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_push_to_remote_repo_not_found() {
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
            .push_to_remote(
                &ManagedRepoId("nonexistent".to_string()),
                &["abc123".to_string()],
                false,
                None,
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_squash_commits_combines_into_one() {
        let (_manager, _repo_id, mirror_dir) =
            setup_push_to_remote_test(BranchStrategy::Direct, SecretScanMode::Block).await;

        // Add multiple commits to the mirror
        std::fs::write(mirror_dir.path().join("file1.txt"), "content1").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "add file1"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();

        std::fs::write(mirror_dir.path().join("file2.txt"), "content2").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "add file2"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();

        // Get the two commit SHAs
        let log_output = std::process::Command::new("git")
            .args(["log", "--format=%H", "-2"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        let shas: Vec<String> = String::from_utf8_lossy(&log_output.stdout)
            .trim()
            .lines()
            .rev() // oldest first
            .map(|s| s.to_string())
            .collect();

        assert_eq!(shas.len(), 2);

        // Squash the commits
        let result = _manager
            .squash_commits(mirror_dir.path(), &shas, "Squashed: file1 + file2", "master")
            .await;
        assert!(result.is_ok(), "squash_commits failed: {:?}", result.err());

        // Verify there's now only one commit after the initial
        let count_output = std::process::Command::new("git")
            .args(["rev-list", "--count", "HEAD"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        let count: u32 = String::from_utf8_lossy(&count_output.stdout)
            .trim()
            .parse()
            .unwrap();
        // Should be 2: initial + squashed
        assert_eq!(count, 2, "Expected 2 commits (initial + squashed), got {}", count);

        // Verify the squash commit message
        let msg_output = std::process::Command::new("git")
            .args(["log", "-1", "--format=%s"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        let msg = String::from_utf8_lossy(&msg_output.stdout).trim().to_string();
        assert_eq!(msg, "Squashed: file1 + file2");

        // Verify both files still exist
        assert!(mirror_dir.path().join("file1.txt").exists());
        assert!(mirror_dir.path().join("file2.txt").exists());
    }

    // --- fetch_from_remote tests ---

    #[tokio::test]
    async fn test_fetch_from_remote_repo_not_found() {
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
            .fetch_from_remote(&ManagedRepoId("nonexistent".to_string()))
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    /// Helper to set up a mirror with a local bare "remote" for fetch_from_remote tests.
    /// Returns (manager, repo_id, bare_remote_dir, mirror_dir).
    async fn setup_fetch_test() -> (
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

        // Create a bare repo to act as "remote"
        let bare_dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["init", "--bare"])
            .current_dir(bare_dir.path())
            .output()
            .unwrap();

        // Create a working repo, commit, and push to the bare repo
        let work_dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["clone", bare_dir.path().to_str().unwrap(), work_dir.path().to_str().unwrap()])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(work_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(work_dir.path())
            .output()
            .unwrap();
        std::fs::write(work_dir.path().join("README.md"), "# Test").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(work_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(work_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["push", "origin", "master"])
            .current_dir(work_dir.path())
            .output()
            .unwrap();

        // Clone the bare repo to create the mirror
        let mirror_dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["clone", bare_dir.path().to_str().unwrap(), mirror_dir.path().to_str().unwrap()])
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
            workspace_path: "/tmp/workspace".to_string(),
            mirror_path: mirror_dir.path().to_string_lossy().to_string(),
            remote_url: Some(bare_dir.path().to_string_lossy().to_string()),
            remote_provider: None,
            branch_strategy: BranchStrategy::Direct,
            branch_pattern: None,
            attribution_mode: AttributionMode::KeepAgent,
            sync_mode: SyncMode::Remote,
            secret_scan_mode: SecretScanMode::Block,
            check_interval_seconds: 300,
            created_at: now,
            updated_at: now,
        };
        db.with_conn(|conn| db_ops::insert_managed_repo(conn, &repo)).unwrap();

        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, PathBuf::from("/tmp/mirrors"));

        (manager, repo_id, bare_dir, mirror_dir)
    }

    #[tokio::test]
    async fn test_fetch_from_remote_credential_env_required() {
        let (manager, repo_id, _bare_dir, _mirror_dir) = setup_fetch_test().await;

        // fetch_from_remote uses build_credential_env which requires the askpass binary.
        // In test env, the binary won't exist, so this will fail with an Internal error
        // about the missing binary. This validates the credential env is built correctly.
        let result = manager.fetch_from_remote(&repo_id).await;

        // The test environment won't have beachead-askpass binary, so we expect
        // an error about the missing credential helper. This confirms the method
        // correctly attempts to build credentials before fetching.
        match result {
            Err(e) => {
                let err_msg = e.to_string();
                assert!(
                    err_msg.contains("beachead-askpass") || err_msg.contains("not found"),
                    "Expected credential helper error, got: {}",
                    err_msg
                );
            }
            Ok(_) => {
                // If somehow the binary exists (full build), the fetch should succeed
                // with 0 commits behind since mirror is up to date with remote
            }
        }
    }

    #[tokio::test]
    async fn test_fetch_from_remote_ahead_behind_logic() {
        let (_manager, _repo_id, bare_dir, mirror_dir) = setup_fetch_test().await;

        // Push new commits to the bare remote (simulating upstream changes)
        let work_dir = tempfile::tempdir().unwrap();
        std::process::Command::new("git")
            .args(["clone", bare_dir.path().to_str().unwrap(), work_dir.path().to_str().unwrap()])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(work_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(work_dir.path())
            .output()
            .unwrap();
        std::fs::write(work_dir.path().join("new_file.txt"), "new content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(work_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "upstream commit 1"])
            .current_dir(work_dir.path())
            .output()
            .unwrap();
        std::fs::write(work_dir.path().join("new_file2.txt"), "more content").unwrap();
        std::process::Command::new("git")
            .args(["add", "."])
            .current_dir(work_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "upstream commit 2"])
            .current_dir(work_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["push", "origin", "master"])
            .current_dir(work_dir.path())
            .output()
            .unwrap();

        // Manually fetch in the mirror (bypassing credential env since it's local)
        // to verify the ahead_behind logic that fetch_from_remote uses
        let git = Arc::new(GitCli::new("git".to_string()));
        git.exec(mirror_dir.path(), &["fetch", "origin"], None, false)
            .await
            .unwrap();

        // Verify the mirror is now 2 commits behind origin/master
        let branch = git.get_current_branch(mirror_dir.path()).await.unwrap();
        let (_, behind) = git
            .ahead_behind(mirror_dir.path(), &branch, &format!("origin/{}", branch))
            .await
            .unwrap();
        assert_eq!(behind, 2, "Mirror should be 2 commits behind remote");
    }

    // --- Tests for scan_workspaces ---

    #[tokio::test]
    async fn test_scan_workspaces_detects_git_repos() {
        // Create a workspace with a git repo
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();

        let db = Arc::new(Database::open_in_memory().unwrap());
        let workspace_str = workspace.path().to_string_lossy().to_string();
        db.with_conn(|conn| {
            conn.execute_batch(&format!(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{{}}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');
                 INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('p1', 'my-agent', 'a1', '{}', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');",
                workspace_str
            )).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        }).unwrap();

        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, mirrors_dir.path().to_path_buf());

        let detected = manager.scan_workspaces().await.unwrap();
        assert_eq!(detected.len(), 1);
        assert_eq!(detected[0].persona_id, "p1");
        assert_eq!(detected[0].persona_name, "my-agent");
        assert_eq!(detected[0].workspace_path, workspace_str);
        assert!(!detected[0].has_remotes);
        assert!(detected[0].remote_url.is_none());
    }

    #[tokio::test]
    async fn test_scan_workspaces_skips_already_tracked() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();

        let db = Arc::new(Database::open_in_memory().unwrap());
        let workspace_str = workspace.path().to_string_lossy().to_string();
        db.with_conn(|conn| {
            conn.execute_batch(&format!(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{{}}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');
                 INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('p1', 'my-agent', 'a1', '{}', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');
                 INSERT INTO managed_repos (id, persona_id, workspace_path, mirror_path, branch_strategy, attribution_mode, sync_mode, secret_scan_mode, check_interval_seconds, created_at, updated_at)
                 VALUES ('r1', 'p1', '{}', '/tmp/mirror', 'direct', 'keep_agent', 'local_only', 'block', 300, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');",
                workspace_str, workspace_str
            )).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        }).unwrap();

        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, mirrors_dir.path().to_path_buf());

        let detected = manager.scan_workspaces().await.unwrap();
        assert!(detected.is_empty(), "Already-tracked repos should be filtered out");
    }

    #[tokio::test]
    async fn test_scan_workspaces_skips_non_git_dirs() {
        // Create a workspace that is NOT a git repo
        let workspace = tempfile::tempdir().unwrap();
        std::fs::write(workspace.path().join("README.md"), "# Not a git repo").unwrap();

        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open_in_memory().unwrap());
        let workspace_str = workspace.path().to_string_lossy().to_string();
        db.with_conn(|conn| {
            conn.execute_batch(&format!(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{{}}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');
                 INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('p1', 'my-agent', 'a1', '{}', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');",
                workspace_str
            )).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        }).unwrap();

        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, mirrors_dir.path().to_path_buf());

        let detected = manager.scan_workspaces().await.unwrap();
        assert!(detected.is_empty(), "Non-git directories should be skipped");
    }

    #[tokio::test]
    async fn test_scan_workspaces_detects_remotes() {
        let workspace = create_test_workspace();
        // Add a remote
        std::process::Command::new("git")
            .args(["remote", "add", "origin", "https://github.com/user/repo.git"])
            .current_dir(workspace.path())
            .output()
            .unwrap();

        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = Arc::new(Database::open_in_memory().unwrap());
        let workspace_str = workspace.path().to_string_lossy().to_string();
        db.with_conn(|conn| {
            conn.execute_batch(&format!(
                "INSERT INTO agent_types (id, name, sbx_agent, is_builtin, metadata, created_at, updated_at)
                 VALUES ('a1', 'claude', 'claude', 1, '{{}}', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');
                 INSERT INTO personas (id, name, agent_type_id, workspace_path, memory_enabled, created_at, updated_at)
                 VALUES ('p1', 'my-agent', 'a1', '{}', 0, '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z');",
                workspace_str
            )).map_err(|e| OrchestratorError::Database(e.to_string()))?;
            Ok(())
        }).unwrap();

        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, mirrors_dir.path().to_path_buf());

        let detected = manager.scan_workspaces().await.unwrap();
        assert_eq!(detected.len(), 1);
        assert!(detected[0].has_remotes);
        assert_eq!(detected[0].remote_url.as_deref(), Some("https://github.com/user/repo.git"));
    }

    // --- Tests for list_commits ---

    #[tokio::test]
    async fn test_list_commits_returns_unpushed() {
        // Create a workspace with commits, clone to mirror, add more commits to mirror
        let workspace = create_test_workspace();
        let mirror_dir = tempfile::tempdir().unwrap();

        // Clone workspace to mirror
        std::process::Command::new("git")
            .args([
                "clone",
                workspace.path().to_str().unwrap(),
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
            .args(["config", "user.name", "TestAuthor"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();

        // Add commits to mirror (these are "unpushed" relative to origin)
        std::fs::write(mirror_dir.path().join("file1.txt"), "content1").unwrap();
        std::process::Command::new("git")
            .args(["add", "file1.txt"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "Add file1"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();

        std::fs::write(mirror_dir.path().join("file2.txt"), "content2").unwrap();
        std::process::Command::new("git")
            .args(["add", "file2.txt"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "Add file2"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();

        // Set up DB with managed repo
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

        let repo_id = ManagedRepoId("r1".to_string());
        let now = Utc::now();
        let repo = ManagedRepo {
            id: repo_id.clone(),
            persona_id: PersonaId("p1".to_string()),
            workspace_path: workspace.path().to_string_lossy().to_string(),
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

        let commits = manager.list_commits(&repo_id).await.unwrap();

        // Should have 2 unpushed commits
        assert_eq!(commits.len(), 2, "Expected 2 unpushed commits, got {}", commits.len());

        // Newest first
        assert_eq!(commits[0].message, "Add file2");
        assert_eq!(commits[1].message, "Add file1");

        // Check author
        assert_eq!(commits[0].author, "TestAuthor");

        // Check stats (each commit adds 1 file with 1 line)
        assert_eq!(commits[0].files_changed, 1);
        assert_eq!(commits[0].insertions, 1);
        assert_eq!(commits[0].deletions, 0);
    }

    #[tokio::test]
    async fn test_list_commits_no_remote_tracking() {
        // Create a standalone mirror with no remote tracking branch
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
            .args(["commit", "-m", "initial commit"])
            .current_dir(mirror_dir.path())
            .output()
            .unwrap();

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

        let repo_id = ManagedRepoId("r1".to_string());
        let now = Utc::now();
        let repo = ManagedRepo {
            id: repo_id.clone(),
            persona_id: PersonaId("p1".to_string()),
            workspace_path: "/tmp/workspace".to_string(),
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

        let commits = manager.list_commits(&repo_id).await.unwrap();

        // With no remote tracking, all commits on HEAD are returned
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].message, "initial commit");
    }

    // --- Tests for parse_log_output ---

    #[test]
    fn test_parse_log_output_single_commit() {
        let sep = "---COMMIT_SEP---";
        let output = format!(
            "{}abc123{}Fix bug{}Alice{}2024-01-15T10:30:00+00:00\n3\t1\tsrc/main.rs\n0\t5\tREADME.md\n",
            sep, sep, sep, sep
        );

        let commits = RepoSyncManager::parse_log_output(&output, sep);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].sha, "abc123");
        assert_eq!(commits[0].message, "Fix bug");
        assert_eq!(commits[0].author, "Alice");
        assert_eq!(commits[0].timestamp, "2024-01-15T10:30:00+00:00");
        assert_eq!(commits[0].files_changed, 2);
        assert_eq!(commits[0].insertions, 3);
        assert_eq!(commits[0].deletions, 6);
    }

    #[test]
    fn test_parse_log_output_multiple_commits() {
        let sep = "---COMMIT_SEP---";
        let output = format!(
            "{}sha1{}First commit{}Bob{}2024-01-01T00:00:00Z\n1\t0\tfile.txt\n{}sha2{}Second commit{}Alice{}2024-01-02T00:00:00Z\n2\t1\tother.rs\n",
            sep, sep, sep, sep, sep, sep, sep, sep
        );

        let commits = RepoSyncManager::parse_log_output(&output, sep);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].sha, "sha1");
        assert_eq!(commits[0].message, "First commit");
        assert_eq!(commits[1].sha, "sha2");
        assert_eq!(commits[1].message, "Second commit");
    }

    #[test]
    fn test_parse_log_output_empty() {
        let sep = "---COMMIT_SEP---";
        let commits = RepoSyncManager::parse_log_output("", sep);
        assert!(commits.is_empty());
    }

    #[test]
    fn test_parse_log_output_binary_files() {
        let sep = "---COMMIT_SEP---";
        // Binary files show "-" for added/deleted lines
        let output = format!(
            "{}abc123{}Add image{}Dev{}2024-01-15T10:30:00Z\n-\t-\timage.png\n5\t0\tindex.html\n",
            sep, sep, sep, sep
        );

        let commits = RepoSyncManager::parse_log_output(&output, sep);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].files_changed, 2); // binary file still counts
        assert_eq!(commits[0].insertions, 5); // only from index.html
        assert_eq!(commits[0].deletions, 0);
    }

    // --- Tests for background checker helper methods ---

    #[test]
    fn test_has_pending_empty_cache() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, PathBuf::from("/tmp/mirrors"));

        assert!(!manager.has_pending(), "Empty cache should have no pending");
    }

    #[test]
    fn test_has_pending_with_workspace_ahead() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, PathBuf::from("/tmp/mirrors"));

        manager.cached_status.insert(
            "repo-1".to_string(),
            SyncStatus {
                workspace_ahead: 3,
                mirror_ahead: 0,
                remote_ahead: 0,
            },
        );

        assert!(manager.has_pending(), "Should be pending when workspace_ahead > 0");
    }

    #[test]
    fn test_has_pending_with_remote_ahead() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, PathBuf::from("/tmp/mirrors"));

        manager.cached_status.insert(
            "repo-1".to_string(),
            SyncStatus {
                workspace_ahead: 0,
                mirror_ahead: 0,
                remote_ahead: 2,
            },
        );

        assert!(manager.has_pending(), "Should be pending when remote_ahead > 0");
    }

    #[test]
    fn test_has_pending_all_synced() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, PathBuf::from("/tmp/mirrors"));

        manager.cached_status.insert(
            "repo-1".to_string(),
            SyncStatus {
                workspace_ahead: 0,
                mirror_ahead: 0,
                remote_ahead: 0,
            },
        );
        manager.cached_status.insert(
            "repo-2".to_string(),
            SyncStatus {
                workspace_ahead: 0,
                mirror_ahead: 0,
                remote_ahead: 0,
            },
        );

        assert!(!manager.has_pending(), "Should not be pending when all zeros");
    }

    #[test]
    fn test_get_cached_status_returns_all_entries() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, PathBuf::from("/tmp/mirrors"));

        manager.cached_status.insert(
            "repo-1".to_string(),
            SyncStatus {
                workspace_ahead: 1,
                mirror_ahead: 2,
                remote_ahead: 3,
            },
        );
        manager.cached_status.insert(
            "repo-2".to_string(),
            SyncStatus {
                workspace_ahead: 0,
                mirror_ahead: 0,
                remote_ahead: 5,
            },
        );

        let status_map = manager.get_cached_status();
        assert_eq!(status_map.len(), 2);
        assert_eq!(status_map["repo-1"].workspace_ahead, 1);
        assert_eq!(status_map["repo-1"].mirror_ahead, 2);
        assert_eq!(status_map["repo-1"].remote_ahead, 3);
        assert_eq!(status_map["repo-2"].remote_ahead, 5);
    }

    #[test]
    fn test_get_cached_status_empty() {
        let db = Arc::new(Database::open_in_memory().unwrap());
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db, git, PathBuf::from("/tmp/mirrors"));

        let status_map = manager.get_cached_status();
        assert!(status_map.is_empty());
    }

    // --- Tests for validate_branch_pattern ---

    #[test]
    fn test_validate_branch_pattern_valid() {
        assert!(RepoSyncManager::validate_branch_pattern("ai/<persona-name>/<date>").is_ok());
        assert!(RepoSyncManager::validate_branch_pattern("feature/my-branch").is_ok());
        assert!(RepoSyncManager::validate_branch_pattern("release-v1.0").is_ok());
        assert!(RepoSyncManager::validate_branch_pattern("a").is_ok());
    }

    #[test]
    fn test_validate_branch_pattern_empty() {
        assert!(RepoSyncManager::validate_branch_pattern("").is_err());
    }

    #[test]
    fn test_validate_branch_pattern_too_long() {
        let long_pattern = "a".repeat(201);
        assert!(RepoSyncManager::validate_branch_pattern(&long_pattern).is_err());
    }

    #[test]
    fn test_validate_branch_pattern_max_length_ok() {
        let pattern = "a".repeat(200);
        assert!(RepoSyncManager::validate_branch_pattern(&pattern).is_ok());
    }

    #[test]
    fn test_validate_branch_pattern_invalid_chars() {
        assert!(RepoSyncManager::validate_branch_pattern("branch name").is_err());
        assert!(RepoSyncManager::validate_branch_pattern("branch~name").is_err());
        assert!(RepoSyncManager::validate_branch_pattern("branch^name").is_err());
        assert!(RepoSyncManager::validate_branch_pattern("branch:name").is_err());
        assert!(RepoSyncManager::validate_branch_pattern("branch?name").is_err());
        assert!(RepoSyncManager::validate_branch_pattern("branch*name").is_err());
        assert!(RepoSyncManager::validate_branch_pattern("branch[name").is_err());
        assert!(RepoSyncManager::validate_branch_pattern("branch\\name").is_err());
    }

    #[test]
    fn test_validate_branch_pattern_control_chars() {
        assert!(RepoSyncManager::validate_branch_pattern("branch\x00name").is_err());
        assert!(RepoSyncManager::validate_branch_pattern("branch\tname").is_err());
        assert!(RepoSyncManager::validate_branch_pattern("branch\nname").is_err());
    }

    #[test]
    fn test_validate_branch_pattern_starts_with_dot_or_slash() {
        assert!(RepoSyncManager::validate_branch_pattern(".hidden").is_err());
        assert!(RepoSyncManager::validate_branch_pattern("/leading-slash").is_err());
    }

    #[test]
    fn test_validate_branch_pattern_ends_with_dot_or_slash() {
        assert!(RepoSyncManager::validate_branch_pattern("trailing.").is_err());
        assert!(RepoSyncManager::validate_branch_pattern("trailing/").is_err());
    }

    #[test]
    fn test_validate_branch_pattern_consecutive_dots() {
        assert!(RepoSyncManager::validate_branch_pattern("branch..name").is_err());
    }

    #[test]
    fn test_validate_branch_pattern_consecutive_slashes() {
        assert!(RepoSyncManager::validate_branch_pattern("branch//name").is_err());
    }

    // --- Tests for update_repo ---

    #[tokio::test]
    async fn test_update_repo_basic_fields() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable_agent_created(&PersonaId("p1".to_string()), workspace.path(), None)
            .await
            .unwrap();

        let req = UpdateRepoRequest {
            remote_url: None,
            remote_provider: None,
            branch_strategy: Some(BranchStrategy::FeatureBranch),
            branch_pattern: Some("feature/<persona-name>/<date>".to_string()),
            attribution_mode: Some(AttributionMode::CoAuthoredBy),
            sync_mode: None,
            secret_scan_mode: Some(SecretScanMode::WarnOnly),
            check_interval_seconds: Some(600),
        };

        let updated = manager.update_repo(&repo.id, &req).await.unwrap();

        assert_eq!(updated.branch_strategy, BranchStrategy::FeatureBranch);
        assert_eq!(
            updated.branch_pattern.as_deref(),
            Some("feature/<persona-name>/<date>")
        );
        assert_eq!(updated.attribution_mode, AttributionMode::CoAuthoredBy);
        assert_eq!(updated.secret_scan_mode, SecretScanMode::WarnOnly);
        assert_eq!(updated.check_interval_seconds, 600);
        assert_eq!(updated.sync_mode, SyncMode::LocalOnly);
    }

    #[tokio::test]
    async fn test_update_repo_invalid_url_rejected() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable_agent_created(&PersonaId("p1".to_string()), workspace.path(), None)
            .await
            .unwrap();

        let req = UpdateRepoRequest {
            remote_url: Some("not-a-valid-url".to_string()),
            remote_provider: None,
            branch_strategy: None,
            branch_pattern: None,
            attribution_mode: None,
            sync_mode: None,
            secret_scan_mode: None,
            check_interval_seconds: None,
        };

        let result = manager.update_repo(&repo.id, &req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_repo_invalid_branch_pattern_rejected() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable_agent_created(&PersonaId("p1".to_string()), workspace.path(), None)
            .await
            .unwrap();

        let req = UpdateRepoRequest {
            remote_url: None,
            remote_provider: None,
            branch_strategy: None,
            branch_pattern: Some("branch with spaces".to_string()),
            attribution_mode: None,
            sync_mode: None,
            secret_scan_mode: None,
            check_interval_seconds: None,
        };

        let result = manager.update_repo(&repo.id, &req).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_repo_sync_mode_to_remote_without_url_rejected() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable_agent_created(&PersonaId("p1".to_string()), workspace.path(), None)
            .await
            .unwrap();

        let req = UpdateRepoRequest {
            remote_url: None,
            remote_provider: None,
            branch_strategy: None,
            branch_pattern: None,
            attribution_mode: None,
            sync_mode: Some(SyncMode::Remote),
            secret_scan_mode: None,
            check_interval_seconds: None,
        };

        let result = manager.update_repo(&repo.id, &req).await;
        assert!(result.is_err());
        let err_msg = format!("{:?}", result.err().unwrap());
        assert!(
            err_msg.contains("remote URL"),
            "Error should mention remote URL: {}",
            err_msg
        );
    }

    #[tokio::test]
    async fn test_update_repo_valid_url_updates_provider() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable_agent_created(&PersonaId("p1".to_string()), workspace.path(), None)
            .await
            .unwrap();

        let req = UpdateRepoRequest {
            remote_url: Some("https://github.com/user/repo.git".to_string()),
            remote_provider: None,
            branch_strategy: None,
            branch_pattern: None,
            attribution_mode: None,
            sync_mode: None,
            secret_scan_mode: None,
            check_interval_seconds: None,
        };

        let updated = manager.update_repo(&repo.id, &req).await.unwrap();
        assert_eq!(
            updated.remote_url.as_deref(),
            Some("https://github.com/user/repo.git")
        );
        assert_eq!(updated.remote_provider, Some(RemoteProvider::Github));
    }

    #[tokio::test]
    async fn test_update_repo_not_found() {
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let req = UpdateRepoRequest {
            remote_url: None,
            remote_provider: None,
            branch_strategy: None,
            branch_pattern: None,
            attribution_mode: None,
            sync_mode: None,
            secret_scan_mode: None,
            check_interval_seconds: None,
        };

        let result = manager
            .update_repo(&ManagedRepoId("nonexistent".to_string()), &req)
            .await;
        assert!(result.is_err());
    }

    // --- Tests for delete_repo ---

    #[tokio::test]
    async fn test_delete_repo_removes_db_record() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable_agent_created(&PersonaId("p1".to_string()), workspace.path(), None)
            .await
            .unwrap();

        manager.delete_repo(&repo.id, false).await.unwrap();

        let result = db.with_conn(|conn| db_ops::get_managed_repo(conn, &repo.id));
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_repo_preserves_mirror_when_false() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable_agent_created(&PersonaId("p1".to_string()), workspace.path(), None)
            .await
            .unwrap();

        let mirror_path = PathBuf::from(&repo.mirror_path);
        assert!(mirror_path.exists());

        manager.delete_repo(&repo.id, false).await.unwrap();

        assert!(mirror_path.exists());
    }

    #[tokio::test]
    async fn test_delete_repo_removes_mirror_when_true() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable_agent_created(&PersonaId("p1".to_string()), workspace.path(), None)
            .await
            .unwrap();

        let mirror_path = PathBuf::from(&repo.mirror_path);
        assert!(mirror_path.exists());

        manager.delete_repo(&repo.id, true).await.unwrap();

        assert!(!mirror_path.exists());
    }

    #[tokio::test]
    async fn test_delete_repo_not_found() {
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let manager = RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let result = manager
            .delete_repo(&ManagedRepoId("nonexistent".to_string()), false)
            .await;
        assert!(result.is_err());
    }

    // --- Tests for update_mirrors_dir ---

    #[tokio::test]
    async fn test_update_mirrors_dir_creates_directory() {
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let mut manager =
            RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let new_dir = mirrors_dir.path().join("new-mirrors-location");
        assert!(!new_dir.exists());

        let result = manager.update_mirrors_dir(new_dir.to_str().unwrap());
        assert!(result.is_ok());
        assert!(new_dir.exists());
        assert_eq!(manager.mirrors_dir, new_dir);
    }

    #[tokio::test]
    async fn test_update_mirrors_dir_empty_path_rejected() {
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let mut manager =
            RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let result = manager.update_mirrors_dir("");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_mirrors_dir_relative_path_rejected() {
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let mut manager =
            RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let result = manager.update_mirrors_dir("relative/path");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_mirrors_dir_too_long_rejected() {
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let mut manager =
            RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let long_path = format!("/{}", "a".repeat(4096));
        let result = manager.update_mirrors_dir(&long_path);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_update_mirrors_dir_updates_repo_records() {
        let workspace = create_test_workspace();
        let mirrors_dir = tempfile::tempdir().unwrap();
        let db = setup_db_with_persona("p1", "my-agent");
        let git = Arc::new(GitCli::new("git".to_string()));
        let mut manager =
            RepoSyncManager::new(db.clone(), git.clone(), mirrors_dir.path().to_path_buf());

        let repo = manager
            .enable_agent_created(&PersonaId("p1".to_string()), workspace.path(), None)
            .await
            .unwrap();

        assert!(repo.mirror_path.starts_with(mirrors_dir.path().to_str().unwrap()));

        let new_dir = tempfile::tempdir().unwrap();
        let new_path = new_dir.path().join("new-mirrors");
        manager
            .update_mirrors_dir(new_path.to_str().unwrap())
            .unwrap();

        let updated_repo = db
            .with_conn(|conn| db_ops::get_managed_repo(conn, &repo.id))
            .unwrap();
        assert!(
            updated_repo
                .mirror_path
                .starts_with(new_path.to_str().unwrap()),
            "Updated mirror_path '{}' should start with new dir '{}'",
            updated_repo.mirror_path,
            new_path.display()
        );
    }
}
