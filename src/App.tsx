import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import { Layout } from "./components/Layout";
import { PersonasPage } from "./pages/PersonasPage";
import { AgentsPage } from "./pages/AgentsPage";
import { PoliciesPage } from "./pages/PoliciesPage";
import { RepoSyncPage } from "./pages/RepoSyncPage";
import { DockerPage } from "./pages/DockerPage";
import { HelpPage } from "./pages/HelpPage";
import { SystemSettingsPage } from "./pages/SystemSettingsPage";

function App() {
  return (
    <BrowserRouter>
      <Routes>
        <Route element={<Layout />}>
          <Route path="/" element={<Navigate to="/sessions" replace />} />
          <Route path="/sessions" element={null} />
          <Route path="/repo-sync" element={<RepoSyncPage />} />
          <Route path="/personas" element={<PersonasPage />} />
          <Route path="/agents" element={<AgentsPage />} />
          <Route path="/policies" element={<PoliciesPage />} />
          <Route path="/docker" element={<DockerPage />} />
          <Route path="/help" element={<HelpPage />} />
          <Route path="/settings" element={<SystemSettingsPage />} />
        </Route>
      </Routes>
    </BrowserRouter>
  );
}

export default App;
