import { describe, it, expect } from "vitest";
import { parseSecretScanError } from "./SecretScanWarningModal";

describe("parseSecretScanError", () => {
  it("returns null for non-secret-scan errors", () => {
    expect(parseSecretScanError("Network error")).toBeNull();
    expect(parseSecretScanError("Push failed")).toBeNull();
    expect(parseSecretScanError("")).toBeNull();
  });

  it("parses a single finding", () => {
    const msg = "Secret scan detected potential secrets: .env: env file";
    const result = parseSecretScanError(msg);
    expect(result).toEqual([{ filePath: ".env", patternName: "env file" }]);
  });

  it("parses multiple findings", () => {
    const msg =
      "Secret scan detected potential secrets: .env: env file; config.pem: private key file; src/main.ts: AWS access key";
    const result = parseSecretScanError(msg);
    expect(result).toEqual([
      { filePath: ".env", patternName: "env file" },
      { filePath: "config.pem", patternName: "private key file" },
      { filePath: "src/main.ts", patternName: "AWS access key" },
    ]);
  });

  it("handles findings without file path (e.g., timeout)", () => {
    const msg =
      "Secret scan detected potential secrets: scan timeout exceeded (30s)";
    const result = parseSecretScanError(msg);
    expect(result).toEqual([
      { filePath: "", patternName: "scan timeout exceeded (30s)" },
    ]);
  });

  it("returns empty array when prefix matches but no findings", () => {
    const msg = "Secret scan detected potential secrets: ";
    const result = parseSecretScanError(msg);
    expect(result).toEqual([]);
  });
});
