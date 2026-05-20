import { useEffect, useState } from "react";
import { Link, Outlet, useLocation } from "react-router-dom";
import { Sidebar } from "./Sidebar";
import { SessionsPage } from "../pages/SessionsPage";
import { api } from "../lib/api";

interface DependencyStatus {
  sbx_available: boolean;
  docker_available: boolean;
}

export function Layout() {
  const location = useLocation();
  const isSessionsRoute = location.pathname === "/sessions";
  const [missingDeps, setMissingDeps] = useState<string[]>([]);

  useEffect(() => {
    api.get<DependencyStatus>("/api/system/dependency-check")
      .then(deps => {
        const missing: string[] = [];
        if (!deps.docker_available) missing.push("Docker");
        if (!deps.sbx_available) missing.push("sbx CLI");
        setMissingDeps(missing);
      })
      .catch(() => {});
  }, []);

  return (
    <div className="app">
      <Sidebar />
      <main className="main-content">
        {missingDeps.length > 0 && (
          <div className="alert alert-warning" role="alert" style={{ margin: "0.75rem", flexShrink: 0 }}>
            <strong>Missing dependencies:</strong> {missingDeps.join(" and ")} not found — Beachhead will not work correctly.{" "}
            <Link to="/settings">Open System Settings →</Link>
          </div>
        )}
        {/* Sessions page is always mounted to preserve terminal state */}
        <div style={{ display: isSessionsRoute ? "flex" : "none", flexDirection: "column", flex: 1, height: "100%", minHeight: 0 }}>
          <SessionsPage />
        </div>
        {/* Other pages render via Outlet (unmount on navigation) */}
        {!isSessionsRoute && <Outlet />}
      </main>
    </div>
  );
}
