import { useEffect, useState, useCallback, useRef } from "react";
import { api } from "../lib/api";
import { useWebSocket, ReadyState } from "../hooks/useWebSocket";
import { useDropzone } from "react-dropzone";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import "@xterm/xterm/css/xterm.css";

interface Persona {
  id: string;
  name: string;
  agent_type_id: string;
}

interface Session {
  id: string;
  persona_id: string;
  sandbox_id: string | null;
  status: string;
  error_message: string | null;
  created_at: string;
}

interface PortMapping {
  host_ip: string;
  host_port: number;
  sandbox_port: number;
  protocol: string;
}

interface SessionTab {
  session: Session;
  personaName: string;
}

/** Extract a clean sandbox name from potentially garbage stored value */
function extractSandboxName(sandboxId: string | null): string {
  if (!sandboxId) return "";
  if (!sandboxId.includes("\n") && sandboxId.length < 80) {
    return sandboxId;
  }
  const createdMatch = sandboxId.match(/Created sandbox '([^']+)'/);
  if (createdMatch && createdMatch[1]) return createdMatch[1];
  const runMatch = sandboxId.match(/sbx run (\S+)/);
  if (runMatch && runMatch[1]) return runMatch[1];
  const lines = sandboxId.split("\n").map((l) => l.trim()).filter((l) => l.length > 0 && l.length < 60);
  return lines[lines.length - 1] || sandboxId.slice(0, 20) + "…";
}

