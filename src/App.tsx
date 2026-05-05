import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import { Layout } from "./components/Layout";
import { PersonasPage } from "./pages/PersonasPage";
import { AgentsPage } from "./pages/AgentsPage";
import { SessionsPage } from "./pages/SessionsPage";
import { PoliciesPage } from "./pages/PoliciesPage";
import { HelpPage } from "./pages/HelpPage";
import { SystemSettingsPage } from "./pages/SystemSettingsPage";

function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route element={<Layout />}>
          <Route path="/" element={<Navigate to="/personas" replace />} />
          <Route path="/personas" element={<PersonasPage />} />
          <Route path="/agents" element={<AgentsPage />} />
          <Route path="/sessions" element={<SessionsPage />} />
          <Route path="/policies" element={<PoliciesPage />} />
          <Route path="/help" element={<HelpPage />} />
          <Route path="/settings" element={<SystemSettingsPage />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
}

export default App;
