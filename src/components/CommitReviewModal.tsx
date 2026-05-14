import { useState, useEffect, useRef, useCallback } from "react";
import { CommitInfo, pushToRemote, PushResult } from "../lib/api";

export interface CommitReviewModalProps {
  open: boolean;
  repoId: string;
  commits: CommitInfo[];
  onClose: () => void;
  onPushComplete: (result: PushResult) => void;
  onPushError: (error: string) => void;
}

export function CommitReviewModal({
  open,
  repoId,
  commits,
  onClose,
  onPushComplete,
  onPushError,
}: CommitReviewModalProps) {
  const [selectedShas, setSelectedShas] = useState<Set<string>>(new Set());
  const [squash, setSquash] = useState(false);
  const [squashMessage, setSquashMessage] = useState("");
  const [pushing, setPushing] = useState(false);
  const cancelBtnRef = useRef<HTMLButtonElement>(null);

  // Initialize selection when commits change
  useEffect(() => {
    if (open && commits.length > 0) {
      setSelectedShas(new Set(commits.map((c) => c.sha)));
      setSquash(false);
      setSquashMessage("");
    }
  }, [open, commits]);

  // Focus cancel button on open
  useEffect(() => {
    if (open && cancelBtnRef.current) {
      cancelBtnRef.current.focus();
    }
  }, [open]);

  // Close on Escape
  useEffect(() => {
    if (!open) return;
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape" && !pushing) {
        onClose();
      }
    }
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [open, pushing, onClose]);

  const toggleCommit = useCallback((sha: string) => {
    setSelectedShas((prev) => {
      const next = new Set(prev);
      if (next.has(sha)) {
        next.delete(sha);
      } else {
        next.add(sha);
      }
      return next;
    });
  }, []);

  const toggleAll = useCallback(() => {
    if (selectedShas.size === commits.length) {
      setSelectedShas(new Set());
    } else {
      setSelectedShas(new Set(commits.map((c) => c.sha)));
    }
  }, [selectedShas.size, commits]);

  const handlePush = async () => {
    if (selectedShas.size === 0) return;
    setPushing(true);
    try {
      const result = await pushToRemote(repoId, {
        commit_shas: commits
          .filter((c) => selectedShas.has(c.sha))
          .map((c) => c.sha),
        squash,
        squash_message: squash ? squashMessage || undefined : undefined,
      });
      onPushComplete(result);
    } catch (err) {
      const message = err instanceof Error ? err.message : "Push failed";
      onPushError(message);
    } finally {
      setPushing(false);
    }
  };

  if (!open) return null;

  const selectedCount = selectedShas.size;
  const canSquash = selectedCount >= 2;
  const canPush = selectedCount > 0 && !pushing;

  function handleBackdropClick(e: React.MouseEvent) {
    if (e.target === e.currentTarget && !pushing) {
      onClose();
    }
  }

  return (
    <div
      className="modal-overlay"
      onClick={handleBackdropClick}
      role="dialog"
      aria-modal="true"
      aria-labelledby="commit-review-title"
    >
      <div className="modal commit-review-modal">
        <h3 id="commit-review-title">Review Commits</h3>
        <p className="commit-review-summary">
          {commits.length} commit{commits.length !== 1 ? "s" : ""} to push
          {selectedCount < commits.length && (
            <span> ({selectedCount} selected)</span>
          )}
        </p>

        <div className="commit-review-controls">
          <label className="commit-review-select-all">
            <input
              type="checkbox"
              checked={selectedCount === commits.length}
              onChange={toggleAll}
              aria-label="Select all commits"
            />
            Select all
          </label>
          <label
            className={`commit-review-squash ${!canSquash ? "disabled" : ""}`}
          >
            <input
              type="checkbox"
              checked={squash}
              disabled={!canSquash}
              onChange={(e) => setSquash(e.target.checked)}
              aria-label="Squash selected commits"
            />
            Squash selected
          </label>
        </div>

        {squash && canSquash && (
          <div className="commit-review-squash-message">
            <input
              type="text"
              className="input"
              placeholder="Squash commit message (optional)"
              value={squashMessage}
              onChange={(e) => setSquashMessage(e.target.value)}
              aria-label="Squash commit message"
            />
          </div>
        )}

        <div className="commit-review-list">
          {commits.map((commit) => (
            <CommitItem
              key={commit.sha}
              commit={commit}
              selected={selectedShas.has(commit.sha)}
              onToggle={toggleCommit}
            />
          ))}
        </div>

        {selectedCount === 0 && (
          <p className="commit-review-warning">
            No commits selected. Select at least one commit to push.
          </p>
        )}

        <div className="modal-actions">
          <button
            className="btn"
            onClick={onClose}
            disabled={pushing}
            ref={cancelBtnRef}
            type="button"
          >
            Cancel
          </button>
          <button
            className="btn btn-primary"
            onClick={handlePush}
            disabled={!canPush}
            type="button"
          >
            {pushing ? "Pushing…" : "Push"}
          </button>
        </div>
      </div>
    </div>
  );
}

function CommitItem({
  commit,
  selected,
  onToggle,
}: {
  commit: CommitInfo;
  selected: boolean;
  onToggle: (sha: string) => void;
}) {
  const shortSha = commit.sha.substring(0, 7);
  const formattedDate = formatTimestamp(commit.timestamp);

  return (
    <div className={`commit-item ${selected ? "commit-item--selected" : ""}`}>
      <label className="commit-item-checkbox">
        <input
          type="checkbox"
          checked={selected}
          onChange={() => onToggle(commit.sha)}
          aria-label={`Select commit ${shortSha}`}
        />
      </label>
      <div className="commit-item-content">
        <div className="commit-item-header">
          <code className="commit-item-sha">{shortSha}</code>
          <span className="commit-item-message">{commit.message}</span>
        </div>
        <div className="commit-item-meta">
          <span className="commit-item-author">{commit.author}</span>
          <span className="commit-item-date">{formattedDate}</span>
          <span className="commit-item-stats">
            {commit.files_changed} file{commit.files_changed !== 1 ? "s" : ""}
            {commit.insertions > 0 && (
              <span className="commit-item-additions">
                +{commit.insertions}
              </span>
            )}
            {commit.deletions > 0 && (
              <span className="commit-item-deletions">
                −{commit.deletions}
              </span>
            )}
          </span>
        </div>
      </div>
    </div>
  );
}

function formatTimestamp(timestamp: string): string {
  try {
    const date = new Date(timestamp);
    if (isNaN(date.getTime())) return timestamp;
    return date.toLocaleString(undefined, {
      year: "numeric",
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  } catch {
    return timestamp;
  }
}
