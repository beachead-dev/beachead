import { NavLink } from "react-router-dom";

const navItems = [
  { to: "/personas", label: "Personas" },
  { to: "/agents", label: "Agents" },
  { to: "/sessions", label: "Sessions" },
  { to: "/policies", label: "Policies" },
  { to: "/settings", label: "System Settings" },
];

export function Sidebar() {
  return (
    <nav className="sidebar" aria-label="Main navigation">
      <div className="sidebar-header">
        <h1 className="sidebar-title">Beachead</h1>
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
      </div>
    </nav>
  );
}
