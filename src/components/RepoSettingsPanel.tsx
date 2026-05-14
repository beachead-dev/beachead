import { useState, useCallback } from "react";
import {
  ManagedRepoResponse,
  UpdateRepoRequest,
  SetCredentialsRequest,
  updateRepo,
  setCredentials,
  deleteCredentials,
} from "../lib/api";

export interface RepoSettingsPanelProps {
  repo: ManagedRepoResponse;
  onSaved: () => void;
}

interface FormState {
  remote_url: string;
  remote_provider: string;
  branch_strategy: string;
  branch_pattern: string;
  attribution_mode: string;
  sync_mode: string;
  secret_scan_mode: string;
}

interface CredentialFormState {
  username: string;
  secret: string;
  credential_type: "token" | "username_password";
}

function buildFormState(repo: ManagedRepoResponse): FormState {
  return {
    remote_url: repo.remote_url ?? "",
    remote_provider: repo.remote_provider ?? "github",
    branch_strategy: repo.branch_strategy,
    branch_pattern: repo.branch_pattern ?? "ai/<persona>/<date>",
    attribution_mode: repo.attribution_mode,
    sync_mode: repo.sync_mode,
    secret_scan_mode: repo.secret_scan_mode,
  };
}

function validateRemoteUrl(url: string): string | null {
  if (!url.trim()) return null; // empty is allowed (clears the URL)
  if (url.length > 2048) return "URL must be 2048 characters or fewer.";
  const httpsPattern = /^https:\/\/.+\/.+/;
  const sshPattern = /^git@.+:.+/;
  if (!httpsPattern.test(url) && !sshPattern.test(url)) {
    return "URL must be HTTPS (https://host/path) or SSH (git@host:path).";
  }
  return null;
}

