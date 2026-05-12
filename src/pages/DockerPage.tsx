import { useState, useCallback } from "react";
import { getSandboxes, stopSandbox, startSandbox, removeSandbox, SandboxInfo } from "../lib/api";
import { usePolling } from "../hooks/usePolling";
import { deriveSandboxButtonStates } from "../lib/sandboxButtonStates";

type DockerTab = "sandboxes" | "containers";

function SandboxesTab({ active }: { active: boolean }) {
  const [showAll, setShowAll] = useState(false);
  const [pendingActions, setPendingActions] = useState<Set<string>>(new Set());
  const [actionError, setActionError] = useState<string | null>(null);

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
          // TODO: Task 5.4 will add confirmation dialog before this call
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
                      onClick={() => handleSandboxAction(id, "remove")}
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
        <div role="tabpanel" aria-label="Containers tab content">
          <p className="empty-state">Containers content</p>
        </div>
      )}
    </div>
  );
}
