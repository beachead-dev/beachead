import { useState, useEffect, useRef } from "react";
import { api, createFromSource, ManagedRepoResponse } from "../lib/api";

interface Persona {
  id: string;
  name: string;
}

export interface CreateMirrorModalProps {
  open: boolean;
  onClose: () => void;
  onCreated: (repo: ManagedRepoResponse) => void;
  onError: (message: string) => void;
}

export function CreateMirrorModal({
  open,
  onClose,
  onCreated,
  onError,
}: CreateMirrorModalProps) {
  const [personas, setPersonas] = useState<Persona[]>([]);
  const [personaId, setPersonaId] = useState("");
  const [source, setSource] = useState("");
  const [useCredentials, setUseCredentials] = useState(false);
  const [username, setUsername] = useState("");
  const [secret, setSecret] = useState("");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);

  // Fetch personas on open
  useEffect(() => {
    if (!open) return;
    api.get<Persona[]>("/api/personas").then((list) => {
      setPersonas(list);
      if (list.length > 0 && !personaId) {
        setPersonaId(list[0]!.id);
      }
    }).catch(() => {
      // Non-critical
    });
  }, [open]);

  // Reset form on open
  useEffect(() => {
    if (open) {
      setSource("");
      setUseCredentials(false);
      setUsername("");
      setSecret("");
      setError(null);
      setCreating(false);
    }
  }, [open]);

  // Focus cancel on open
  useEffect(() => {
    if (open && cancelRef.current) {
      cancelRef.current.focus();
    }
  }, [open]);

  // Escape to close
  useEffect(() => {
    if (!open) return;
    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape" && !creating) onClose();
    }
    document.addEventListener("keydown", handleKey);
    return () => document.removeEventListener("keydown", handleKey);
  }, [open, creating, onClose]);

  const handleCreate = async () => {
    if (!personaId || !source.trim()) return;
    setCreating(true);
    setError(null);
    try {
      const repo = await createFromSource({
        persona_id: personaId,
        source: source.trim(),
        username: useCredentials && username.trim() ? username.trim() : undefined,
        secret: useCredentials && secret.trim() ? secret.trim() : undefined,
      });
      onCreated(repo);
    } catch (err) {
      const message = err instanceof Error ? err.message : "Failed to create mirror";
      setError(message);
      onError(message);
    } finally {
      setCreating(false);
    }
  };

  if (!open) return null;

  const isUrl = source.startsWith("https://") || source.startsWith("git@");
  const canCreate = personaId && source.trim() && !creating;

  return (
    <div
      className="modal-overlay"
      onClick={(e) => { if (e.target === e.currentTarget && !creating) onClose(); }}
      role="dialog"
      aria-modal="true"
      aria-labelledby="create-mirror-title"
    >
      <div className="modal create-mirror-modal">
        <h3 id="create-mirror-title">Create Mirror</h3>
        <p className="create-mirror-description">
          Clone a repository from a remote URL or local directory into the mirror,
          then create a stripped workspace for the selected persona.
        </p>

        <div className="form-group">
          <label htmlFor="create-mirror-persona">Persona</label>
          <select
            id="create-mirror-persona"
            value={personaId}
            onChange={(e) => setPersonaId(e.target.value)}
            disabled={creating}
          >
            {personas.map((p) => (
              <option key={p.id} value={p.id}>{p.name}</option>
            ))}
          </select>
        </div>

        <div className="form-group">
          <label htmlFor="create-mirror-source">Source (URL or local path)</label>
          <input
            id="create-mirror-source"
            type="text"
            className="input"
            value={source}
            onChange={(e) => setSource(e.target.value)}
            placeholder="https://github.com/user/repo.git or /path/to/local/repo"
            disabled={creating}
            autoFocus
          />
          {source && (
            <span className="field-hint">
              {isUrl ? "Remote URL — will clone via network" : "Local path — will clone from disk"}
            </span>
          )}
        </div>

        {isUrl && (
          <div className="form-group">
            <label className="create-mirror-cred-toggle">
              <input
                type="checkbox"
                checked={useCredentials}
                onChange={(e) => setUseCredentials(e.target.checked)}
                disabled={creating}
              />
              Private repository (requires credentials)
            </label>
          </div>
        )}

        {useCredentials && isUrl && (
          <>
            <div className="form-group">
              <label htmlFor="create-mirror-username">Username</label>
              <input
                id="create-mirror-username"
                type="text"
                className="input"
                value={username}
                onChange={(e) => setUsername(e.target.value)}
                placeholder="Username or token name"
                disabled={creating}
                autoComplete="off"
              />
            </div>
            <div className="form-group">
              <label htmlFor="create-mirror-secret">Token / Password</label>
              <input
                id="create-mirror-secret"
                type="password"
                className="input"
                value={secret}
                onChange={(e) => setSecret(e.target.value)}
                placeholder="Personal access token or password"
                disabled={creating}
                autoComplete="off"
              />
            </div>
          </>
        )}

        {error && (
          <div className="field-error" role="alert">{error}</div>
        )}

        <div className="modal-actions">
          <button
            className="btn"
            onClick={onClose}
            disabled={creating}
            ref={cancelRef}
            type="button"
          >
            Cancel
          </button>
          <button
            className="btn btn-primary"
            onClick={handleCreate}
            disabled={!canCreate}
            type="button"
          >
            {creating ? "Creating…" : "Create Mirror"}
          </button>
        </div>
      </div>
    </div>
  );
}
