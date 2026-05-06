import { useEffect, useState, useCallback } from "react";
import { api } from "../lib/api";
import { ExportImport } from "../components/ExportImport";

interface SbxVersion {
  version: string;
}

interface AuthStatus {
  authenticated: boolean;
  username?: string;
}

interface DiagnoseResult {
  raw_output: string;
  json: unknown | null;
}

interface DependencyStatus {
  sbx_available: boolean;
  sbx_version: string | null;
  docker_available: boolean;
  docker_version: string | null;
}

export function SystemSettingsPage() {
  const [version, setVersion] = useState<SbxVersion | null>(null);
  const [authStatus, setAuthStatus] = useState<AuthStatus | null>(null);
  const [dependencies, setDependencies] = useState<DependencyStatus | null>(null);
  const [diagnoseResult, setDiagnoseResult] = useState<DiagnoseResult | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [diagnosing, setDiagnosing] = useState(false);
  const [loggingIn, setLoggingIn] = useState(false);
  const [loggingOut, setLoggingOut] = useState(false);

  const fetchData = useCallback(async () => {
    try {
      setLoading(true);
      const [ver, auth, deps] = await Promise.all([
        api.get<SbxVersion>("/api/system/version").catch(() => null),
        api.get<AuthStatus>("/api/system/auth-status").catch(() => null),
        api.get<DependencyStatus>("/api/system/dependency-check").catch(() => null),
      ]);
      setVersion(ver);
      setAuthStatus(auth);
      setDependencies(deps);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Failed to load system info");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    fetchData();
  }, [fetchData]);

  const handleLogin = async () => {
    setLoggingIn(true);
    try {
      await api.post("/api/system/login");
      await fetchData();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Login failed");
    } finally {
      setLoggingIn(false);
    }
  };

  const handleLogout = async () => {
    setLoggingOut(true);
    try {
      await api.post("/api/system/logout");
      await fetchData();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Logout failed");
    } finally {
      setLoggingOut(false);
    }
  };

  const handleDiagnose = async () => {
    setDiagnosing(true);
    setDiagnoseResult(null);
    try {
      const result = await api.get<DiagnoseResult>("/api/system/diagnose");
      setDiagnoseResult(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Diagnostics failed");
    } finally {
      setDiagnosing(false);
    }
  };

  if (loading) {
    return (
      <div>
        <h2>System Settings</h2>
        <p>Loading...</p>
      </div>
    );
  }

  return (
    <div className="system-settings-page">
      <div className="page-header">
        <h2>System Settings</h2>
        <div className="page-header-actions">
          <a href="/help" className="help-link" aria-label="System settings help">?</a>
        </div>
      </div>

      {error && <div className="alert alert-error" role="alert">{error}</div>}

      {authStatus && !authStatus.authenticated && (
        <div className="alert alert-warning" role="alert">
          Docker authentication is not active. Sign in to use Docker Sandboxes.
        </div>
      )}

      <section className="settings-section card" aria-label="Docker Authentication">
        <h3>Docker Authentication</h3>
        <dl className="detail-list">
          <dt>Status</dt>
          <dd>
            <span className={`status-dot ${authStatus?.authenticated ? "status-ok" : "status-missing"}`} />
            {authStatus?.authenticated ? "Authenticated" : "Not authenticated"}
          </dd>
          {authStatus?.username && (
            <>
              <dt>Username</dt>
              <dd>{authStatus.username}</dd>
            </>
          )}
        </dl>
        <div className="section-actions">
          {!authStatus?.authenticated ? (
            <button className="btn btn-primary" onClick={handleLogin} disabled={loggingIn} aria-label="Sign in to Docker">
              {loggingIn ? "Signing in..." : "Sign In"}
            </button>
          ) : (
            <button className="btn" onClick={handleLogout} disabled={loggingOut} aria-label="Sign out of Docker">
              {loggingOut ? "Signing out..." : "Sign Out"}
            </button>
          )}
        </div>
      </section>

      <section className="settings-section card" aria-label="Version Information">
        <h3>Version</h3>
        <dl className="detail-list">
          <dt>sbx CLI</dt>
          <dd>{version?.version || "Not available"}</dd>
        </dl>
      </section>

      <section className="settings-section card" aria-label="Dependency Check">
        <h3>Dependencies</h3>
        {dependencies ? (
          <dl className="detail-list">
            <dt>sbx CLI</dt>
            <dd>
              <span className={`status-dot ${dependencies.sbx_available ? "status-ok" : "status-missing"}`} />
              {dependencies.sbx_available ? `Available (${dependencies.sbx_version})` : "Not found"}
            </dd>
            <dt>Docker Engine</dt>
            <dd>
              <span className={`status-dot ${dependencies.docker_available ? "status-ok" : "status-missing"}`} />
              {dependencies.docker_available ? `Available (${dependencies.docker_version})` : "Not found"}
            </dd>
          </dl>
        ) : (
          <p>Unable to check dependencies.</p>
        )}
      </section>

      <section className="settings-section card" aria-label="Diagnostics">
        <h3>Diagnostics</h3>
        <p className="section-description">Run sbx diagnostics to check system health and configuration.</p>
        <button className="btn" onClick={handleDiagnose} disabled={diagnosing} aria-label="Run diagnostics">
          {diagnosing ? "Running..." : "Run Diagnostics"}
        </button>
        {diagnoseResult && (
          <pre className="diagnose-output" aria-label="Diagnostics output">
            {diagnoseResult.raw_output}
          </pre>
        )}
      </section>

      <ExportImport />
    </div>
  );
}
