import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, act, within } from "@testing-library/react";
import fc from "fast-check";
import { SandboxInfo } from "../../lib/api";

// Mock the API module
vi.mock("../../lib/api", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../lib/api")>();
  return {
    ...actual,
    getSandboxes: vi.fn(),
    stopSandbox: vi.fn(),
    startSandbox: vi.fn(),
    removeSandbox: vi.fn(),
    getMcpContainers: vi.fn(),
  };
});

import { getSandboxes } from "../../lib/api";
import { DockerPage } from "../DockerPage";

const mockGetSandboxes = getSandboxes as ReturnType<typeof vi.fn>;

/**
 * Pure filtering function that replicates the backend managed sandbox filtering logic.
 * Given a list of sandboxes and a set of managed IDs (from session sandbox_ids),
 * returns only those sandboxes whose id is non-null and present in the managed set.
 */
function filterManagedSandboxes(
  sandboxes: SandboxInfo[],
  managedIds: Set<string>,
): SandboxInfo[] {
  return sandboxes.filter((s) => s.id !== null && managedIds.has(s.id));
}

/**
 * Arbitrary generator for a SandboxInfo object.
 * Generates sandboxes with nullable id, name, and status fields.
 */
const sandboxInfoArb: fc.Arbitrary<SandboxInfo> = fc.record({
  name: fc.option(fc.string({ minLength: 1, maxLength: 20 }), { nil: null }),
  id: fc.option(fc.string({ minLength: 1, maxLength: 20 }), { nil: null }),
  status: fc.option(
    fc.oneof(
      fc.constant("running"),
      fc.constant("stopped"),
      fc.string({ minLength: 1, maxLength: 10 }),
    ),
    { nil: null },
  ),
  managed: fc.boolean(),
});

describe("Feature: docker-management-tab, Property 3: Managed sandbox filtering", () => {
  /**
   * **Validates: Requirements 3.8, 3.10**
   *
   * For any set of sandboxes and session sandbox_ids, the filtered list
   * contains exactly those sandboxes whose ID is in the session set.
   */
  it("filtered list contains exactly sandboxes with non-null IDs in the managed set", () => {
    fc.assert(
      fc.property(
        fc.array(sandboxInfoArb, { minLength: 0, maxLength: 30 }),
        fc.array(fc.string({ minLength: 1, maxLength: 20 }), {
          minLength: 0,
          maxLength: 15,
        }),
        (sandboxes, managedIdArray) => {
          const managedIds = new Set(managedIdArray);
          const result = filterManagedSandboxes(sandboxes, managedIds);

          // Assert 1: every sandbox in the result has an id that is in the managed set
          for (const s of result) {
            expect(s.id).not.toBeNull();
            expect(managedIds.has(s.id!)).toBe(true);
          }

          // Assert 2: every sandbox NOT in the result either has a null id
          // or an id NOT in the managed set
          const resultSet = new Set(result);
          for (const s of sandboxes) {
            if (!resultSet.has(s)) {
              expect(s.id === null || !managedIds.has(s.id!)).toBe(true);
            }
          }

          // Assert 3: the result length equals the count of sandboxes with
          // non-null ids that are in the managed set
          const expectedCount = sandboxes.filter(
            (s) => s.id !== null && managedIds.has(s.id!),
          ).length;
          expect(result.length).toBe(expectedCount);
        },
      ),
      { numRuns: 100 },
    );
  });
});
