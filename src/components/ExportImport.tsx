import { useState, useRef } from "react";
import { api, ApiError } from "../lib/api";

// --- Types ---

interface PersonaPreview {
  name: string;
  has_conflict: boolean;
}

interface ImportPreview {
  personas: PersonaPreview[];
  missing_secrets: string[];
  invalid_workspaces: string[];
}

interface ImportSummary {
  personas_imported: number;
  personas_skipped: number;
  personas_renamed: number;
  errors: string[];
}

type ResolutionAction = "Skip" | "Overwrite" | { action: "Rename"; new_name: string };

interface PersonaResolutions {
  [name: string]: { action: ResolutionAction };
}

// --- Component ---

export function ExportImport() {
  // Export state
  const [exportPassword, setExportPassword] = useState("");
  const [showExportForm, setShowExportForm] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [exportError, setExportError] = useState<string | null>(null);

  // Import state
  const [importPassword, setImportPassword] = useState("");
  const [showImportForm, setShowImportForm] = useState(false);
  const [importFile, setImportFile] = useState<File | null>(null);
  const [importing, setImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);

  // Preview state
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [resolutions, setResolutions] = useState<PersonaResolutions>({});
  const [renameValues, setRenameValues] = useState<Record<string, string>>({});

  // Summary state
  const [summary, setSummary] = useState<ImportSummary | null>(null);

  // Confirming import
  const [confirming, setConfirming] = useState(false);

  const fileInputRef = useRef<HTMLInputElement>(null);

  // --- Export ---

  const handleExport = async () => {
    if (!exportPassword) return;
    setExporting(true);
    setExportError(null);
    try {
      const blob = await api.postForBlob("/api/export", { password: exportPassword });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `beachead-export-${new Date().toISOString().slice(0, 10)}.beachead`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
      setShowExportForm(false);
      setExportPassword("");
    } catch (e) {
      setExportError(e instanceof ApiError ? e.message : "Export failed");
    } finally {
      setExporting(false);
    }
  };

  // --- Import: file selection ---

  const handleFileSelect = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0] ?? null;
    setImportFile(file);
    setPreview(null);
    setSummary(null);
    setImportError(null);
    setResolutions({});
    setRenameValues({});
  };

  const handleImportPreview = async () => {
    if (!importFile || !importPassword) return;
    setImporting(true);
    setImportError(null);
    setPreview(null);
    setSummary(null);
    try {
      const base64 = await fileToBase64(importFile);
      const result = await api.post<ImportPreview>("/api/import/preview", {
        data: base64,
        password: importPassword,
      });
      setPreview(result);
      // Initialize resolutions for conflicts
      const initialResolutions: PersonaResolutions = {};
      for (const p of result.personas) {
        if (p.has_conflict) {
          initialResolutions[p.name] = { action: "Skip" };
        }
      }
      setResolutions(initialResolutions);
    } catch (e) {
      setImportError(e instanceof ApiError ? e.message : "Failed to preview import");
    } finally {
      setImporting(false);
    }
  };

  // --- Import: confirm ---

  const handleConfirmImport = async () => {
    if (!importFile || !importPassword) return;
    setConfirming(true);
    setImportError(null);
    try {
      const base64 = await fileToBase64(importFile);
      // Build resolutions payload
      const personaResolutions: Record<string, ResolutionAction> = {};
      for (const [name, res] of Object.entries(resolutions)) {
        if (typeof res.action === "object" && res.action.action === "Rename") {
          personaResolutions[name] = { action: "Rename", new_name: renameValues[name] || name };
        } else {
          personaResolutions[name] = res.action as "Skip" | "Overwrite";
        }
      }
      const result = await api.post<ImportSummary>("/api/import", {
        data: base64,
        password: importPassword,
        resolutions: { persona_resolutions: personaResolutions },
      });
      setSummary(result);
      setPreview(null);
    } catch (e) {
      setImportError(e instanceof ApiError ? e.message : "Import failed");
    } finally {
      setConfirming(false);
    }
  };

  // --- Resolution helpers ---

  const setResolutionAction = (name: string, action: "Skip" | "Overwrite" | "Rename") => {
    if (action === "Rename") {
      setResolutions((prev) => ({ ...prev, [name]: { action: { action: "Rename", new_name: renameValues[name] || "" } } }));
    } else {
      setResolutions((prev) => ({ ...prev, [name]: { action } }));
    }
  };

  const setRenameName = (name: string, newName: string) => {
    setRenameValues((prev) => ({ ...prev, [name]: newName }));
    setResolutions((prev) => ({ ...prev, [name]: { action: { action: "Rename", new_name: newName } } }));
  };

  const resetImport = () => {
    setShowImportForm(false);
    setImportFile(null);
    setImportPassword("");
    setPreview(null);
    setSummary(null);
    setImportError(null);
    setResolutions({});
    setRenameValues({});
    if (fileInputRef.current) fileInputRef.current.value = "";
  };

  return (
    <section className="settings-section card" aria-label="Export / Import Configuration">
      <h3>Export / Import Configuration</h3>
      <p className="section-description">
        Export your personas and agent configurations to a password-protected file, or import from a previous export.
      </p>

      {/* Export */}
      <div style={{ marginBottom: "1rem" }}>
        {!showExportForm ? (
          <button className="btn btn-primary" onClick={() => setShowExportForm(true)} aria-label="Export configuration">
            Export Configuration
          </button>
        ) : (
          <div className="export-form">
            <div className="form-group">
              <label htmlFor="export-password">Encryption Password</label>
              <div className="input-with-button">
                <input
                  id="export-password"
                  type="password"
                  value={exportPassword}
                  onChange={(e) => setExportPassword(e.target.value)}
                  placeholder="Enter password to encrypt export"
                  onKeyDown={(e) => { if (e.key === "Enter") handleExport(); }}
                />
                <button className="btn btn-primary" onClick={handleExport} disabled={exporting || !exportPassword}>
                  {exporting ? "Exporting..." : "Download"}
                </button>
                <button className="btn" onClick={() => { setShowExportForm(false); setExportPassword(""); setExportError(null); }}>
                  Cancel
                </button>
              </div>
            </div>
            {exportError && <div className="alert alert-error" role="alert">{exportError}</div>}
          </div>
        )}
      </div>

      {/* Import */}
      <div>
        {!showImportForm ? (
          <button className="btn" onClick={() => setShowImportForm(true)} aria-label="Import configuration">
            Import Configuration
          </button>
        ) : (
          <div className="import-form">
            <div className="form-group">
              <label htmlFor="import-file">Select .beachead file</label>
              <input
                id="import-file"
                ref={fileInputRef}
                type="file"
                accept=".beachead"
                onChange={handleFileSelect}
                style={{ fontSize: "0.875rem" }}
              />
            </div>

            {importFile && (
              <div className="form-group">
                <label htmlFor="import-password">Decryption Password</label>
                <div className="input-with-button">
                  <input
                    id="import-password"
                    type="password"
                    value={importPassword}
                    onChange={(e) => setImportPassword(e.target.value)}
                    placeholder="Enter password used during export"
                    onKeyDown={(e) => { if (e.key === "Enter") handleImportPreview(); }}
                  />
                  <button className="btn btn-primary" onClick={handleImportPreview} disabled={importing || !importPassword}>
                    {importing ? "Loading..." : "Preview"}
                  </button>
                </div>
              </div>
            )}

            {importError && <div className="alert alert-error" role="alert">{importError}</div>}

            {/* Preview */}
            {preview && (
              <div className="import-preview" style={{ marginTop: "1rem" }}>
                <h4 style={{ margin: "0 0 0.5rem" }}>Import Preview</h4>

                {/* Personas */}
                <div style={{ marginBottom: "0.75rem" }}>
                  <strong style={{ fontSize: "0.875rem" }}>Personas ({preview.personas.length})</strong>
                  {preview.personas.length === 0 && <p className="empty-state">No personas in export.</p>}
                  <ul style={{ listStyle: "none", padding: 0, margin: "0.25rem 0" }}>
                    {preview.personas.map((p) => (
                      <li key={p.name} style={{ padding: "0.375rem 0", borderBottom: "1px solid #f0f0f0" }}>
                        <span style={{ fontWeight: 500, fontSize: "0.875rem" }}>{p.name}</span>
                        {p.has_conflict && (
                          <span className="badge badge-denied" style={{ marginLeft: "0.5rem" }}>Conflict</span>
                        )}
                        {p.has_conflict && (
                          <div style={{ marginTop: "0.375rem", display: "flex", gap: "0.5rem", alignItems: "center", flexWrap: "wrap" }}>
                            <select
                              value={getResolutionType(resolutions[p.name]?.action)}
                              onChange={(e) => setResolutionAction(p.name, e.target.value as "Skip" | "Overwrite" | "Rename")}
                              style={{ padding: "0.25rem 0.5rem", fontSize: "0.8125rem", borderRadius: "0.25rem", border: "1px solid #ddd" }}
                              aria-label={`Resolution for ${p.name}`}
                            >
                              <option value="Skip">Skip</option>
                              <option value="Overwrite">Overwrite</option>
                              <option value="Rename">Rename</option>
                            </select>
                            {getResolutionType(resolutions[p.name]?.action) === "Rename" && (
                              <input
                                type="text"
                                value={renameValues[p.name] || ""}
                                onChange={(e) => setRenameName(p.name, e.target.value)}
                                placeholder="New name"
                                style={{ padding: "0.25rem 0.5rem", fontSize: "0.8125rem", borderRadius: "0.25rem", border: "1px solid #ddd", width: "160px" }}
                                aria-label={`New name for ${p.name}`}
                              />
                            )}
                          </div>
                        )}
                      </li>
                    ))}
                  </ul>
                </div>

                {/* Missing secrets */}
                {preview.missing_secrets.length > 0 && (
                  <div style={{ marginBottom: "0.75rem" }}>
                    <strong style={{ fontSize: "0.875rem" }}>Missing Secrets</strong>
                    <ul style={{ margin: "0.25rem 0", paddingLeft: "1.25rem", fontSize: "0.8125rem" }}>
                      {preview.missing_secrets.map((s) => (
                        <li key={s}>{s}</li>
                      ))}
                    </ul>
                  </div>
                )}

                {/* Invalid workspaces */}
                {preview.invalid_workspaces.length > 0 && (
                  <div style={{ marginBottom: "0.75rem" }}>
                    <strong style={{ fontSize: "0.875rem" }}>Invalid Workspaces</strong>
                    <ul style={{ margin: "0.25rem 0", paddingLeft: "1.25rem", fontSize: "0.8125rem" }}>
                      {preview.invalid_workspaces.map((w) => (
                        <li key={w}>{w}</li>
                      ))}
                    </ul>
                  </div>
                )}

                {/* Confirm button */}
                <div className="form-actions">
                  <button className="btn btn-primary" onClick={handleConfirmImport} disabled={confirming} aria-label="Confirm import">
                    {confirming ? "Importing..." : "Confirm Import"}
                  </button>
                  <button className="btn" onClick={resetImport}>Cancel</button>
                </div>
              </div>
            )}

            {/* Summary */}
            {summary && (
              <div className="import-summary" style={{ marginTop: "1rem" }}>
                <h4 style={{ margin: "0 0 0.5rem" }}>Import Complete</h4>
                <dl className="detail-list">
                  <dt>Imported</dt>
                  <dd>{summary.personas_imported}</dd>
                  <dt>Skipped</dt>
                  <dd>{summary.personas_skipped}</dd>
                  <dt>Renamed</dt>
                  <dd>{summary.personas_renamed}</dd>
                </dl>
                {summary.errors.length > 0 && (
                  <div className="alert alert-error" role="alert" style={{ marginTop: "0.5rem" }}>
                    {summary.errors.map((err, i) => <div key={i}>{err}</div>)}
                  </div>
                )}
                <div className="form-actions">
                  <button className="btn" onClick={resetImport}>Done</button>
                </div>
              </div>
            )}

            {/* Cancel when no preview/summary shown */}
            {!preview && !summary && (
              <div className="form-actions">
                <button className="btn" onClick={resetImport}>Cancel</button>
              </div>
            )}
          </div>
        )}
      </div>
    </section>
  );
}

// --- Helpers ---

function fileToBase64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result as string;
      // Strip the data URL prefix (e.g. "data:application/octet-stream;base64,")
      const base64 = result.includes(",") ? result.split(",")[1] : result;
      resolve(base64 ?? "");
    };
    reader.onerror = () => reject(new Error("Failed to read file"));
    reader.readAsDataURL(file);
  });
}

function getResolutionType(action: ResolutionAction | undefined): "Skip" | "Overwrite" | "Rename" {
  if (!action) return "Skip";
  if (action === "Skip") return "Skip";
  if (action === "Overwrite") return "Overwrite";
  if (typeof action === "object" && action.action === "Rename") return "Rename";
  return "Skip";
}
