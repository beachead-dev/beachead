import { Outlet, useLocation } from "react-router-dom";
import { Sidebar } from "./Sidebar";
import { SessionsPage } from "../pages/SessionsPage";

export function Layout() {
  const location = useLocation();
  const isSessionsRoute = location.pathname === "/sessions";

  return (
    <div className="app">
      <Sidebar />
      <main className="main-content">
        {/* Sessions page is always mounted to preserve terminal state */}
        <div style={{ display: isSessionsRoute ? "flex" : "none", flexDirection: "column", flex: 1, height: "100%" }}>
          <SessionsPage />
        </div>
        {/* Other pages render via Outlet (unmount on navigation) */}
        {!isSessionsRoute && <Outlet />}
      </main>
    </div>
  );
}
