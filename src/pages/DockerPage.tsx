import { useState } from "react";

type DockerTab = "sandboxes" | "containers";

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
        <div role="tabpanel" aria-label="Sandboxes tab content">
          <p className="empty-state">Sandboxes content</p>
        </div>
      )}

      {activeTab === "containers" && (
        <div role="tabpanel" aria-label="Containers tab content">
          <p className="empty-state">Containers content</p>
        </div>
      )}
    </div>
  );
}
