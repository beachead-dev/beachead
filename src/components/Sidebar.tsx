import { useRef, useState, useEffect } from "react";
import { NavLink } from "react-router-dom";
import { ResizeHandle } from "./ResizeHandle";
import { api } from "../lib/api";
import logoDark from "../assets/logo-dark.png";
import iconDark from "../assets/icon-dark.png";

const navItems = [
  { to: "/sessions", label: "Sessions" },
  { to: "/personas", label: "Personas" },
  { to: "/agents", label: "Agents" },
  { to: "/policies", label: "Policies" },
  { to: "/settings", label: "System Settings" },
];

type Theme = "light" | "dark" | "system";

function applyTheme(theme: Theme) {
  if (theme === "dark") {
    document.documentElement.setAttribute("data-theme", "dark");
  } else if (theme === "light") {
    document.documentElement.setAttribute("data-theme", "light");
  } else {
    document.documentElement.removeAttribute("data-theme");
  }
}

export function Sidebar() {
  const sidebarRef = useRef<HTMLElement>(null);
  const [theme, setTheme] = useState<Theme>("system");

  // Load saved theme on mount
  useEffect(() => {
    api.get<{ key: string; value: string }>("/api/system/settings/theme")
      .then((res) => {
        const saved = res.value as Theme;
        if (saved === "light" || saved === "dark" || saved === "system") {
          setTheme(saved);
          applyTheme(saved);
        }
      })
      .catch(() => {
        // No saved preference — use system default
        applyTheme("system");
      });
  }, []);

  const handleThemeChange = async (newTheme: Theme) => {
    setTheme(newTheme);
    applyTheme(newTheme);
    try {
      await api.put("/api/system/settings/theme", { value: newTheme });
    } catch {
      // Non-critical — theme still applied locally
    }
  };

  return (
    <nav className="sidebar" aria-label="Main navigation" ref={sidebarRef}>
      <div className="sidebar-header">
        <img
          src={logoDark}
          alt="Beachead"
          className="sidebar-logo"
        />
        <img
          src={iconDark}
          alt="Beachead"
          className="sidebar-icon"
        />
      </div>
      <ul className="sidebar-nav">
        {navItems.map((item) => (
          <li key={item.to}>
            <NavLink
              to={item.to}
              className={({ isActive }) =>
                `sidebar-link${isActive ? " sidebar-link--active" : ""}`
              }
            >
              {item.label}
            </NavLink>
          </li>
        ))}
      </ul>
      <div className="sidebar-footer">
        <NavLink
          to="/help"
          className={({ isActive }) =>
            `sidebar-help-link${isActive ? " sidebar-help-link--active" : ""}`
          }
          aria-label="Help and documentation"
        >
          <span className="sidebar-help-icon" aria-hidden="true">?</span>
          Help
        </NavLink>
        <div className="theme-toggle" aria-label="Theme selector">
          <button
            className={`theme-toggle-btn ${theme === "light" ? "active" : ""}`}
            onClick={() => handleThemeChange("light")}
            aria-label="Light theme"
            title="Light"
          >
            ☀
          </button>
          <button
            className={`theme-toggle-btn ${theme === "system" ? "active" : ""}`}
            onClick={() => handleThemeChange("system")}
            aria-label="System theme"
            title="System"
          >
            ◐
          </button>
          <button
            className={`theme-toggle-btn ${theme === "dark" ? "active" : ""}`}
            onClick={() => handleThemeChange("dark")}
            aria-label="Dark theme"
            title="Dark"
          >
            ☾
          </button>
        </div>
      </div>
      <ResizeHandle targetRef={sidebarRef} minWidth={120} maxWidth={280} />
    </nav>
  );
}
