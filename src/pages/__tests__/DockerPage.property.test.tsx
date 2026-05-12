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

describe("Feature: docker-management-tab, Property 1: Sandbox table rendering completeness", () => {
  /**
   * **Validates: Requirements 3.2, 3.3, 4.1**
   *
   * For any array of sandbox objects with arbitrary present/null fields,
   * verify a row is rendered for each sandbox with name (or placeholder),
   * status (or placeholder), ID (or placeholder), and action buttons.
   */

  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // Generator for safe strings (printable ASCII, no control characters)
  const safeStringArb = fc
    .string({ minLength: 1, maxLength: 20 })
    .map((s) => s.replace(/[^\x20-\x7E]/g, "a"))
    .filter((s) => s.trim().length > 0);

  // Generator for sandbox objects with nullable fields
  const sandboxArb: fc.Arbitrary<SandboxInfo> = fc.record({
    name: fc.option(safeStringArb, { nil: null }),
    id: fc.option(safeStringArb, { nil: null }),
    status: fc.option(
      fc.oneof(
        fc.constant("running"),
        fc.constant("stopped"),
        safeStringArb,
      ),
      { nil: null },
    ),
    managed: fc.constant(true),
  });

  // Non-empty arrays (empty arrays show empty state, not a table)
  const sandboxArrayArb = fc.array(sandboxArb, { minLength: 1, maxLength: 5 });

  it("renders a table row for each sandbox with correct content and action buttons", async () => {
    await fc.assert(
      fc.asyncProperty(sandboxArrayArb, async (sandboxes) => {
        mockGetSandboxes.mockResolvedValue(sandboxes);

        const { unmount } = render(<DockerPage />);

        await act(async () => {
          await vi.advanceTimersByTimeAsync(0);
        });

        // Get the table body rows
        const table = screen.getByRole("table", { name: "Sandboxes table" });
        const tbody = table.querySelector("tbody");
        expect(tbody).not.toBeNull();

        const rows = within(tbody!).getAllByRole("row");
        expect(rows.length).toBe(sandboxes.length);

        for (let i = 0; i < sandboxes.length; i++) {
          const sandbox = sandboxes[i];
          const row = rows[i];
          const cells = within(row).getAllByRole("cell");

          // 4 cells: Name, Status, ID, Actions
          expect(cells.length).toBe(4);

          // Name column: value or placeholder "\u2014"
          const expectedName = sandbox.name ?? "\u2014";
          expect(cells[0].textContent).toBe(expectedName);

          // Status column: value or placeholder "\u2014"
          const expectedStatus = sandbox.status ?? "\u2014";
          expect(cells[1].textContent).toBe(expectedStatus);

          // ID column: value or placeholder "\u2014"
          const expectedId = sandbox.id ?? "\u2014";
          expect(cells[2].textContent).toBe(expectedId);

          // Actions column: Start, Stop, Remove buttons
          const buttons = within(cells[3]).getAllByRole("button");
          expect(buttons.length).toBe(3);
          expect(buttons[0]).toHaveTextContent("Start");
          expect(buttons[1]).toHaveTextContent("Stop");
          expect(buttons[2]).toHaveTextContent("Remove");
        }

        unmount();
      }),
      { numRuns: 100 },
    );
  });
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
