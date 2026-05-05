import { BrowserRouter, Routes, Route } from "react-router-dom";

function App() {
  return (
    <BrowserRouter>
      <div className="app">
        <main>
          <Routes>
            <Route path="/" element={<div>Beachead - Secure AI Orchestrator</div>} />
          </Routes>
        </main>
      </div>
    </BrowserRouter>
  );
}

export default App;
