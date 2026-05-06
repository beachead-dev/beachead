import { useEffect, useState, useCallback } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../lib/api";

interface McpServer {
  id: string;
  name: string;
  url: string;
  description?: string;
  auth_headers?: Record<string, string>;
}

interface Persona {
  id: string;
  name: string;
  agent_type_id: string;
  workspace_path: string;
  memory_enabled: boolean;
  agent_cli_args: string[];
  mcp_servers: McpServer[];
  created_at: string;
  updated_at: string;
}

interface AgentType {
  id: string;
  name: string;
  is_builtin: boolean;
  metadata: {
    required_secrets: string[];
    auth_methods: string[];
    description: string;
    supports_interactive_auth: boolean;
  };
}

interface SecretStatus {
  service: string;
  configured: boolean;
}

interface McpContainer {
  id: string;
  persona_id: string | null;
  container_id: string | null;
  port: number;
  status: string;
}

interface McpEntry {
  name: string;
  url: string;
  description: string;
  auth_headers: string;
}

export function PersonasPage() {
  const [personas, setPersonas] = useState<Persona[]>([]);
  const [agents, setAgents] = useState<AgentType[]>([]);
  const [secrets, setSecrets] = useState<SecretStatus[]>([]);
  const [mcpContainers, setMcpContainers] = useState<McpContainer[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showForm, setShowForm] = useState(false);
  const [editingPersona, setEditingPersona] = useState<Persona | null>(null);
  const [deleteConfirm, setDeleteConfirm] = useState<string | null>(null);

  // Form state
  const [formName, setFormName] = useState("");
  const [formAgentType, setFormAgentType] = useState("");
  const [formWorkspace, setFormWorkspace] = useState("");
  const [formMemory, setFormMemory] = useState(false);
  const [formCliArgs, setFormCliArgs] = useState("");
  const [formMcpServers, setFormMcpServers] = useState<McpEntry[]>([]);
  const [formError, setFormError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const fetchData = useCallback(async () => {
    try {
      setLoading(true);
      const [personaList, agentList, secretList, containerList] = await Promise.all([
        api.get<Persona[]>("/api/personas"),
        api.get<AgentType[]>("/api/agents"),
        api.get<SecretStatus[]>("/api/secrets"),
        api.get<McpContainer[]>("/api/mcp-containers"),
      ]);
      setPersonas(personaList);
      setAgents(agentList);
      setSecrets(secretList);
      setMcpContainers(containerList);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load data");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  const resetForm = () => {
    setFormName("");
    setFormAgentType("");
    setFormWorkspace("");
    setFormMemory(false);
    setFormCliArgs("");
    setFormMcpServers([]);
    setFormError(null);
    setEditingPersona(null);
  };

  const openCreateForm = () => {
    resetForm();
    setShowForm(true);
  };

  const openEditForm = (persona: Persona) => {
    setEditingPersona(persona);
    setFormName(persona.name);
    setFormAgentType(persona.agent_type_id);
    setFormWorkspace(persona.workspace_path);
    setFormMemory(persona.memory_enabled);
    setFormCliArgs(persona.agent_cli_args.join(" "));
    setFormMcpServers(
      persona.mcp_servers.map((s) => ({
        name: s.name,
        url: s.url,
        description: s.description || "",
        auth_headers: s.auth_headers ? JSON.stringify(s.auth_headers) : "",
      }))
    );
    setFormError(null);
    setShowForm(true);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setFormError(null);

    if (!formName.trim()) {
      setFormError("Name is required");
      return;
    }
    if (!formAgentType) {
      setFormError("Agent type is required");
      return;
    }
    if (!formWorkspace.trim()) {
      setFormError("Workspace path is required");
      return;
    }

    for (const mcp of formMcpServers) {
      if (!mcp.name.trim() || !mcp.url.trim()) {
        setFormError("All MCP server entries require a name and URL");
        return;
      }
      try {
        new URL(mcp.url);
      } catch {
        setFormError(`Invalid URL for MCP server "${mcp.name}": ${mcp.url}`);
        return;
      }
    }

    setSubmitting(true);
    try {
      const mcpServers = formMcpServers.map((s) => ({
        name: s.name,
        url: s.url,
        description: s.description || undefined,
        auth_headers: s.auth_headers ? JSON.parse(s.auth_headers) : undefined,
      }));

      const body = {
        name: formName.trim(),
        agent_type_id: formAgentType,
        workspace_path: formWorkspace.trim(),
        memory_enabled: formMemory,
        agent_cli_args: formCliArgs.trim() ? formCliArgs.trim().split(/\s+/) : [],
        mcp_servers: mcpServers.length > 0 ? mcpServers : undefined,
      };

      if (editingPersona) {
        await api.put(`/api/personas/${editingPersona.id}`, body);
      } else {
        await api.post("/api/personas", body);
      }

      setShowForm(false);
      resetForm();
      await fetchData();
    } catch (e) {
      setFormError(e instanceof Error ? e.message : "Failed to save persona");
    } finally {
      setSubmitting(false);
    }
  };

  const handleDelete = async (id: string) => {
    try {
      await api.del(`/api/personas/${id}`);
      setDeleteConfirm(null);
      await fetchData();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to delete persona");
      setDeleteConfirm(null);
    }
  };

  const addMcpEntry = () => {
    setFormMcpServers([...formMcpServers, { name: "", url: "", description: "", auth_headers: "" }]);
  };

  const removeMcpEntry = (index: number) => {
    setFormMcpServers(formMcpServers.filter((_, i) => i !== index));
  };

  const updateMcpEntry = (index: number, field: keyof McpEntry, value: string) => {
    const updated = formMcpServers.map((entry, i) =>
      i === index ? { ...entry, [field]: value } : entry
    );
    setFormMcpServers(updated);
  };

  const getAgentName = (agentTypeId: string) => {
    const agent = agents.find((a) => a.id === agentTypeId);
    return agent?.name || "Unknown";
  };

  const hasMissingSecrets = (persona: Persona) => {
    const agent = agents.find((a) => a.id === persona.agent_type_id);
    if (!agent) return false;
    return agent.metadata.required_secrets.some(
      (s) => !secrets.find((sec) => sec.service === s && sec.configured)
    );
  };

  const getContainerStatus = (persona: Persona): McpContainer | undefined => {
    if (!persona.memory_enabled) return undefined;
    return mcpContainers.find((c) => c.persona_id === persona.id);
  };

  if (loading) {
    return (
      <div>
        <h2>Personas</h2>
        <p>Loading...</p>
      </div>
    );
  }

  return (
    <div className="personas-page">
      <div className="page-header">
        <h2>Personas</h2>
        <div className="page-header-actions">
          <button className="btn btn-primary" onClick={openCreateForm} aria-label="Create new persona">
            + New Persona
          </button>
          <a href="/help" className="help-link" aria-label="Personas help">?</a>
        </div>
      </div>

      {error && <div className="alert alert-error" role="alert">{error}</div>}

      {!showForm && (
        <div className="persona-list" role="list" aria-label="Personas list">
          {personas.length === 0 && <p className="empty-state">No personas configured. Create one to get started.</p>}
          {personas.map((persona) => (
            <div key={persona.id} className="card" role="listitem">
              <div className="card-header">
                <h3 className="card-title">
                  {persona.name}
                  {hasMissingSecrets(persona) && (
                    <span className="warning-badge" title="Missing required agent secrets">⚠️</span>
                  )}
                </h3>
                <div className="card-actions">
                  <button className="btn btn-sm" onClick={() => openEditForm(persona)} aria-label={`Edit ${persona.name}`}>
                    Edit
                  </button>
                  <button
                    className="btn btn-sm btn-danger"
                    onClick={() => setDeleteConfirm(persona.id)}
                    aria-label={`Delete ${persona.name}`}
                  >
                    Delete
                  </button>
                </div>
              </div>
              <div className="card-body">
                <dl className="detail-list">
                  <dt>Agent Type</dt>
                  <dd>{getAgentName(persona.agent_type_id)}</dd>
                  <dt>Workspace</dt>
                  <dd><code>{persona.workspace_path}</code></dd>
                  <dt>Memory</dt>
                  <dd>
                    {persona.memory_enabled ? "Enabled" : "Disabled"}
                    {persona.memory_enabled && (() => {
                      const container = getContainerStatus(persona);
                      if (!container) return null;
                      return (
                        <span className="card-meta" style={{ display: "inline-flex", marginTop: 0, marginLeft: "0.5rem" }}>
                          <span className={`status-indicator status-${container.status}`} />
                          {container.status}
                        </span>
                      );
                    })()}
                  </dd>
                  {persona.mcp_servers.length > 0 && (
                    <>
                      <dt>MCP Servers</dt>
                      <dd>{persona.mcp_servers.map((s) => s.name).join(", ")}</dd>
                    </>
                  )}
                </dl>
              </div>
            </div>
          ))}
        </div>
      )}

      {deleteConfirm && (
        <div className="modal-overlay" role="dialog" aria-label="Confirm deletion">
          <div className="modal">
            <h3>Confirm Delete</h3>
            <p>Are you sure you want to delete this persona? This cannot be undone.</p>
            <div className="modal-actions">
              <button className="btn" onClick={() => setDeleteConfirm(null)}>Cancel</button>
              <button className="btn btn-danger" onClick={() => handleDelete(deleteConfirm)}>Delete</button>
            </div>
          </div>
        </div>
      )}

      {showForm && (
        <form className="persona-form" onSubmit={handleSubmit} aria-label={editingPersona ? "Edit persona" : "Create persona"}>
          <h3>{editingPersona ? "Edit Persona" : "Create Persona"}</h3>
          {formError && <div className="alert alert-error" role="alert">{formError}</div>}

          <div className="form-group">
            <label htmlFor="persona-name">Name *</label>
            <input
              id="persona-name"
              type="text"
              value={formName}
              onChange={(e) => setFormName(e.target.value)}
              required
              aria-required="true"
            />
          </div>

          <div className="form-group">
            <label htmlFor="persona-agent-type">Agent Type *</label>
            <select
              id="persona-agent-type"
              value={formAgentType}
              onChange={(e) => setFormAgentType(e.target.value)}
              required
              aria-required="true"
            >
              <option value="">Select an agent type</option>
              {agents.map((agent) => (
                <option key={agent.id} value={agent.id}>
                  {agent.name}
                </option>
              ))}
            </select>
          </div>

          <div className="form-group">
            <label htmlFor="persona-workspace">Workspace Path *</label>
            <div className="input-with-button">
              <input
                id="persona-workspace"
                type="text"
                value={formWorkspace}
                onChange={(e) => setFormWorkspace(e.target.value)}
                placeholder="/path/to/workspace"
                required
                aria-required="true"
              />
              <button
                type="button"
                className="btn btn-sm"
                onClick={async () => {
                  const selected = await open({ directory: true, multiple: false });
                  if (selected) {
                    setFormWorkspace(selected as string);
                  }
                }}
                aria-label="Browse for workspace folder"
              >
                Browse
              </button>
            </div>
          </div>

          <div className="form-group form-group-inline">
            <label htmlFor="persona-memory">
              <input
                id="persona-memory"
                type="checkbox"
                checked={formMemory}
                onChange={(e) => setFormMemory(e.target.checked)}
              />
              Enable Memory
            </label>
          </div>

          <div className="form-group">
            <label htmlFor="persona-cli-args">Agent CLI Args</label>
            <input
              id="persona-cli-args"
              type="text"
              value={formCliArgs}
              onChange={(e) => setFormCliArgs(e.target.value)}
              placeholder="--flag1 --flag2 value"
            />
          </div>

          <fieldset className="form-group">
            <legend>MCP Servers</legend>
            {formMcpServers.map((entry, i) => (
              <div key={i} className="mcp-entry">
                <div className="mcp-entry-fields">
                  <input
                    type="text"
                    placeholder="Name"
                    value={entry.name}
                    onChange={(e) => updateMcpEntry(i, "name", e.target.value)}
                    aria-label={`MCP server ${i + 1} name`}
                  />
                  <input
                    type="text"
                    placeholder="URL (e.g., http://localhost:9100/sse)"
                    value={entry.url}
                    onChange={(e) => updateMcpEntry(i, "url", e.target.value)}
                    aria-label={`MCP server ${i + 1} URL`}
                  />
                  <input
                    type="text"
                    placeholder="Description (optional)"
                    value={entry.description}
                    onChange={(e) => updateMcpEntry(i, "description", e.target.value)}
                    aria-label={`MCP server ${i + 1} description`}
                  />
                  <input
                    type="text"
                    placeholder='Auth headers JSON (optional)'
                    value={entry.auth_headers}
                    onChange={(e) => updateMcpEntry(i, "auth_headers", e.target.value)}
                    aria-label={`MCP server ${i + 1} auth headers`}
                  />
                </div>
                <button type="button" className="btn btn-sm btn-danger" onClick={() => removeMcpEntry(i)} aria-label={`Remove MCP server ${i + 1}`}>
                  ✕
                </button>
              </div>
            ))}
            <button type="button" className="btn btn-sm" onClick={addMcpEntry}>
              + Add MCP Server
            </button>
          </fieldset>

          <div className="form-actions">
            <button type="button" className="btn" onClick={() => { setShowForm(false); resetForm(); }}>
              Cancel
            </button>
            <button type="submit" className="btn btn-primary" disabled={submitting}>
              {submitting ? "Saving..." : editingPersona ? "Update" : "Create"}
            </button>
          </div>
        </form>
      )}
    </div>
  );
}
