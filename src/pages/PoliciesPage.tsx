import { useEffect, useState, useCallback, useRef } from "react";
import { api } from "../lib/api";

interface PolicyRule {
  id: string | null;
  action: string;
  target: string;
  origin: string | null;
  provenance: string | null;
  rule_type: string | null;
  status: string | null;
}

interface PolicyState {
  default_policy: string;
  rules: PolicyRule[];
}

interface PolicyLogEntry {
  timestamp: string | null;
  sandbox: string | null;
  host: string | null;
  action: string | null;
  proxy: string | null;
  rule: string | null;
  reason: string | null;
}

type SortColumn = "provenance" | "origin" | "id" | "rule_type" | "action" | "target";

// Initial column widths in px: Provenance, AppliesTo, Policy/Rule, Type, Decision, Resources, Actions
const DEFAULT_COL_WIDTHS = [96, 128, 220, 88, 88, 220, 80];

function getRuleVal(rule: PolicyRule, col: SortColumn): string {
  switch (col) {
    case "provenance": return rule.provenance || "";
    case "origin":     return rule.origin || "";
    case "id":         return rule.id || "";
    case "rule_type":  return rule.rule_type || "";
    case "action":     return rule.action;
    case "target":     return rule.target;
  }
}

export function PoliciesPage() {
  const [policyState, setPolicyState] = useState<PolicyState | null>(null);
  const [logEntries, setLogEntries] = useState<PolicyLogEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [ruleAction, setRuleAction] = useState("allow");
  const [ruleTarget, setRuleTarget] = useState("");
  const [ruleSearch, setRuleSearch] = useState("");
  const [sandboxFilter, setSandboxFilter] = useState("");
  const [view, setView] = useState<"rules" | "log">("rules");
  const [autoRefresh, setAutoRefresh] = useState(false);
  const [sortColumn, setSortColumn] = useState<SortColumn>("id");
  const [sortDirection, setSortDirection] = useState<"asc" | "desc">("asc");
  const [colWidths, setColWidths] = useState<number[]>(DEFAULT_COL_WIDTHS);

  // Stable refs so resize handlers don't need to be re-registered on every render
  const colWidthsRef = useRef(colWidths);
  useEffect(() => { colWidthsRef.current = colWidths; }, [colWidths]);

  const dragRef = useRef<{ colIdx: number; startX: number; startWidth: number } | null>(null);

  const startResize = useCallback((colIdx: number, e: React.MouseEvent) => {
    e.preventDefault();
    e.stopPropagation();
    dragRef.current = { colIdx, startX: e.clientX, startWidth: colWidthsRef.current[colIdx] ?? DEFAULT_COL_WIDTHS[colIdx] ?? 100 };
  }, []);

  useEffect(() => {
    const onMouseMove = (e: MouseEvent) => {
      if (!dragRef.current) return;
      const { colIdx, startX, startWidth } = dragRef.current;
      const newWidth = Math.max(50, startWidth + (e.clientX - startX));
      setColWidths(prev => {
        const next = [...prev];
        next[colIdx] = newWidth;
        return next;
      });
    };
    const onMouseUp = () => { dragRef.current = null; };
    document.addEventListener("mousemove", onMouseMove);
    document.addEventListener("mouseup", onMouseUp);
    return () => {
      document.removeEventListener("mousemove", onMouseMove);
      document.removeEventListener("mouseup", onMouseUp);
    };
  }, []);

  const fetchPolicies = useCallback(async () => {
    try {
      setLoading(true);
      const state = await api.get<PolicyState>("/api/policies");
      setPolicyState(state);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load policies");
    } finally {
      setLoading(false);
    }
  }, []);

  const fetchLog = useCallback(async () => {
    try {
      const params = new URLSearchParams();
      if (sandboxFilter) params.set("sandbox_id", sandboxFilter);
      params.set("limit", "100");
      const entries = await api.get<PolicyLogEntry[]>(`/api/policies/log?${params.toString()}`);
      setLogEntries(entries);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load traffic log");
    }
  }, [sandboxFilter]);

  useEffect(() => {
    fetchPolicies();
    if (!autoRefresh) return;
    const interval = setInterval(fetchPolicies, 30000);
    return () => clearInterval(interval);
  }, [fetchPolicies, autoRefresh]);

  useEffect(() => {
    if (view === "log") {
      fetchLog();
    }
  }, [view, fetchLog]);

  // TEMPORARILY DISABLED — FIX NEXT RELEASE (spec: default-policy-init-behavior).
  // sbx 0.34.0+ made `sbx policy init` (which this triggers via
  // PUT /api/policies/default) a one-time initialization. Re-invoking it to
  // switch the baseline either errors or resets the global policy, which could
  // silently wipe the user's custom rules. The Default Policy buttons are
  // disabled until the correct reset-then-init flow (with confirmation) lands.
  // Handler left in place (commented) so it can be restored without rewriting.
  // const handleSetDefault = async (mode: string) => {
  //   try {
  //     await api.put("/api/policies/default", { mode });
  //     await fetchPolicies();
  //   } catch (e) {
  //     setError(e instanceof Error ? e.message : "Failed to set default policy");
  //   }
  // };

  const handleAddRule = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!ruleTarget.trim()) return;
    try {
      await api.post("/api/policies/rules", { action: ruleAction, target: ruleTarget.trim() });
      setRuleTarget("");
      await fetchPolicies();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to add rule");
    }
  };

  const handleRemoveRule = async (ruleId: string) => {
    try {
      await api.del(`/api/policies/rules/${encodeURIComponent(ruleId)}`);
      await fetchPolicies();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to remove rule");
    }
  };

  const handleSort = (column: SortColumn) => {
    if (sortColumn === column) {
      setSortDirection(sortDirection === "asc" ? "desc" : "asc");
    } else {
      setSortColumn(column);
      setSortDirection("asc");
    }
  };

  const sortIndicator = (column: SortColumn) => {
    if (sortColumn !== column) return "";
    return sortDirection === "asc" ? " ▲" : " ▼";
  };

  if (loading) {
    return (
      <div>
        <h2>Policies</h2>
        <p>Loading...</p>
      </div>
    );
  }

  return (
    <div className="policies-page">
      <div className="page-header">
        <h2>Policies</h2>
        <div className="page-header-actions">
          <a href="/help" className="help-link" aria-label="Policies help">?</a>
        </div>
      </div>

      {error && <div className="alert alert-error" role="alert">{error}</div>}

      <nav className="tab-nav" aria-label="Policy sections">
        <button className={`tab-btn ${view === "rules" ? "active" : ""}`} onClick={() => setView("rules")}>
          Rules
        </button>
        <button className={`tab-btn ${view === "log" ? "active" : ""}`} onClick={() => setView("log")}>
          Traffic Log
        </button>
      </nav>

      {view === "rules" && policyState && (
        <div className="policy-rules-section">
          <div className="default-policy card">
            <h3>Default Policy</h3>
            <p>Current: <strong>{policyState.default_policy}</strong></p>
            {/* TEMPORARILY DISABLED — FIX NEXT RELEASE (spec: default-policy-init-behavior).
                onClick handlers commented out and buttons disabled to prevent
                triggering `sbx policy init` re-invocation, which could wipe custom
                policy rules on sbx 0.34.0+. Re-enable with the corrected flow. */}
            <div className="policy-mode-buttons">
              <button
                className={`btn btn-sm ${policyState.default_policy === "balanced" ? "btn-primary" : ""}`}
                /* onClick={() => handleSetDefault("balanced")} */
                disabled
                aria-label="Set balanced policy"
              >
                Balanced (recommended)
              </button>
              <button
                className={`btn btn-sm ${policyState.default_policy === "deny" ? "btn-primary" : ""}`}
                /* onClick={() => handleSetDefault("deny")} */
                disabled
                aria-label="Set deny policy"
              >
                Deny All
              </button>
              <button
                className={`btn btn-sm ${policyState.default_policy === "allow" ? "btn-primary" : ""}`}
                /* onClick={() => handleSetDefault("allow")} */
                disabled
                aria-label="Set allow policy"
              >
                Allow All
              </button>
            </div>
          </div>

          <div className="add-rule card">
            <h3>Add Rule</h3>
            <form className="rule-form" onSubmit={handleAddRule} aria-label="Add network rule">
              <select value={ruleAction} onChange={(e) => setRuleAction(e.target.value)} aria-label="Rule action">
                <option value="allow">Allow</option>
                <option value="deny">Deny</option>
              </select>
              <input
                type="text"
                value={ruleTarget}
                onChange={(e) => setRuleTarget(e.target.value)}
                placeholder="IP:PORT or domain (e.g., 127.0.0.1:8080)"
                aria-label="Rule target"
              />
              <button type="submit" className="btn btn-primary">Add Rule</button>
            </form>
          </div>

          <div className="rule-list">
            <div style={{ display: "flex", alignItems: "center", gap: "0.75rem", marginBottom: "0.5rem" }}>
              <h3 style={{ margin: 0 }}>Active Rules</h3>
              <button className="btn btn-sm" onClick={fetchPolicies} aria-label="Refresh policies">
                Refresh
              </button>
              <label style={{ display: "flex", alignItems: "center", gap: "0.25rem", fontSize: "0.8rem", marginLeft: "auto" }}>
                <input
                  type="checkbox"
                  checked={autoRefresh}
                  onChange={(e) => setAutoRefresh(e.target.checked)}
                />
                Auto-refresh
              </label>
            </div>
            <div className="rule-search">
              <input
                type="text"
                placeholder="Search rules by target..."
                value={ruleSearch}
                onChange={(e) => setRuleSearch(e.target.value)}
                aria-label="Search policy rules"
              />
            </div>
            {policyState.rules.length === 0 ? (
              <p className="empty-state">No custom rules configured.</p>
            ) : (
              <table className="rules-table" aria-label="Policy rules">
                <colgroup>
                  {colWidths.map((w, i) => <col key={i} style={{ width: w }} />)}
                </colgroup>
                <thead>
                  <tr>
                    {(["provenance", "origin", "id", "rule_type", "action", "target"] as SortColumn[]).map((col, i) => {
                      const labels: Record<SortColumn, string> = {
                        provenance: "Provenance",
                        origin: "Applies To",
                        id: "Policy/Rule",
                        rule_type: "Type",
                        action: "Decision",
                        target: "Resources",
                      };
                      return (
                        <th key={col} style={{ position: "relative", userSelect: "none" }}>
                          <span
                            onClick={() => handleSort(col)}
                            style={{ cursor: "pointer", display: "inline-block", width: "calc(100% - 8px)" }}
                          >
                            {labels[col]}{sortIndicator(col)}
                          </span>
                          <span
                            className="col-resize-handle"
                            onMouseDown={(e) => startResize(i, e)}
                          />
                        </th>
                      );
                    })}
                    <th style={{ position: "relative" }}></th>
                  </tr>
                </thead>
                <tbody>
                  {policyState.rules
                    .filter((rule) => {
                      if (!ruleSearch) return true;
                      const search = ruleSearch.toLowerCase();
                      return (
                        rule.target.toLowerCase().includes(search) ||
                        (rule.id && rule.id.toLowerCase().includes(search)) ||
                        rule.action.toLowerCase().includes(search)
                      );
                    })
                    .sort((a, b) => {
                      const cmp = getRuleVal(a, sortColumn).toLowerCase()
                        .localeCompare(getRuleVal(b, sortColumn).toLowerCase());
                      return sortDirection === "asc" ? cmp : -cmp;
                    })
                    .map((rule, i) => (
                    <tr key={`${rule.id}-${i}`}>
                      <td><span className="rule-provenance">{rule.provenance || "—"}</span></td>
                      <td>
                        {rule.origin && rule.origin !== "all"
                          ? <code className="rule-scope">{rule.origin.startsWith("sandbox:") ? rule.origin.slice("sandbox:".length) : rule.origin}</code>
                          : <span className="rule-scope-global">all</span>
                        }
                      </td>
                      <td><code className="rule-name">{rule.id || "—"}</code></td>
                      <td><span className="rule-type">{rule.rule_type || "—"}</span></td>
                      <td><span className={`badge badge-${rule.action}`}>{rule.action}</span></td>
                      <td><code>{rule.target}</code></td>
                      <td>
                        {rule.id && !rule.id.startsWith("default-") && (
                          <button className="btn btn-sm btn-danger" onClick={() => handleRemoveRule(rule.id!)} aria-label={`Remove rule for ${rule.target}`}>
                            Remove
                          </button>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        </div>
      )}

      {view === "log" && (
        <div className="traffic-log-section">
          <div className="log-filter">
            <label htmlFor="sandbox-filter">Filter by sandbox:</label>
            <input
              id="sandbox-filter"
              type="text"
              value={sandboxFilter}
              onChange={(e) => setSandboxFilter(e.target.value)}
              placeholder="Sandbox ID (optional)"
              aria-label="Filter traffic log by sandbox"
            />
            <button className="btn btn-sm" onClick={fetchLog}>Refresh</button>
          </div>

          {logEntries.length === 0 ? (
            <p className="empty-state">No traffic log entries.</p>
          ) : (
            <table className="log-table" aria-label="Traffic log">
              <thead>
                <tr>
                  <th>Time</th>
                  <th>Sandbox</th>
                  <th>Host</th>
                  <th>Action</th>
                  <th>Proxy</th>
                  <th>Rule</th>
                </tr>
              </thead>
              <tbody>
                {logEntries.map((entry, i) => (
                  <tr key={i}>
                    <td>{entry.timestamp || "—"}</td>
                    <td>{entry.sandbox || "—"}</td>
                    <td><code>{entry.host || "—"}</code></td>
                    <td><span className={`badge badge-${entry.action}`}>{entry.action || "—"}</span></td>
                    <td>{entry.proxy || "—"}</td>
                    <td>{entry.rule || "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      )}
    </div>
  );
}