export function SessionsPage() {
  const [personas, setPersonas] = useState<Persona[]>([]);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [tabs, setTabs] = useState<SessionTab[]>([]);
  const [activeTabId, setActiveTabId] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [showLauncher, setShowLauncher] = useState(false);
  const [selectedPersonaId, setSelectedPersonaId] = useState("");
  const [launching, setLaunching] = useState(false);

  const fetchData = useCallback(async () => {
    try {
      setLoading(true);
      const [personaList, sessionList] = await Promise.all([
        api.get<Persona[]>("/api/personas"),
        api.get<Session[]>("/api/sessions"),
      ]);
      setPersonas(personaList);
      setSessions(sessionList);

      const activeSessions = sessionList.filter(
        (s) => s.status === "running" || s.status === "starting"
      );
      setTabs(
        activeSessions.map((session) => ({
          session,
          personaName: personaList.find((p) => p.id === session.persona_id)?.name || "Unknown",
        }))
      );
      if (activeSessions.length > 0 && !activeTabId) {
        setActiveTabId(activeSessions[0]?.id ?? null);
      }
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load data");
    } finally {
      setLoading(false);
    }
  }, [activeTabId]);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  const handleLaunch = async () => {
    if (!selectedPersonaId) return;
    setLaunching(true);
    try {
      const resp = await api.post<{ session_id: string; ws_url: string }>("/api/sessions", {
        persona_id: selectedPersonaId,
      });
      setShowLauncher(false);
      setSelectedPersonaId("");
      await fetchData();
      setActiveTabId(resp.session_id);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to start session");
    } finally {
      setLaunching(false);
    }
  };

  const handleResumeSession = async (sessionId: string) => {
    try {
      await api.post(`/api/sessions/${sessionId}/resume`);
      await fetchData();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to resume session");
    }
  };

  const handleRemoveSession = async (sessionId: string) => {
    try {
      await api.del(`/api/sessions/${sessionId}`);
      setTabs((prev) => prev.filter((t) => t.session.id !== sessionId));
      if (activeTabId === sessionId) {
        const remaining = tabs.filter((t) => t.session.id !== sessionId);
        setActiveTabId(remaining.length > 0 ? (remaining[0]?.session.id ?? null) : null);
      }
      await fetchData();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to remove session");
    }
  };

  const handleCloseTab = async (sessionId: string) => {
    try {
      await api.post(`/api/sessions/${sessionId}/stop`);
      setTabs((prev) => prev.filter((t) => t.session.id !== sessionId));
      if (activeTabId === sessionId) {
        const remaining = tabs.filter((t) => t.session.id !== sessionId);
        setActiveTabId(remaining.length > 0 ? (remaining[0]?.session.id ?? null) : null);
      }
      await fetchData();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to stop session");
    }
  };

  if (loading) {
    return (
      <div>
        <h2>Sessions</h2>
        <p>Loading...</p>
      </div>
    );
  }

  return (
    <div className="sessions-page">
      {error && <div className="alert alert-error" role="alert">{error}</div>}

      {showLauncher && (
        <div className="session-launcher card" aria-label="New session launcher">
          <h3>Start New Session</h3>
          <div className="form-group">
            <label htmlFor="launch-persona">Select Persona</label>
            <select
              id="launch-persona"
              value={selectedPersonaId}
              onChange={(e) => setSelectedPersonaId(e.target.value)}
            >
              <option value="">Choose a persona...</option>
              {personas.map((p) => (
                <option key={p.id} value={p.id}>{p.name}</option>
              ))}
            </select>
          </div>
          <div className="form-actions">
            <button className="btn" onClick={() => setShowLauncher(false)}>Cancel</button>
            <button className="btn btn-primary" onClick={handleLaunch} disabled={!selectedPersonaId || launching}>
              {launching ? "Starting..." : "Start Session"}
            </button>
          </div>
        </div>
      )}

      <div className="session-tabs">
        {/* Vertical session sidebar */}
        <div className="session-sidebar">
          <div className="session-sidebar-header">
            <button className="btn btn-primary btn-sm" onClick={() => setShowLauncher(true)} aria-label="Start new session">
              + New Session
            </button>
          </div>
          <ul className="session-list" role="tablist" aria-label="Session list">
            {tabs.map((tab) => (
              <li
                key={tab.session.id}
                className={`session-list-item ${activeTabId === tab.session.id ? "active" : ""}`}
                role="tab"
                aria-selected={activeTabId === tab.session.id}
                onClick={() => setActiveTabId(tab.session.id)}
                onKeyDown={(e) => { if (e.key === "Enter") setActiveTabId(tab.session.id); }}
                tabIndex={0}
              >
                <span className={`status-indicator status-${tab.session.status}`} aria-label={tab.session.status} />
                <span className="session-name">{tab.personaName}</span>
                <button
                  className="tab-close"
                  onClick={(e) => { e.stopPropagation(); handleCloseTab(tab.session.id); }}
                  aria-label={`Close ${tab.personaName} session`}
                >
                  ✕
                </button>
              </li>
            ))}
          </ul>

          {/* Stopped sessions — collapsible section at bottom of sidebar */}
          <StoppedSessionsSection
            sessions={sessions.filter((s) => s.status === "stopped")}
            onResume={handleResumeSession}
            onRemove={handleRemoveSession}
          />
        </div>

        {/* Main content: render ALL session panels, hide inactive ones */}
        <div className="session-main">
          {tabs.length === 0 && (
            <div style={{ padding: "1.5rem" }}>
              <p className="empty-state">No active sessions. Start a new session to begin.</p>
            </div>
          )}
          {tabs.map((tab) => (
            <div
              key={tab.session.id}
              style={{ display: activeTabId === tab.session.id ? "flex" : "none", flexDirection: "column", flex: 1, height: "100%" }}
            >
              <SessionPanel sessionId={tab.session.id} sandboxId={tab.session.sandbox_id} />
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

interface StoppedSessionsSectionProps {
  sessions: Session[];
  onResume: (id: string) => void;
  onRemove: (id: string) => void;
}

function StoppedSessionsSection({ sessions, onResume, onRemove }: StoppedSessionsSectionProps) {
  const [collapsed, setCollapsed] = useState(false);

  if (sessions.length === 0) return null;

  return (
    <div className={`stopped-section ${collapsed ? "collapsed" : ""}`}>
      <button
        className="stopped-section-header"
        onClick={() => setCollapsed(!collapsed)}
        aria-expanded={!collapsed}
        aria-label="Toggle stopped sessions"
      >
        <span className={`caret ${collapsed ? "caret-right" : "caret-down"}`}>▸</span>
        <span>Stopped ({sessions.length})</span>
      </button>
      {!collapsed && (
        <ul className="stopped-list">
          {sessions.map((session) => (
            <li key={session.id} className="stopped-list-item">
              <span className="stopped-item-name">
                {extractSandboxName(session.sandbox_id) || session.id.slice(0, 8)}
              </span>
              <div className="stopped-item-actions">
                <button
                  className="btn-icon"
                  onClick={() => onResume(session.id)}
                  aria-label="Resume"
                  title="Resume"
                >
                  ▶
                </button>
                <button
                  className="btn-icon btn-icon-danger"
                  onClick={() => onRemove(session.id)}
                  aria-label="Remove"
                  title="Remove"
                >
                  ✕
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

interface SessionPanelProps {
  sessionId: string;
  sandboxId: string | null;
}

function SessionPanel({ sessionId, sandboxId }: SessionPanelProps) {
  const [panelView, setPanelView] = useState<"terminal" | "files" | "ports">("terminal");

  return (
    <div className="session-panel">
      <nav className="panel-nav" aria-label="Session panel sections">
        <button className={`tab-btn ${panelView === "terminal" ? "active" : ""}`} onClick={() => setPanelView("terminal")}>
          Terminal
        </button>
        <button className={`tab-btn ${panelView === "files" ? "active" : ""}`} onClick={() => setPanelView("files")}>
          Files
        </button>
        <button className={`tab-btn ${panelView === "ports" ? "active" : ""}`} onClick={() => setPanelView("ports")}>
          Ports
        </button>
      </nav>

      {panelView === "terminal" && <TerminalView sessionId={sessionId} />}
      {panelView === "files" && <FileUploadView sessionId={sessionId} />}
      {panelView === "ports" && sandboxId && <PortManagerView sandboxId={sandboxId} />}
      {panelView === "ports" && !sandboxId && <p>No sandbox associated with this session.</p>}
    </div>
  );
}

function TerminalView({ sessionId }: { sessionId: string }) {
  const termRef = useRef<HTMLDivElement>(null);
  const terminalRef = useRef<Terminal | null>(null);
  const fitAddonRef = useRef<FitAddon | null>(null);
  const wsUrl = `ws://127.0.0.1:9876/api/sessions/${sessionId}/terminal`;
  const { sendMessage, lastMessage, readyState, connect } = useWebSocket(wsUrl);

  useEffect(() => {
    if (!termRef.current) return;

    const term = new Terminal({
      cursorBlink: true,
      fontSize: 14,
      fontFamily: "Menlo, Monaco, 'Courier New', monospace",
      theme: { background: "#1a1a2e" },
    });
    const fitAddon = new FitAddon();
    const webLinksAddon = new WebLinksAddon();

    term.loadAddon(fitAddon);
    term.loadAddon(webLinksAddon);
    term.open(termRef.current);
    fitAddon.fit();

    term.onData((data) => {
      sendMessage(data);
    });

    terminalRef.current = term;
    fitAddonRef.current = fitAddon;
    connect();

    const handleResize = () => fitAddon.fit();
    window.addEventListener("resize", handleResize);

    return () => {
      window.removeEventListener("resize", handleResize);
      term.dispose();
      terminalRef.current = null;
      fitAddonRef.current = null;
    };
  }, [sessionId, connect, sendMessage]);

  useEffect(() => {
    if (lastMessage && terminalRef.current) {
      terminalRef.current.write(lastMessage.data);
    }
  }, [lastMessage]);

  return (
    <div className="terminal-container">
      <div className="terminal-status">
        <span className={`status-dot ${readyState === ReadyState.OPEN ? "status-ok" : "status-missing"}`} />
        <span>{readyState === ReadyState.OPEN ? "Connected" : "Disconnected"}</span>
      </div>
      <div ref={termRef} className="terminal-view" aria-label="Terminal" role="application" />
    </div>
  );
}

function FileUploadView({ sessionId }: { sessionId: string }) {
  const [uploadResult, setUploadResult] = useState<string | null>(null);
  const [uploading, setUploading] = useState(false);
  const [uploadError, setUploadError] = useState<string | null>(null);

  const onDrop = useCallback(async (acceptedFiles: File[]) => {
    if (acceptedFiles.length === 0) return;
    setUploading(true);
    setUploadError(null);
    setUploadResult(null);

    try {
      const formData = new FormData();
      for (const file of acceptedFiles) {
        formData.append("file", file);
      }

      const response = await fetch(`http://127.0.0.1:9876/api/sessions/${sessionId}/upload`, {
        method: "POST",
        body: formData,
      });

      if (!response.ok) {
        throw new Error(`Upload failed: ${response.statusText}`);
      }

      const result = await response.json();
      setUploadResult(result.sandbox_path || "Upload complete");
    } catch (e) {
      setUploadError(e instanceof Error ? e.message : "Upload failed");
    } finally {
      setUploading(false);
    }
  }, [sessionId]);

  const { getRootProps, getInputProps, isDragActive } = useDropzone({ onDrop });

  return (
    <div className="file-upload-view">
      <div
        {...getRootProps()}
        className={`dropzone ${isDragActive ? "dropzone-active" : ""}`}
        aria-label="File upload drop zone"
      >
        <input {...getInputProps()} />
        {uploading ? (
          <p>Uploading...</p>
        ) : isDragActive ? (
          <p>Drop files here...</p>
        ) : (
          <p>Drag & drop files here, or click to select files</p>
        )}
      </div>
      {uploadError && <div className="alert alert-error" role="alert">{uploadError}</div>}
      {uploadResult && (
        <div className="upload-result">
          <p>Uploaded to: <code>{uploadResult}</code></p>
        </div>
      )}
    </div>
  );
}

function PortManagerView({ sandboxId }: { sandboxId: string }) {
  const [ports, setPorts] = useState<PortMapping[]>([]);
  const [portSpec, setPortSpec] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchPorts = useCallback(async () => {
    try {
      setLoading(true);
      const portList = await api.get<PortMapping[]>(`/api/sandboxes/${sandboxId}/ports`);
      setPorts(portList);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load ports");
    } finally {
      setLoading(false);
    }
  }, [sandboxId]);

  useEffect(() => {
    fetchPorts();
  }, [fetchPorts]);

  const handlePublish = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!portSpec.trim()) return;
    try {
      await api.post(`/api/sandboxes/${sandboxId}/ports`, { port_spec: portSpec.trim() });
      setPortSpec("");
      await fetchPorts();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to publish port");
    }
  };

  const handleUnpublish = async (_port: PortMapping) => {
    try {
      await api.del(`/api/sandboxes/${sandboxId}/ports`);
      await fetchPorts();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to unpublish port");
    }
  };

  if (loading) return <p>Loading ports...</p>;

  return (
    <div className="port-manager">
      <p className="section-description">Published ports do not persist across sandbox restarts.</p>
      {error && <div className="alert alert-error" role="alert">{error}</div>}

      <form className="port-form" onSubmit={handlePublish} aria-label="Publish port">
        <input
          type="text"
          value={portSpec}
          onChange={(e) => setPortSpec(e.target.value)}
          placeholder="Port spec (e.g., 8080 or 8080:80)"
          aria-label="Port specification"
        />
        <button type="submit" className="btn btn-sm btn-primary">Publish</button>
      </form>

      {ports.length > 0 ? (
        <table className="port-table" aria-label="Published ports">
          <thead>
            <tr>
              <th>Host Port</th>
              <th>Sandbox Port</th>
              <th>Protocol</th>
              <th>Actions</th>
            </tr>
          </thead>
          <tbody>
            {ports.map((p, i) => (
              <tr key={i}>
                <td>{p.host_ip}:{p.host_port}</td>
                <td>{p.sandbox_port}</td>
                <td>{p.protocol}</td>
                <td>
                  <button className="btn btn-sm btn-danger" onClick={() => handleUnpublish(p)} aria-label={`Unpublish port ${p.sandbox_port}`}>
                    Unpublish
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <p className="empty-state">No ports published.</p>
      )}
    </div>
  );
}