function validateBranchPattern(pattern: string): string | null {
  if (!pattern.trim()) return "Branch pattern cannot be empty.";
  if (pattern.length > 200) return "Branch pattern must be 200 characters or fewer.";
  // Check for invalid git branch name characters
  const invalidChars = /[~^:?*\[\\]/;
  if (invalidChars.test(pattern.replace(/<[^>]+>/g, ""))) {
    return "Branch pattern contains invalid git branch name characters.";
  }
  return null;
}

export function RepoSettingsPanel({ repo, onSaved }: RepoSettingsPanelProps) {
  const [form, setForm] = useState<FormState>(() => buildFormState(repo));
  const [credForm, setCredForm] = useState<CredentialFormState>({
    username: "",
    secret: "",
    credential_type: "token",
  });
  const [saving, setSaving] = useState(false);
  const [savingCreds, setSavingCreds] = useState(false);
  const [removingCreds, setRemovingCreds] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [credError, setCredError] = useState<string | null>(null);
  const [validationErrors, setValidationErrors] = useState<Record<string, string>>({});

  const handleFieldChange = useCallback(
    (field: keyof FormState, value: string) => {
      setForm((prev) => ({ ...prev, [field]: value }));
      setValidationErrors((prev) => {
        const next = { ...prev };
        delete next[field];
        return next;
      });
    },
    [],
  );

  const handleCredFieldChange = useCallback(
    (field: keyof CredentialFormState, value: string) => {
      setCredForm((prev) => ({ ...prev, [field]: value }));
      setCredError(null);
    },
    [],
  );

  const handleSaveSettings = async () => {
    setError(null);
    const errors: Record<string, string> = {};

    // Validate remote URL
    if (form.remote_url) {
      const urlErr = validateRemoteUrl(form.remote_url);
      if (urlErr) errors.remote_url = urlErr;
    }

    // Validate branch pattern when strategy is feature_branch
    if (form.branch_strategy === "feature_branch") {
      const patternErr = validateBranchPattern(form.branch_pattern);
      if (patternErr) errors.branch_pattern = patternErr;
    }

    if (Object.keys(errors).length > 0) {
      setValidationErrors(errors);
      return;
    }

    setSaving(true);
    try {
      const req: UpdateRepoRequest = {
        remote_url: form.remote_url || undefined,
        remote_provider: form.remote_provider,
        branch_strategy: form.branch_strategy,
        branch_pattern:
          form.branch_strategy === "feature_branch"
            ? form.branch_pattern
            : undefined,
        attribution_mode: form.attribution_mode,
        sync_mode: form.sync_mode,
        secret_scan_mode: form.secret_scan_mode,
      };
      await updateRepo(repo.id, req);
      onSaved();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to save settings";
      setError(message);
    } finally {
      setSaving(false);
    }
  };

  const handleSaveCredentials = async () => {
    setCredError(null);
    if (!credForm.username.trim()) {
      setCredError("Username is required.");
      return;
    }
    if (!credForm.secret.trim()) {
      setCredError("Token/password is required.");
      return;
    }

    setSavingCreds(true);
    try {
      const req: SetCredentialsRequest = {
        username: credForm.username.trim(),
        secret: credForm.secret.trim(),
        credential_type: credForm.credential_type,
      };
      await setCredentials(repo.id, req);
      setCredForm({ username: "", secret: "", credential_type: credForm.credential_type });
      onSaved();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to save credentials";
      setCredError(message);
    } finally {
      setSavingCreds(false);
    }
  };

  const handleRemoveCredentials = async () => {
    setCredError(null);
    setRemovingCreds(true);
    try {
      await deleteCredentials(repo.id);
      onSaved();
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to remove credentials";
      setCredError(message);
    } finally {
      setRemovingCreds(false);
    }
  };

  return (
    <div className="repo-settings-panel">
      <div className="repo-settings-section">
        <h5 className="repo-settings-section-title">Repository Settings</h5>

        <div className="form-group">
          <label htmlFor={`remote-url-${repo.id}`}>Remote URL</label>
          <input
            id={`remote-url-${repo.id}`}
            type="text"
            value={form.remote_url}
            onChange={(e) => handleFieldChange("remote_url", e.target.value)}
            placeholder="https://github.com/user/repo.git"
            aria-invalid={!!validationErrors.remote_url}
          />
          {validationErrors.remote_url && (
            <span className="field-error" role="alert">
              {validationErrors.remote_url}
            </span>
          )}
        </div>

        <div className="form-group">
          <label htmlFor={`provider-${repo.id}`}>Provider</label>
          <select
            id={`provider-${repo.id}`}
            value={form.remote_provider}
            onChange={(e) => handleFieldChange("remote_provider", e.target.value)}
          >
            <option value="github">GitHub</option>
            <option value="gitlab">GitLab</option>
            <option value="bitbucket">Bitbucket</option>
            <option value="custom">Custom</option>
          </select>
        </div>

        <div className="form-group">
          <label htmlFor={`branch-strategy-${repo.id}`}>Branch Strategy</label>
          <select
            id={`branch-strategy-${repo.id}`}
            value={form.branch_strategy}
            onChange={(e) => handleFieldChange("branch_strategy", e.target.value)}
          >
            <option value="direct">Direct push</option>
            <option value="feature_branch">Auto-create feature branch</option>
          </select>
        </div>

        {form.branch_strategy === "feature_branch" && (
          <div className="form-group">
            <label htmlFor={`branch-pattern-${repo.id}`}>Branch Pattern</label>
            <input
              id={`branch-pattern-${repo.id}`}
              type="text"
              value={form.branch_pattern}
              onChange={(e) => handleFieldChange("branch_pattern", e.target.value)}
              placeholder="ai/<persona>/<date>"
              aria-invalid={!!validationErrors.branch_pattern}
            />
            {validationErrors.branch_pattern && (
              <span className="field-error" role="alert">
                {validationErrors.branch_pattern}
              </span>
            )}
          </div>
        )}

        <div className="form-group">
          <label htmlFor={`attribution-${repo.id}`}>Attribution Mode</label>
          <select
            id={`attribution-${repo.id}`}
            value={form.attribution_mode}
            onChange={(e) => handleFieldChange("attribution_mode", e.target.value)}
          >
            <option value="keep_agent">Keep agent&apos;s author</option>
            <option value="rewrite_user">Rewrite to user</option>
            <option value="co_authored_by">Add Co-authored-by</option>
          </select>
        </div>

        <div className="form-group">
          <label htmlFor={`sync-mode-${repo.id}`}>Sync Mode</label>
          <select
            id={`sync-mode-${repo.id}`}
            value={form.sync_mode}
            onChange={(e) => handleFieldChange("sync_mode", e.target.value)}
          >
            <option value="remote">Remote</option>
            <option value="local_only">Local only</option>
          </select>
        </div>

        <div className="form-group">
          <label htmlFor={`secret-scan-${repo.id}`}>Secret Scan Mode</label>
          <select
            id={`secret-scan-${repo.id}`}
            value={form.secret_scan_mode}
            onChange={(e) => handleFieldChange("secret_scan_mode", e.target.value)}
          >
            <option value="block">Block</option>
            <option value="warn_only">Warn only</option>
          </select>
        </div>

        {error && (
          <div className="field-error" role="alert">
            {error}
          </div>
        )}

        <button
          className="btn btn-primary btn-sm"
          onClick={handleSaveSettings}
          disabled={saving}
          type="button"
        >
          {saving ? "Saving…" : "Save Settings"}
        </button>
      </div>

      <div className="repo-settings-section">
        <h5 className="repo-settings-section-title">Credentials</h5>
        <p className="repo-settings-credential-status">
          Status:{" "}
          <span
            className={
              repo.credential_status === "configured"
                ? "credential-configured"
                : "credential-not-configured"
            }
          >
            {repo.credential_status === "configured" ? "Configured" : "Not configured"}
          </span>
        </p>

        <div className="form-group">
          <label htmlFor={`cred-username-${repo.id}`}>Username</label>
          <input
            id={`cred-username-${repo.id}`}
            type="text"
            value={credForm.username}
            onChange={(e) => handleCredFieldChange("username", e.target.value)}
            placeholder="Username"
            autoComplete="off"
          />
        </div>

        <div className="form-group">
          <label htmlFor={`cred-secret-${repo.id}`}>Token / Password</label>
          <input
            id={`cred-secret-${repo.id}`}
            type="password"
            value={credForm.secret}
            onChange={(e) => handleCredFieldChange("secret", e.target.value)}
            placeholder="Token or password"
            autoComplete="off"
          />
        </div>

        <div className="form-group">
          <label htmlFor={`cred-type-${repo.id}`}>Credential Type</label>
          <select
            id={`cred-type-${repo.id}`}
            value={credForm.credential_type}
            onChange={(e) =>
              handleCredFieldChange("credential_type", e.target.value)
            }
          >
            <option value="token">Token</option>
            <option value="username_password">Username &amp; Password</option>
          </select>
        </div>

        {credError && (
          <div className="field-error" role="alert">
            {credError}
          </div>
        )}

        <div className="repo-settings-cred-actions">
          <button
            className="btn btn-primary btn-sm"
            onClick={handleSaveCredentials}
            disabled={savingCreds || removingCreds}
            type="button"
          >
            {savingCreds ? "Saving…" : "Save Credentials"}
          </button>
          {repo.credential_status === "configured" && (
            <button
              className="btn btn-sm btn-danger"
              onClick={handleRemoveCredentials}
              disabled={savingCreds || removingCreds}
              type="button"
            >
              {removingCreds ? "Removing…" : "Remove Credentials"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
