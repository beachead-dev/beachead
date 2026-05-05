import { useEffect, useState, useCallback } from "react";
import { api } from "../lib/api";

interface AgentType {
  id: { "0": string };
  name: string;
  sbx_agent: string | null;
  kit_ref: string | null;
  is_builtin: boolean;
  metadata: {
    required_secrets: string[];
    auth_methods: string[];
    description: string;
    supports_interactive_auth: boolean;
  };
  created_at: string;
  updated_at: string;
}

interface SecretStatus {
  service: string;
  configured: boolean;
}

interface TemplateInfo {
  tag: string;
  size: string | null;
  created: string | null;
}

export function AgentsPage() {
  const [agents, setAgents] = useState<AgentType[]>([]);
  const [secrets, setSecrets] = useState<SecretStatus[]>([]);
  const [templates, setTemplates] = useState<TemplateInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selectedAgent, setSelectedAgent] = useState<AgentType | null>(null);
  const [showRegisterForm, setShowRegisterForm] = useState(false);
  const [registerName, setRegisterName] = useState("");
  const [registerKitRef, setRegisterKitRef] = useState("");
  const [registerError, setRegisterError] = useState<string | null>(null);
  const [secretValue, setSecretValue] = useState("");
  const [settingSecret, setSettingSecret] = useState<string | null>(null);
  const [view, setView] = useState<"agents" | "credentials" | "templates">("agents");

  const fetchData = useCallback(async () => {
    try {
      setLoading(true);
      const [agentList, secretList, templateList] = await Promise.all([
        api.get<AgentType[]>("/api/agents"),
        api.get<SecretStatus[]>("/api/secrets"),
        api.get<TemplateInfo[]>("/api/templates"),
      ]);
      setAgents(agentList);
      setSecrets(secretList);
      setTemplates(templateList);
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

  const handleRegister = async (e: React.FormEvent) => {
    e.preventDefault();
    setRegisterError(null);
    if (!registerName.trim()) {
      setRegisterError("Name is required");
      return;
    }
    try {
      await api.post("/api/agents", {
        name: registerName.trim(),
        kit_ref: registerKitRef.trim() || undefined,
      });
      setShowRegisterForm(false);
      setRegisterName("");
      setRegisterKitRef("");
      await fetchData();
    } catch (e) {
      setRegisterError(e instanceof Error ? e.message : "Failed to register agent");
    }
  };

  const handleSetSecret = async (service: string) => {
    if (!secretValue.trim()) return;
    try {
      await api.post(`/api/secrets/${service}`, { value: secretValue });
      setSecretValue("");
      setSettingSecret(null);
      await fetchData();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to set secret");
    }
  };

  const handleRemoveSecret = async (service: string) => {
    try {
      await api.del(`/api/secrets/${service}`);
      await fetchData();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to remove secret");
    }
  };

  const handleOAuth = async (service: string) => {
    try {
      await api.post(`/api/secrets/${service}/oauth`);
      await fetchData();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to initiate OAuth");
    }
  };

  const handleRemoveTemplate = async (tag: string) => {
    try {
      await api.del(`/api/templates/${encodeURIComponent(tag)}`);
      await fetchData();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to remove template");
    }
  };

  const isSecretConfigured = (service: string) => {
    return secrets.find((s) => s.service === service)?.configured ?? false;
  };

  const allServices = Array.from(
    new Set(agents.flatMap((a) => a.metadata.required_secrets))
  ).sort();

  if (loading) {
    return (
      <div>
        <h2>Agents</h2>
        <p>Loading...</p>
      </div>
    );
  }

  return (
    <div className="agents-page">
      <div className="page-header">
        <h2>Agents</h2>
        <div className="page-header-actions">
          <a href="/help" className="help-link" aria-label="Agents help">?</a>
        </div>
      </div>

      {error && <div className="alert alert-error" role="alert">{error}</div>}

      <nav className="tab-nav" aria-label="Agents sections">
        <button className={`tab-btn ${view === "agents" ? "active" : ""}`} onClick={() => setView("agents")}>
          Agents
        </button>
        <button className={`tab-btn ${view === "credentials" ? "active" : ""}`} onClick={() => setView("credentials")}>
          Credentials
        </button>
        <button className={`tab-btn ${view === "templates" ? "active" : ""}`} onClick={() => setView("templates")}>
          Templates
        </button>
      </nav>

      {view === "agents" && (
        <div className="agents-section">
          <div className="section-header">
            <h3>Registered Agents</h3>
            <button className="btn btn-primary" onClick={() => setShowRegisterForm(true)} aria-label="Register custom agent">
              + Register Agent
            </button>
          </div>

          {showRegisterForm && (
            <form className="register-form card" onSubmit={handleRegister} aria-label="Register custom agent">
              <h4>Register Custom Agent</h4>
              {registerError && <div className="alert alert-error" role="alert">{registerError}</div>}
              <div className="form-group">
                <label htmlFor="agent-name">Name *</label>
                <input id="agent-name" type="text" value={registerName} onChange={(e) => setRegisterName(e.target.value)} required aria-required="true" />
              </div>
              <div className="form-group">
                <label htmlFor="agent-kit-ref">Kit Reference</label>
                <input id="agent-kit-ref" type="text" value={registerKitRef} onChange={(e) => setRegisterKitRef(e.target.value)} placeholder="path, OCI ref, or Git URL" />
              </div>
              <div className="form-actions">
                <button type="button" className="btn" onClick={() => setShowRegisterForm(false)}>Cancel</button>
                <button type="submit" className="btn btn-primary">Register</button>
              </div>
            </form>
          )}

          <div className="agent-list" role="list" aria-label="Agent list">
            {agents.map((agent) => (
              <div
                key={agent.id["0"]}
                className={`card card-clickable ${selectedAgent?.id["0"] === agent.id["0"] ? "card-selected" : ""}`}
                role="listitem"
                onClick={() => setSelectedAgent(selectedAgent?.id["0"] === agent.id["0"] ? null : agent)}
                onKeyDown={(e) => { if (e.key === "Enter") setSelectedAgent(agent); }}
                tabIndex={0}
                aria-label={`${agent.name} - ${agent.is_builtin ? "built-in" : "custom"}`}
              >
                <div className="card-header">
                  <h4 className="card-title">{agent.name}</h4>
                  <span className={`badge ${agent.is_builtin ? "badge-builtin" : "badge-custom"}`}>
                    {agent.is_builtin ? "Built-in" : "Custom"}
                  </span>
                </div>
                <p className="card-description">{agent.metadata.description}</p>
                {agent.metadata.required_secrets.length > 0 && (
                  <div className="card-meta">
                    <span className="meta-label">Auth:</span>
                    {agent.metadata.required_secrets.map((s) => (
                      <span key={s} className={`secret-badge ${isSecretConfigured(s) ? "configured" : "missing"}`}>
                        {s}
                      </span>
                    ))}
                  </div>
                )}
              </div>
            ))}
          </div>

          {selectedAgent && (
            <div className="agent-detail card" aria-label={`${selectedAgent.name} details`}>
              <h4>{selectedAgent.name} — Details</h4>
              <dl className="detail-list">
                <dt>Type</dt>
                <dd>{selectedAgent.is_builtin ? "Built-in" : "Custom"}</dd>
                {selectedAgent.sbx_agent && <><dt>sbx agent</dt><dd><code>{selectedAgent.sbx_agent}</code></dd></>}
                {selectedAgent.kit_ref && <><dt>Kit Reference</dt><dd><code>{selectedAgent.kit_ref}</code></dd></>}
                <dt>Auth Methods</dt>
                <dd>{selectedAgent.metadata.auth_methods.join(", ") || "None"}</dd>
                <dt>Interactive Auth</dt>
                <dd>{selectedAgent.metadata.supports_interactive_auth ? "Yes" : "No"}</dd>
                <dt>Required Secrets</dt>
                <dd>{selectedAgent.metadata.required_secrets.join(", ") || "None"}</dd>
              </dl>
            </div>
          )}
        </div>
      )}

      {view === "credentials" && (
        <div className="credentials-section">
          <h3>Credential Management</h3>
          <p className="section-description">Manage API keys and OAuth tokens for agent services. Secrets are stored in the OS keychain via <code>sbx secret</code>.</p>
          <div className="credential-list" role="list" aria-label="Credentials list">
            {allServices.map((service) => {
              const configured = isSecretConfigured(service);
              return (
                <div key={service} className="card credential-card" role="listitem">
                  <div className="card-header">
                    <h4 className="card-title">{service}</h4>
                    <span className={`status-dot ${configured ? "status-ok" : "status-missing"}`} aria-label={configured ? "Configured" : "Not configured"} />
                  </div>
                  <div className="card-body credential-actions">
                    {settingSecret === service ? (
                      <div className="secret-input-row">
                        <input
                          type="password"
                          value={secretValue}
                          onChange={(e) => setSecretValue(e.target.value)}
                          placeholder="Enter secret value"
                          aria-label={`Secret value for ${service}`}
                        />
                        <button className="btn btn-sm btn-primary" onClick={() => handleSetSecret(service)}>Save</button>
                        <button className="btn btn-sm" onClick={() => { setSettingSecret(null); setSecretValue(""); }}>Cancel</button>
                      </div>
                    ) : (
                      <>
                        <button className="btn btn-sm" onClick={() => setSettingSecret(service)} aria-label={`Set secret for ${service}`}>
                          {configured ? "Update" : "Set"}
                        </button>
                        {configured && (
                          <button className="btn btn-sm btn-danger" onClick={() => handleRemoveSecret(service)} aria-label={`Remove secret for ${service}`}>
                            Remove
                          </button>
                        )}
                        <button className="btn btn-sm" onClick={() => handleOAuth(service)} aria-label={`OAuth for ${service}`}>
                          OAuth
                        </button>
                      </>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        </div>
      )}

      {view === "templates" && (
        <div className="templates-section">
          <h3>Saved Templates</h3>
          <p className="section-description">Templates are saved sandbox states that can be reused when starting new sessions.</p>
          {templates.length === 0 ? (
            <p className="empty-state">No templates saved yet. Save a running sandbox as a template from the Sessions page.</p>
          ) : (
            <div className="template-list" role="list" aria-label="Templates list">
              {templates.map((tpl) => (
                <div key={tpl.tag} className="card" role="listitem">
                  <div className="card-header">
                    <h4 className="card-title">{tpl.tag}</h4>
                    <button className="btn btn-sm btn-danger" onClick={() => handleRemoveTemplate(tpl.tag)} aria-label={`Remove template ${tpl.tag}`}>
                      Remove
                    </button>
                  </div>
                  <div className="card-body">
                    {tpl.size && <span className="meta-label">Size: {tpl.size}</span>}
                    {tpl.created && <span className="meta-label" style={{ marginLeft: "1rem" }}>Created: {tpl.created}</span>}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
