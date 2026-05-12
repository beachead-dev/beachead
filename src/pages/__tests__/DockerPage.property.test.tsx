import { describe, it, expect } from "vitest";
import * as fc from "fast-check";
import { deriveSandboxButtonStates } from "../../lib/sandboxButtonStates";

/**
 * Feature: docker-management-tab, Property 2: Sandbox button state derivation from status
 *
 * Validates: Requirements 4.2, 4.3, 4.10
 *
 * For any status string, the derived button enabled/disabled states follow:
 * - "running" → Stop enabled, Start/Remove disabled
 * - "stopped" → Start/Remove enabled, Stop disabled
 * - anything else (including null, empty string, unknown) → all disabled
 */
describe("Feature: docker-management-tab, Property 2: Sandbox button state derivation from status", () => {
  it('status "running" enables only Stop button', () => {
    const result = deriveSandboxButtonStates("running");
    expect(result).toEqual({
      startEnabled: false,
      stopEnabled: true,
      removeEnabled: false,
    });
  });

  it('status "stopped" enables Start and Remove buttons', () => {
    const result = deriveSandboxButtonStates("stopped");
    expect(result).toEqual({
      startEnabled: true,
      stopEnabled: false,
      removeEnabled: true,
    });
  });

  it("null status disables all buttons", () => {
    const result = deriveSandboxButtonStates(null);
    expect(result).toEqual({
      startEnabled: false,
      stopEnabled: false,
      removeEnabled: false,
    });
  });

  it("for any arbitrary status string (not running/stopped), all buttons are disabled", () => {
    fc.assert(
      fc.property(
        fc.string().filter((s) => s !== "running" && s !== "stopped"),
        (status) => {
          const result = deriveSandboxButtonStates(status);
          expect(result).toEqual({
            startEnabled: false,
            stopEnabled: false,
            removeEnabled: false,
          });
        },
      ),
      { numRuns: 100 },
    );
  });

  it("for any status (including running, stopped, and arbitrary strings), button states follow the derivation rules", () => {
    const statusArb = fc.oneof(
      fc.constant("running"),
      fc.constant("stopped"),
      fc.constant(null as string | null),
      fc.constant(""),
      fc.string(),
    );

    fc.assert(
      fc.property(statusArb, (status) => {
        const result = deriveSandboxButtonStates(status);

        if (status === "running") {
          expect(result).toEqual({
            startEnabled: false,
            stopEnabled: true,
            removeEnabled: false,
          });
        } else if (status === "stopped") {
          expect(result).toEqual({
            startEnabled: true,
            stopEnabled: false,
            removeEnabled: true,
          });
        } else {
          expect(result).toEqual({
            startEnabled: false,
            stopEnabled: false,
            removeEnabled: false,
          });
        }
      }),
      { numRuns: 100 },
    );
  });
});
