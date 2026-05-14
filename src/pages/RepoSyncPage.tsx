import { useState, useEffect, useCallback } from "react";
import { getRepos, ManagedRepoResponse } from "../lib/api";
import { usePolling } from "../hooks/usePolling";

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

export function RepoSyncPage() {
  const [pageVisible, setPageVisible] = useState(!document.hidden);

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

  const { data, error, loading } = usePolling<ManagedRepoResponse[]>(
    fetchFn,
    10000,
    pageVisible,
  );

  const repos = data ?? [];
  const groups = groupReposByPersona(repos);

  return (
    <div className="repo-sync-page">
      <div className="page-header">
        <h2>Repo Sync</h2>
      </div>

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
                  <RepoCard key={repo.id} repo={repo} />
                ))}
              </div>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function RepoCard({ repo }: { repo: ManagedRepoResponse }) {
  const projectName = folderName(repo.workspace_path);

  return (
    <div className="card repo-sync-card">
      <div className="card-header">
        <h4 className="card-title">{projectName}</h4>
        <span className="badge badge-sync-mode">
          {formatSyncMode(repo.sync_mode)}
        </span>
      </div>
      <div className="card-body">
        {repo.remote_url && (
          <p className="card-description">{repo.remote_url}</p>
        )}
        <div className="repo-sync-status">
          <SyncStatusIndicators status={repo.sync_status} syncMode={repo.sync_mode} />
        </div>
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
