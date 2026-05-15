import { useState, useEffect, useCallback } from "react";
import {
  getRepos,
  scanWorkspaces,
  enableRepo,
  getCommits,
  pullFromAgent,
  fetchFromRemote,
  pushToAgent,
  getMirrorsDir,
  setMirrorsDir,
  ManagedRepoResponse,
  DetectedRepo,
  CommitInfo,
} from "../lib/api";
import { usePolling } from "../hooks/usePolling";
import { CommitReviewModal } from "../components/CommitReviewModal";
import { RepoSettingsPanel } from "../components/RepoSettingsPanel";
import {
  SecretScanWarningModal,
  SecretScanFinding,
  parseSecretScanError,
} from "../components/SecretScanWarningModal";

/**
 * Groups repos by persona name, with repos sorted alphabetically within each group.
 * Groups themselves are sorted alphabetically by persona name.
 */
function groupReposByPersona(
  repos: ManagedRepoResponse[],
): { personaName: string; repos: ManagedRepoResponse[] }[] {
  const grouped = new Map<string, ManagedRepoResponse[]>();

  for (const repo of repos) {
    const existing = grouped.get(repo.persona_name);
    if (existing) {
      existing.push(repo);
    } else {
      grouped.set(repo.persona_name, [repo]);
    }
  }

  const result: { personaName: string; repos: ManagedRepoResponse[] }[] = [];
  for (const [personaName, personaRepos] of grouped) {
    personaRepos.sort((a, b) => {
      const nameA = folderName(a.workspace_path);
      const nameB = folderName(b.workspace_path);
      return nameA.localeCompare(nameB);
    });
    result.push({ personaName, repos: personaRepos });
  }

  result.sort((a, b) => a.personaName.localeCompare(b.personaName));
  return result;
}

/** Extracts the last path segment as the project folder name. */
function folderName(workspacePath: string): string {
  const parts = workspacePath.replace(/\\/g, "/").split("/");
  return parts[parts.length - 1] ?? workspacePath;
}

/** Formats sync mode for display. */
function formatSyncMode(mode: string): string {
  switch (mode) {
    case "local_only":
      return "Local only";
    case "remote":
      return "Remote";
    default:
      return mode;
  }
}

type SyncOperation = "pull_from_agent" | "push_to_remote" | "fetch_from_remote" | "push_to_agent";

interface OperationState {
  repoId: string;
  operation: SyncOperation;
}


