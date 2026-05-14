import { useEffect, useRef } from "react";

export interface SecretScanFinding {
  filePath: string;
  patternName: string;
}

export interface SecretScanWarningModalProps {
  open: boolean;
  findings: SecretScanFinding[];
  onDismiss: () => void;
}

/**
 * Parses a secret scan error message from the backend into structured findings.
 * Expected format: "Secret scan detected potential secrets: file1: pattern1; file2: pattern2"
 */
export function parseSecretScanError(errorMessage: string): SecretScanFinding[] | null {
  const prefix = "Secret scan detected potential secrets: ";
  if (!errorMessage.startsWith(prefix)) {
    return null;
  }

  const findingsStr = errorMessage.slice(prefix.length);
  if (!findingsStr.trim()) {
    return [];
  }

  const parts = findingsStr.split("; ");
  return parts.map((part) => {
    const colonIdx = part.indexOf(": ");
    if (colonIdx === -1) {
      return { filePath: "", patternName: part.trim() };
    }
    return {
      filePath: part.slice(0, colonIdx),
      patternName: part.slice(colonIdx + 2),
    };
  });
}

/**
 * Modal displayed when a push-to-remote operation is rejected due to
 * secret scan findings (block mode). Shows the list of detected secrets
 * and allows the user to dismiss.
 */
export function SecretScanWarningModal({
  open,
  findings,
  onDismiss,
}: SecretScanWarningModalProps) {
  const dismissBtnRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open && dismissBtnRef.current) {
      dismissBtnRef.current.focus();
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        onDismiss();
      }
    }
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [open, onDismiss]);

  if (!open) return null;

  function handleBackdropClick(e: React.MouseEvent) {
    if (e.target === e.currentTarget) {
      onDismiss();
    }
  }

  return (
    <div
      className="modal-overlay"
      onClick={handleBackdropClick}
      role="dialog"
      aria-modal="true"
      aria-labelledby="secret-scan-warning-title"
    >
      <div className="modal secret-scan-warning-modal">
        <h3 id="secret-scan-warning-title" className="secret-scan-warning-heading">
          ⚠ Push Blocked — Secrets Detected
        </h3>
        <p className="secret-scan-warning-description">
          The push was rejected because potential secrets were found in the
          selected commits. Remove or exclude these files before pushing.
        </p>
        <div className="secret-scan-findings-list" role="list" aria-label="Secret scan findings">
          {findings.map((finding, idx) => (
            <div key={idx} className="secret-scan-finding" role="listitem">
              {finding.filePath ? (
                <>
                  <code className="secret-scan-finding-file">{finding.filePath}</code>
                  <span className="secret-scan-finding-pattern">{finding.patternName}</span>
                </>
              ) : (
                <span className="secret-scan-finding-pattern">{finding.patternName}</span>
              )}
            </div>
          ))}
        </div>
        <div className="modal-actions">
          <button
            className="btn btn-primary"
            onClick={onDismiss}
            ref={dismissBtnRef}
            type="button"
          >
            OK
          </button>
        </div>
      </div>
    </div>
  );
}
