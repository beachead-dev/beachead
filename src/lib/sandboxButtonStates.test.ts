import { describe, it, expect } from "vitest";
import { deriveSandboxButtonStates } from "./sandboxButtonStates";

describe("deriveSandboxButtonStates", () => {
  it('returns Stop enabled, Start/Remove disabled for "running"', () => {
    const result = deriveSandboxButtonStates("running");
    expect(result).toEqual({
      startEnabled: false,
      stopEnabled: true,
      removeEnabled: false,
    });
  });

  it('returns Start/Remove enabled, Stop disabled for "stopped"', () => {
    const result = deriveSandboxButtonStates("stopped");
    expect(result).toEqual({
      startEnabled: true,
      stopEnabled: false,
      removeEnabled: true,
    });
  });

  it("returns all disabled for null", () => {
    const result = deriveSandboxButtonStates(null);
    expect(result).toEqual({
      startEnabled: false,
      stopEnabled: false,
      removeEnabled: false,
    });
  });

  it("returns all disabled for empty string", () => {
    const result = deriveSandboxButtonStates("");
    expect(result).toEqual({
      startEnabled: false,
      stopEnabled: false,
      removeEnabled: false,
    });
  });

  it("returns all disabled for unknown status", () => {
    const result = deriveSandboxButtonStates("restarting");
    expect(result).toEqual({
      startEnabled: false,
      stopEnabled: false,
      removeEnabled: false,
    });
  });
});
