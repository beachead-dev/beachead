import { useState, useEffect } from "react";
import ReactMarkdown from "react-markdown";
import { api } from "../lib/api";

interface HelpTopic {
  id: string;
  label: string;
}

interface AppVersion {
  version: string;
}

const HELP_TOPICS: HelpTopic[] = [
  { id: "getting-started", label: "Getting Started" },
  { id: "personas", label: "Personas" },
  { id: "agents", label: "Agents" },
  { id: "sessions", label: "Sessions" },
  { id: "repo-sync", label: "Repo Sync" },
  { id: "policies", label: "Policies" },
  { id: "docker", label: "Docker" },
  { id: "system-settings", label: "System Settings" },
  { id: "troubleshooting", label: "Troubleshooting" },
  { id: "glossary", label: "Glossary" },
];

export function HelpPage() {
  const [activeTopic, setActiveTopic] = useState("getting-started");
  const [content, setContent] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [appVersion, setAppVersion] = useState<string | null>(null);

  useEffect(() => {
    api
      .get<AppVersion>("/api/system/app-version")
      .then((data) => setAppVersion(data.version))
      .catch(() => {});
  }, []);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    api
      .getText(`/api/system/help/${activeTopic}`)
      .then((data) => {
        if (!cancelled) {
          setContent(data);
          setLoading(false);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setError(err.message || "Failed to load help content");
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [activeTopic]);

  return (
    <div className="help-page">
      <nav className="help-sidebar" aria-label="Help topics">
        <h2 className="help-sidebar-title">Documentation</h2>
        <ul className="help-topic-list">
          {HELP_TOPICS.map((topic) => (
            <li key={topic.id}>
              <button
                className={`help-topic-btn${
                  activeTopic === topic.id ? " help-topic-btn--active" : ""
                }`}
                onClick={() => setActiveTopic(topic.id)}
                aria-current={activeTopic === topic.id ? "page" : undefined}
              >
                {topic.label}
              </button>
            </li>
          ))}
        </ul>
        {appVersion && (
          <p className="help-version">Beachead v{appVersion}</p>
        )}
      </nav>
      <main className="help-content" aria-live="polite">
        {loading && <p className="help-loading">Loading...</p>}
        {error && <p className="help-error">{error}</p>}
        {!loading && !error && (
          <div className="help-markdown">
            <ReactMarkdown>{content}</ReactMarkdown>
          </div>
        )}
      </main>
    </div>
  );
}