export function RepoSyncPage() {
  const [pageVisible, setPageVisible] = useState(!document.hidden);
  const [scanning, setScanning] = useState(false);
  const [scanResults, setScanResults] = useState<DetectedRepo[] | null>(null);
  const [scanError, setScanError] = useState<string | null>(null);
  const [enablingRepos, setEnablingRepos] = useState<Set<string>>(new Set());
  const [enableError, setEnableError] = useState<string | null>(null);
  const [linkRemoteTarget, setLinkRemoteTarget] = useState<DetectedRepo | null>(null);
  const [linkRemoteUrl, setLinkRemoteUrl] = useState("");
  const [activeOperation, setActiveOperation] = useState<OperationState | null>(null);
  const [syncError, setSyncError] = useState<string | null>(null);
  const [commitReviewOpen, setCommitReviewOpen] = useState(false);
  const [commitReviewRepoId, setCommitReviewRepoId] = useState<string | null>(null);
  const [commitReviewCommits, setCommitReviewCommits] = useState<CommitInfo[]>([]);
  const [secretScanFindings, setSecretScanFindings] = useState<SecretScanFinding[]>([]);
  const [secretScanWarningOpen, setSecretScanWarningOpen] = useState(false);

  useEffect(() => {
    const handleVisibility = () => {
      setPageVisible(!document.hidden);
    };
    document.addEventListener("visibilitychange", handleVisibility);
    return () => {
      document.removeEventListener("visibilitychange", handleVisibility);
    };
  }, []);

  const fetchFn = useCallback(() => getRepos(), []);

  const { data, error, loading, refresh } = usePolling<ManagedRepoResponse[]>(
    fetchFn,
    10000,
    pageVisible,
  );

  const repos = data ?? [];
  const groups = groupReposByPersona(repos);

  const handleScan = async () => {
    setScanning(true);
    setScanError(null);
    setScanResults(null);
    try {
      const results = await scanWorkspaces();
      setScanResults(results);
    } catch (err) {
      const message = err instanceof Error ? err.message : "Scan failed";
      setScanError(message);
    } finally {
      setScanning(false);
    }
  };

  const handleEnableRepo = async (repo: DetectedRepo) => {
    const key = `${repo.persona_id}:${repo.workspace_path}`;
    setEnablingRepos((prev) => new Set(prev).add(key));
    setEnableError(null);
    try {
      await enableRepo({
        persona_id: repo.persona_id,
        workspace_path: repo.workspace_path,
      });
      // Remove from scan results after successful enable
      setScanResults((prev) =>
        prev ? prev.filter((r) => r.workspace_path !== repo.workspace_path || r.persona_id !== repo.persona_id) : null,
      );
      refresh();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to enable repo";
      setEnableError(message);
    } finally {
      setEnablingRepos((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  };

  const handleKeepLocal = async (repo: DetectedRepo) => {
    const key = `${repo.persona_id}:${repo.workspace_path}`;
    setEnablingRepos((prev) => new Set(prev).add(key));
    setEnableError(null);
    try {
      await enableRepo({
        persona_id: repo.persona_id,
        workspace_path: repo.workspace_path,
      });
      setScanResults((prev) =>
        prev ? prev.filter((r) => r.workspace_path !== repo.workspace_path || r.persona_id !== repo.persona_id) : null,
      );
      refresh();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to enable repo";
      setEnableError(message);
    } finally {
      setEnablingRepos((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  };

  const handleLinkToRemote = (repo: DetectedRepo) => {
    setLinkRemoteTarget(repo);
    setLinkRemoteUrl("");
  };

  const handleLinkRemoteConfirm = async () => {
    if (!linkRemoteTarget || !linkRemoteUrl.trim()) return;
    const key = `${linkRemoteTarget.persona_id}:${linkRemoteTarget.workspace_path}`;
    setEnablingRepos((prev) => new Set(prev).add(key));
    setEnableError(null);
    setLinkRemoteTarget(null);
    try {
      await enableRepo({
        persona_id: linkRemoteTarget.persona_id,
        workspace_path: linkRemoteTarget.workspace_path,
        remote_url: linkRemoteUrl.trim(),
      });
      setScanResults((prev) =>
        prev ? prev.filter((r) => r.workspace_path !== linkRemoteTarget.workspace_path || r.persona_id !== linkRemoteTarget.persona_id) : null,
      );
      refresh();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to link repo";
      setEnableError(message);
    } finally {
      setEnablingRepos((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }
  };

  const handleLinkRemoteCancel = () => {
    setLinkRemoteTarget(null);
    setLinkRemoteUrl("");
  };

  const handleSyncOperation = async (repoId: string, operation: SyncOperation) => {
    if (operation === "push_to_remote") {
      // Fetch commits and open review modal instead of pushing directly
      setActiveOperation({ repoId, operation });
      setSyncError(null);
      try {
        const commits = await getCommits(repoId);
        if (commits.length === 0) {
          setSyncError("No commits to push.");
        } else {
          setCommitReviewRepoId(repoId);
          setCommitReviewCommits(commits);
          setCommitReviewOpen(true);
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : "Failed to fetch commits";
        setSyncError(message);
      } finally {
        setActiveOperation(null);
      }
      return;
    }

    setActiveOperation({ repoId, operation });
    setSyncError(null);
    try {
      switch (operation) {
        case "pull_from_agent":
          await pullFromAgent(repoId);
          break;
        case "fetch_from_remote":
          await fetchFromRemote(repoId);
          break;
        case "push_to_agent":
          await pushToAgent(repoId);
          break;
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : "Sync operation failed";
      setSyncError(message);
    } finally {
      refresh();
      setActiveOperation(null);
    }
  };

  const handleCommitReviewClose = () => {
    setCommitReviewOpen(false);
    setCommitReviewRepoId(null);
    setCommitReviewCommits([]);
  };

  const handlePushComplete = () => {
    setCommitReviewOpen(false);
    setCommitReviewRepoId(null);
    setCommitReviewCommits([]);
    refresh();
  };

  const handlePushError = (message: string) => {
    setCommitReviewOpen(false);
    setCommitReviewRepoId(null);
    setCommitReviewCommits([]);

    // Check if this is a secret scan rejection
    const findings = parseSecretScanError(message);
    if (findings) {
      setSecretScanFindings(findings);
      setSecretScanWarningOpen(true);
    } else {
      setSyncError(message);
    }
    refresh();
  };

  return (
    <div className="repo-sync-page">
      <div className="page-header">
        <h2>Repo Sync</h2>
        <button
          className="btn btn-primary"
          onClick={handleScan}
          disabled={scanning}
          aria-label="Scan Workspace"
        >
          {scanning ? "Scanning…" : "Scan Workspace"}
        </button>
      </div>

      <MirrorsDirectorySettings />

      {scanError && (
        <div className="alert alert-error" role="alert">
          {scanError}
          <button
            className="btn btn-sm"
            onClick={() => setScanError(null)}
            aria-label="Dismiss scan error"
          >
            ✕
          </button>
        </div>
      )}

      {enableError && (
        <div className="alert alert-error" role="alert">
          {enableError}
          <button
            className="btn btn-sm"
            onClick={() => setEnableError(null)}
            aria-label="Dismiss enable error"
          >
            ✕
          </button>
        </div>
      )}

      {syncError && (
        <div className="alert alert-error" role="alert">
          {syncError}
          <button
            className="btn btn-sm"
            onClick={() => setSyncError(null)}
            aria-label="Dismiss sync error"
          >
            ✕
          </button>
        </div>
      )}

      {scanResults !== null && (
        <div className="scan-results">
          {scanResults.length === 0 ? (
            <p className="scan-results-empty">No new repositories found.</p>
          ) : (
            <>
              <h3 className="scan-results-title">Detected Repositories</h3>
              <div className="scan-results-list">
                {scanResults.map((repo) => {
                  const key = `${repo.persona_id}:${repo.workspace_path}`;
                  const isEnabling = enablingRepos.has(key);
                  return (
                    <DetectedRepoCard
                      key={key}
                      repo={repo}
                      isEnabling={isEnabling}
                      onEnable={handleEnableRepo}
                      onLinkToRemote={handleLinkToRemote}
                      onKeepLocal={handleKeepLocal}
                    />
                  );
                })}
              </div>
            </>
          )}
        </div>
      )}

      {linkRemoteTarget && (
        <div className="modal-backdrop" onClick={handleLinkRemoteCancel}>
          <div
            className="modal"
            role="dialog"
            aria-label="Link to remote"
            onClick={(e) => e.stopPropagation()}
          >
            <h3>Link to Remote</h3>
            <p>
              Provide a remote URL for{" "}
              <strong>{folderName(linkRemoteTarget.workspace_path)}</strong>
            </p>
            <input
              type="text"
              className="input"
              placeholder="https://github.com/user/repo.git"
              value={linkRemoteUrl}
              onChange={(e) => setLinkRemoteUrl(e.target.value)}
              aria-label="Remote URL"
              autoFocus
            />
            <div className="modal-actions">
              <button
                className="btn"
                onClick={handleLinkRemoteCancel}
              >
                Cancel
              </button>
              <button
                className="btn btn-primary"
                onClick={handleLinkRemoteConfirm}
                disabled={!linkRemoteUrl.trim()}
              >
                Link
              </button>
            </div>
          </div>
        </div>
      )}

      {loading && (
        <p className="loading-indicator">Loading repositories…</p>
      )}

      {error && !data && !loading && (
        <div className="alert alert-error" role="alert">
          {error.message}
        </div>
      )}

      {!loading && !error && repos.length === 0 && (
        <div className="empty-state">
          <p>
            Repo Sync is not enabled for any repositories. Use the "Scan
            Workspace" button to detect git repositories in your persona
            workspaces and enable synchronization.
          </p>
        </div>
      )}

      {groups.length > 0 && (
        <div className="repo-sync-groups">
          {groups.map((group) => (
            <div key={group.personaName} className="repo-sync-group">
              <h3 className="repo-sync-group-title">{group.personaName}</h3>
              <div className="repo-sync-repo-list">
                {group.repos.map((repo) => (
                  <RepoCard
                    key={repo.id}
                    repo={repo}
                    activeOperation={activeOperation}
                    onSyncOperation={handleSyncOperation}
                    onSettingsSaved={refresh}
                  />
                ))}
              </div>
            </div>
          ))}
        </div>
      )}

      {commitReviewRepoId && (
        <CommitReviewModal
          open={commitReviewOpen}
          repoId={commitReviewRepoId}
          commits={commitReviewCommits}
          onClose={handleCommitReviewClose}
          onPushComplete={handlePushComplete}
          onPushError={handlePushError}
        />
      )}

      <SecretScanWarningModal
        open={secretScanWarningOpen}
        findings={secretScanFindings}
        onDismiss={() => setSecretScanWarningOpen(false)}
      />
    </div>
  );
}

function DetectedRepoCard({
  repo,
  isEnabling,
  onEnable,
  onLinkToRemote,
  onKeepLocal,
}: {
  repo: DetectedRepo;
  isEnabling: boolean;
  onEnable: (repo: DetectedRepo) => void;
  onLinkToRemote: (repo: DetectedRepo) => void;
  onKeepLocal: (repo: DetectedRepo) => void;
}) {
  const projectName = folderName(repo.workspace_path);

  return (
    <div className="card repo-sync-card detected-repo-card">
      <div className="card-header">
        <h4 className="card-title">{projectName}</h4>
        <span className="badge badge-persona">{repo.persona_name}</span>
      </div>
      <div className="card-body">
        <p className="card-description">{repo.workspace_path}</p>
        {repo.remote_url && (
          <p className="card-description">Remote: {repo.remote_url}</p>
        )}
        <div className="detected-repo-actions">
          {repo.has_remotes ? (
            <button
              className="btn btn-primary btn-sm"
              disabled={isEnabling}
              onClick={() => onEnable(repo)}
              aria-label={`Enable Repo Sync for ${projectName}`}
            >
              {isEnabling ? "Enabling…" : "Enable Repo Sync"}
            </button>
          ) : (
            <>
              <button
                className="btn btn-primary btn-sm"
                disabled={isEnabling}
                onClick={() => onLinkToRemote(repo)}
                aria-label={`Link ${projectName} to remote`}
              >
                Link to remote
              </button>
              <button
                className="btn btn-sm"
                disabled={isEnabling}
                onClick={() => onKeepLocal(repo)}
                aria-label={`Keep ${projectName} local only`}
              >
                {isEnabling ? "Enabling…" : "Keep local only"}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

function RepoCard({
  repo,
  activeOperation,
  onSyncOperation,
  onSettingsSaved,
}: {
  repo: ManagedRepoResponse;
  activeOperation: OperationState | null;
  onSyncOperation: (repoId: string, operation: SyncOperation) => void;
  onSettingsSaved: () => void;
}) {
  const [settingsOpen, setSettingsOpen] = useState(false);
  const projectName = folderName(repo.workspace_path);
  const isLocalOnly = repo.sync_mode === "local_only";
  const isOperationInProgress = activeOperation?.repoId === repo.id;
  const currentOp = isOperationInProgress ? activeOperation.operation : null;

  return (
    <div className="card repo-sync-card">
      <div className="card-header">
        <h4 className="card-title">{projectName}</h4>
        <span className="badge badge-sync-mode">
          {formatSyncMode(repo.sync_mode)}
        </span>
        <button
          className="btn btn-sm repo-settings-toggle"
          onClick={() => setSettingsOpen((prev) => !prev)}
          aria-expanded={settingsOpen}
          aria-label={`${settingsOpen ? "Hide" : "Show"} settings for ${projectName}`}
          type="button"
        >
          {settingsOpen ? "Hide Settings" : "Settings"}
        </button>
      </div>
      <div className="card-body">
        {repo.remote_url && (
          <p className="card-description">{repo.remote_url}</p>
        )}
        <div className="repo-sync-status">
          <SyncStatusIndicators status={repo.sync_status} syncMode={repo.sync_mode} />
        </div>
        <div className="repo-sync-buttons">
          <button
            className="btn btn-sm"
            disabled={isOperationInProgress}
            onClick={() => onSyncOperation(repo.id, "pull_from_agent")}
            aria-label={`Pull from agent for ${projectName}`}
          >
            {currentOp === "pull_from_agent" ? "Pulling…" : "Pull from agent"}
          </button>
          <button
            className="btn btn-sm"
            disabled={isOperationInProgress || isLocalOnly}
            onClick={() => onSyncOperation(repo.id, "push_to_remote")}
            aria-label={`Push to remote for ${projectName}`}
          >
            {currentOp === "push_to_remote" ? "Pushing…" : "Push to remote"}
          </button>
          <button
            className="btn btn-sm"
            disabled={isOperationInProgress || isLocalOnly}
            onClick={() => onSyncOperation(repo.id, "fetch_from_remote")}
            aria-label={`Fetch from remote for ${projectName}`}
          >
            {currentOp === "fetch_from_remote" ? "Fetching…" : "Fetch from remote"}
          </button>
          <button
            className="btn btn-sm"
            disabled={isOperationInProgress}
            onClick={() => onSyncOperation(repo.id, "push_to_agent")}
            aria-label={`Push to agent for ${projectName}`}
          >
            {currentOp === "push_to_agent" ? "Pushing…" : "Push to agent"}
          </button>
        </div>
        {settingsOpen && (
          <RepoSettingsPanel repo={repo} onSaved={onSettingsSaved} />
        )}
      </div>
    </div>
  );
}

function SyncStatusIndicators({
  status,
  syncMode,
}: {
  status: ManagedRepoResponse["sync_status"];
  syncMode: string;
}) {
  return (
    <div className="sync-indicators">
      <span className="sync-indicator" title="Workspace → Mirror: commits ahead">
        <span className="sync-indicator-label">Workspace→Mirror</span>
        <span
          className={`sync-indicator-value ${status.workspace_ahead > 0 ? "sync-indicator-pending" : ""}`}
        >
          {status.workspace_ahead > 0
            ? `${status.workspace_ahead} ahead`
            : "in sync"}
        </span>
      </span>
      {syncMode === "remote" && (
        <span className="sync-indicator" title="Mirror → Remote: commits ahead/behind">
          <span className="sync-indicator-label">Mirror→Remote</span>
          <span
            className={`sync-indicator-value ${status.mirror_ahead > 0 || status.remote_ahead > 0 ? "sync-indicator-pending" : ""}`}
          >
            {status.mirror_ahead > 0 && `${status.mirror_ahead} ahead`}
            {status.mirror_ahead > 0 && status.remote_ahead > 0 && ", "}
            {status.remote_ahead > 0 && `${status.remote_ahead} behind`}
            {status.mirror_ahead === 0 && status.remote_ahead === 0 && "in sync"}
          </span>
        </span>
      )}
    </div>
  );
}

function MirrorsDirectorySettings() {
  const [expanded, setExpanded] = useState(false);
  const [currentPath, setCurrentPath] = useState<string | null>(null);
  const [editPath, setEditPath] = useState("");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState(false);

  useEffect(() => {
    getMirrorsDir()
      .then((res) => {
        setCurrentPath(res.path);
        setEditPath(res.path);
      })
      .catch(() => {
        // Non-critical — mirrors dir just won't display
      });
  }, []);

  const handleSave = async () => {
    if (!editPath.trim()) return;
    setSaving(true);
    setError(null);
    setSuccess(false);
    try {
      const res = await setMirrorsDir(editPath.trim());
      setCurrentPath(res.path);
      setEditPath(res.path);
      setSuccess(true);
      setTimeout(() => setSuccess(false), 3000);
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to update mirrors directory";
      setError(message);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="mirrors-dir-settings">
      <button
        className="btn btn-sm mirrors-dir-toggle"
        onClick={() => setExpanded((prev) => !prev)}
        aria-expanded={expanded}
        type="button"
      >
        {expanded ? "▾" : "▸"} Mirrors Directory
        {currentPath && !expanded && (
          <span className="mirrors-dir-current">{currentPath}</span>
        )}
      </button>
      {expanded && (
        <div className="mirrors-dir-form">
          <p className="mirrors-dir-description">
            Mirror repositories are stored in this directory. Each mirror is at{" "}
            <code>&lt;mirrors-dir&gt;/&lt;persona&gt;/&lt;project&gt;/</code>.
          </p>
          <div className="form-group">
            <label htmlFor="mirrors-dir-input">Path</label>
            <input
              id="mirrors-dir-input"
              type="text"
              className="input"
              value={editPath}
              onChange={(e) => {
                setEditPath(e.target.value);
                setError(null);
                setSuccess(false);
              }}
              placeholder="/home/user/.local/share/beachead/mirrors"
              aria-invalid={!!error}
            />
          </div>
          {error && (
            <span className="field-error" role="alert">{error}</span>
          )}
          {success && (
            <span className="field-success">Saved.</span>
          )}
          <button
            className="btn btn-primary btn-sm"
            onClick={handleSave}
            disabled={saving || !editPath.trim() || editPath === currentPath}
            type="button"
          >
            {saving ? "Saving…" : "Update"}
          </button>
        </div>
      )}
    </div>
  );
}
