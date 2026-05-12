import { useState, useCallback } from "react";
import { getSandboxes, stopSandbox, startSandbox, removeSandbox, SandboxInfo, getMcpContainers, McpContainerResponse, startContainer, stopContainer, removeContainer } from "../lib/api";
import { usePolling } from "../hooks/usePolling";
import { deriveSandboxButtonStates } from "../lib/sandboxButtonStates";
import { deriveContainerButtonStates } from "../lib/containerButtonStates";
import { ConfirmDialog } from "../components/ConfirmDialog";

type DockerTab = "sandboxes" | "containers";

function SandboxesTab({ active }: { active: boolean }) {
  const [showAll, setShowAll] = useState(false);
  const [pendingActions, setPendingActions] = useState<Set<string>>(new Set());
  const [actionError, setActionError] = useState<string | null>(null);
  const [removeTarget, setRemoveTarget] = useState<{ id: string; name: string } | null>(null);

  const fetchFn = useCallback(() => getSandboxes(showAll), [showAll]);

  const { data, error, loading, stale, refresh } = usePolling<SandboxInfo[]>(
    fetchFn,
    10000,
    active,
  );

  const handleSandboxAction = async (
    id: string,
    action: "start" | "stop" | "remove",
  ) => {
    setPendingActions((prev) => new Set(prev).add(id));
    setActionError(null);

    try {
      switch (action) {
        case "stop":
          await stopSandbox(id);
          break;
        case "start":
          await startSandbox(id);
          break;
        case "remove":
          await removeSandbox(id);
          break;
      }
      refresh();
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Action failed";
      setActionError(`Failed to ${action} sandbox: ${message}`);
      refresh();
    } finally {
      setPendingActions((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  };

  const handleRemoveClick = (id: string, name: string) => {
    setRemoveTarget({ id, name });
  };

  const handleRemoveConfirm = () => {
    if (removeTarget) {
      handleSandboxAction(removeTarget.id, "remove");
      setRemoveTarget(null);
    }
  };

  const handleRemoveCancel = () => {
    setRemoveTarget(null);
  };

  const sandboxes = data ?? [];

  return (
    <div role="tabpanel" aria-label="Sandboxes tab content">
      <div className="tab-toolbar">
        <label className="toggle-label">
          <input
            type="checkbox"
            checked={showAll}
            onChange={(e) => setShowAll(e.target.checked)}
          />
          Show All
        </label>
      </div>

      {stale && (
        <div className="alert alert-warning" role="status">
          Data may be stale. Retrying…
        </div>
      )}

      {actionError && (
        <div className="alert alert-error" role="alert">
          {actionError}
          <button
            className="btn btn-sm"
            onClick={() => setActionError(null)}
            aria-label="Dismiss error"
          >
            ✕
          </button>
        </div>
      )}

      {loading && (
        <p className="loading-indicator">Loading sandboxes…</p>
      )}

      {error && !data && !loading && (
        <div className="alert alert-error" role="alert">
          {error.message}
        </div>
      )}

      {!loading && !error && sandboxes.length === 0 && (
        <p className="empty-state">No sandboxes found</p>
      )}

      {sandboxes.length > 0 && (
        <table className="sandbox-table" aria-label="Sandboxes table">
          <thead>
            <tr>
              <th>Name</th>
              <th>Status</th>
              <th>ID</th>
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            {sandboxes.map((sandbox, index) => {
              const id = sandbox.id ?? "";
              const isPending = pendingActions.has(id);
              const buttonStates = deriveSandboxButtonStates(sandbox.status);

              return (
                <tr key={sandbox.id ?? `sandbox-${index}`}>
                  <td>{sandbox.name ?? "\u2014"}</td>
                  <td>{sandbox.status ?? "\u2014"}</td>
                  <td>{sandbox.id ?? "\u2014"}</td>
                  <td className="action-buttons">
                    <button
                      className="btn btn-sm"
                      disabled={isPending || !buttonStates.startEnabled}
                      onClick={() => handleSandboxAction(id, "start")}
                      aria-label={`Start sandbox ${sandbox.name ?? id}`}
                    >
                      Start
                    </button>
                    <button
                      className="btn btn-sm"
                      disabled={isPending || !buttonStates.stopEnabled}
                      onClick={() => handleSandboxAction(id, "stop")}
                      aria-label={`Stop sandbox ${sandbox.name ?? id}`}
                    >
                      Stop
                    </button>
                    <button
                      className="btn btn-sm btn-danger"
                      disabled={isPending || !buttonStates.removeEnabled}
                      onClick={() => handleRemoveClick(id, sandbox.name ?? id)}
                      aria-label={`Remove sandbox ${sandbox.name ?? id}`}
                    >
                      Remove
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}

      <ConfirmDialog
        open={removeTarget !== null}
        title="Remove Sandbox"
        message={`This will permanently remove the sandbox '${removeTarget?.name ?? ""}'. This action is irreversible.`}
        confirmLabel="Remove"
        onConfirm={handleRemoveConfirm}
        onCancel={handleRemoveCancel}
      />
    </div>
  );
}

function ContainersTab({ active }: { active: boolean }) {
  const [showAll, setShowAll] = useState(false);
  const [pendingActions, setPendingActions] = useState<Set<string>>(new Set());
  const [actionError, setActionError] = useState<string | null>(null);
  const [removeTarget, setRemoveTarget] = useState<{ id: string; name: string } | null>(null);
  const [deleteVolume, setDeleteVolume] = useState(false);

  const fetchFn = useCallback(() => getMcpContainers(showAll), [showAll]);

  const { data, error, loading, stale, refresh } = usePolling<McpContainerResponse[]>(
    fetchFn,
    10000,
    active,
  );

  // Sort by created_at descending (newest first) client-side for safety
  const containers = (data ?? []).slice().sort(
    (a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime(),
  );

  const isUnmanaged = (container: McpContainerResponse) =>
    container.id.startsWith("unmanaged-");

  const formatDate = (dateStr: string) => {
    try {
      return new Date(dateStr).toLocaleString();
    } catch {
      return dateStr;
    }
  };

  const handleContainerAction = async (
    id: string,
    action: "start" | "stop" | "remove",
    deleteVol = false,
  ) => {
    setPendingActions((prev) => new Set(prev).add(id));
    setActionError(null);

    try {
      switch (action) {
        case "start":
          await startContainer(id);
          break;
        case "stop":
          await stopContainer(id);
          break;
        case "remove":
          await removeContainer(id, deleteVol);
          break;
      }
      refresh();
    } catch (err) {
      const message =
        err instanceof Error ? err.message : "Action failed";
      setActionError(`Failed to ${action} container: ${message}`);
      refresh();
    } finally {
      setPendingActions((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  };

  const handleRemoveClick = (id: string, name: string) => {
    setRemoveTarget({ id, name });
  };

  const handleRemoveConfirm = () => {
    if (removeTarget) {
      handleContainerAction(removeTarget.id, "remove", deleteVolume);
      setRemoveTarget(null);
      setDeleteVolume(false);
    }
  };

  const handleRemoveCancel = () => {
    setRemoveTarget(null);
    setDeleteVolume(false);
  };

  return (
    <div role="tabpanel" aria-label="Containers tab content">
      <div className="tab-toolbar">
        <label className="toggle-label">
          <input
            type="checkbox"
            checked={showAll}
            onChange={(e) => setShowAll(e.target.checked)}
          />
          Show All
        </label>
      </div>

      {stale && (
        <div className="alert alert-warning" role="status">
          Data may be stale. Retrying…
        </div>
      )}

      {actionError && (
        <div className="alert alert-error" role="alert">
          {actionError}
          <button
            className="btn btn-sm"
            onClick={() => setActionError(null)}
            aria-label="Dismiss error"
          >
            ✕
          </button>
        </div>
      )}

      {loading && (
        <p className="loading-indicator">Loading containers…</p>
      )}

      {error && !data && !loading && (
        <div className="alert alert-error" role="alert">
          {error.message}
        </div>
      )}

      {!loading && !error && containers.length === 0 && (
        <p className="empty-state">No containers found</p>
      )}

      {containers.length > 0 && (
        <table className="container-table" aria-label="Containers table">
          <thead>
            <tr>
              <th>Persona Name</th>
              <th>Port</th>
              <th>Status</th>
              <th>Volume Name</th>
              <th>Created Date</th>
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            {containers.map((container) => {
              const unmanaged = isUnmanaged(container);
              const isPending = pendingActions.has(container.id);
              const buttonStates = deriveContainerButtonStates(container.status, unmanaged);

              return (
                <tr key={container.id}>
                  <td>
                    {container.persona_name}
                    {unmanaged && (
                      <span className="badge badge-unmanaged">Unmanaged</span>
                    )}
                  </td>
                  <td>{container.port}</td>
                  <td>{container.status}</td>
                  <td>{container.volume_name}</td>
                  <td>{formatDate(container.created_at)}</td>
                  <td className="action-buttons">
                    <button
                      className="btn btn-sm"
                      disabled={isPending || !buttonStates.startEnabled}
                      onClick={() => handleContainerAction(container.id, "start")}
                      aria-label={`Start container ${container.persona_name}`}
                    >
                      Start
                    </button>
                    <button
                      className="btn btn-sm"
                      disabled={isPending || !buttonStates.stopEnabled}
                      onClick={() => handleContainerAction(container.id, "stop")}
                      aria-label={`Stop container ${container.persona_name}`}
                    >
                      Stop
                    </button>
                    <button
                      className="btn btn-sm btn-danger"
                      disabled={isPending || !buttonStates.removeEnabled}
                      onClick={() => handleRemoveClick(container.id, container.persona_name)}
                      aria-label={`Remove container ${container.persona_name}`}
                    >
                      Remove
                    </button>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}

      <ConfirmDialog
        open={removeTarget !== null}
        title="Remove Container"
        message={`This will permanently remove the container '${removeTarget?.name ?? ""}'. This action is irreversible.`}
        confirmLabel="Remove"
        onConfirm={handleRemoveConfirm}
        onCancel={handleRemoveCancel}
      >
        <label className="confirm-dialog-checkbox">
          <input
            type="checkbox"
            checked={deleteVolume}
            onChange={(e) => setDeleteVolume(e.target.checked)}
          />
          Also delete associated Docker volume
        </label>
      </ConfirmDialog>
    </div>
  );
}

export function DockerPage() {
  const [activeTab, setActiveTab] = useState<DockerTab>("sandboxes");

  return (
    <div className="docker-page">
      <div className="page-header">
        <h2>Docker</h2>
      </div>

      <nav className="tab-nav" aria-label="Docker resource tabs">
        <button
          className={`tab-btn ${activeTab === "sandboxes" ? "active" : ""}`}
          onClick={() => setActiveTab("sandboxes")}
          aria-selected={activeTab === "sandboxes"}
          role="tab"
        >
          Sandboxes
        </button>
        <button
          className={`tab-btn ${activeTab === "containers" ? "active" : ""}`}
          onClick={() => setActiveTab("containers")}
          aria-selected={activeTab === "containers"}
          role="tab"
        >
          Containers
        </button>
      </nav>

      {activeTab === "sandboxes" && (
        <SandboxesTab active={activeTab === "sandboxes"} />
      )}

      {activeTab === "containers" && (
        <ContainersTab active={activeTab === "containers"} />
      )}
    </div>
  );
}
