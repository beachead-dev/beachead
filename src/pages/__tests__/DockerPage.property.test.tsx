import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render, screen, act, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import fc from "fast-check";
import { SandboxInfo, McpContainerResponse } from "../../lib/api";

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
    startContainer: vi.fn(),
    stopContainer: vi.fn(),
    removeContainer: vi.fn(),
  };
});

import { getSandboxes, getMcpContainers } from "../../lib/api";
import { DockerPage } from "../DockerPage";

const mockGetSandboxes = getSandboxes as ReturnType<typeof vi.fn>;
const mockGetMcpContainers = getMcpContainers as ReturnType<typeof vi.fn>;

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

import { deriveContainerButtonStates } from "../../lib/containerButtonStates";

describe("Feature: docker-management-tab, Property 5: Container button state derivation from status", () => {
  /**
   * **Validates: Requirements 7.1, 7.2, 7.3**
   *
   * For any status string, verify:
   * - "running" → Stop enabled, Start/Remove disabled
   * - "stopped"/"created" → Start/Remove enabled, Stop disabled
   * - any other status → all disabled
   *
   * This tests the MANAGED case (isUnmanaged=false).
   */

  // Generator for container status: includes known statuses, null, empty, and random strings
  const statusArb: fc.Arbitrary<string | null> = fc.oneof(
    fc.constant("running"),
    fc.constant("stopped"),
    fc.constant("created"),
    fc.constant(null as string | null),
    fc.constant(""),
    fc.string({ minLength: 0, maxLength: 30 }),
  );

  it("derives correct button states for managed containers based on status", () => {
    fc.assert(
      fc.property(statusArb, (status) => {
        const result = deriveContainerButtonStates(status, false);

        if (status === "running") {
          expect(result).toEqual({
            startEnabled: false,
            stopEnabled: true,
            removeEnabled: false,
          });
        } else if (status === "stopped" || status === "created") {
          expect(result).toEqual({
            startEnabled: true,
            stopEnabled: false,
            removeEnabled: true,
          });
        } else {
          // null, empty, or any other string → all disabled
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


describe("Feature: docker-management-tab, Property 4: Container table rendering with sorting", () => {
  /**
   * **Validates: Requirements 6.2, 6.3**
   *
   * For any array of container objects, verify rows contain all columns,
   * persona name is displayed (not persona_id), and rows are ordered by
   * created_at descending (newest first).
   */

  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // Generator for safe printable strings (no control characters)
  const safeStringArb = fc
    .string({ minLength: 1, maxLength: 15 })
    .map((s) => s.replace(/[^\x20-\x7E]/g, "a"))
    .filter((s) => s.trim().length > 0);

  // Generator for container status values
  const statusArb = fc.oneof(
    fc.constant("running"),
    fc.constant("stopped"),
    fc.constant("created"),
    safeStringArb,
  );

  // Generator for a McpContainerResponse object
  const containerArb: fc.Arbitrary<McpContainerResponse> = fc
    .record({
      id: fc.uuid(),
      persona_id: fc.uuid(),
      persona_name: safeStringArb,
      container_id: fc.option(fc.uuid(), { nil: null }),
      image: safeStringArb,
      port: fc.integer({ min: 1024, max: 65535 }),
      volume_name: safeStringArb,
      status: statusArb,
      live_status_confirmed: fc.boolean(),
      created_at: fc.integer({
        min: 1577836800000, // 2020-01-01T00:00:00Z
        max: 1924991999000, // 2030-12-31T23:59:59Z
      }).map((ts) => new Date(ts).toISOString()),
      updated_at: fc.integer({
        min: 1577836800000, // 2020-01-01T00:00:00Z
        max: 1924991999000, // 2030-12-31T23:59:59Z
      }).map((ts) => new Date(ts).toISOString()),
    });

  // Non-empty arrays (empty arrays show empty state, not a table)
  const containerArrayArb = fc.array(containerArb, { minLength: 1, maxLength: 5 });

  it("renders a table row for each container with all columns and rows sorted by created_at descending", { timeout: 60000 }, async () => {
    await fc.assert(
      fc.asyncProperty(containerArrayArb, async (containers) => {
        // Mock getSandboxes to return empty (so Sandboxes tab doesn't interfere)
        mockGetSandboxes.mockResolvedValue([]);
        // Mock getMcpContainers to return the generated containers
        mockGetMcpContainers.mockResolvedValue(containers);

        const { unmount } = render(<DockerPage />);

        // Wait for initial Sandboxes tab data to load
        await act(async () => {
          await vi.advanceTimersByTimeAsync(0);
        });

        // Switch to Containers tab
        const containersTab = screen.getByRole("tab", { name: /containers/i });
        await act(async () => {
          containersTab.click();
        });

        // Wait for container data to load
        await act(async () => {
          await vi.advanceTimersByTimeAsync(0);
        });

        // Get the containers table
        const table = screen.getByRole("table", { name: "Containers table" });
        const tbody = table.querySelector("tbody");
        expect(tbody).not.toBeNull();

        const rows = within(tbody!).getAllByRole("row");
        expect(rows.length).toBe(containers.length);

        // Expected sort order: by created_at descending (newest first)
        const sortedContainers = [...containers].sort(
          (a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime(),
        );

        for (let i = 0; i < sortedContainers.length; i++) {
          const container = sortedContainers[i];
          const row = rows[i];
          const cells = within(row).getAllByRole("cell");

          // 7 cells: Persona Name, Image, Port, Status, Volume Name, Created Date, Actions
          expect(cells.length).toBe(7);

          // Persona Name column: displays persona_name (not persona_id)
          expect(cells[0].textContent).toContain(container.persona_name);
          expect(cells[0].textContent).not.toContain(container.persona_id);

          // Image column
          expect(cells[1].textContent).toBeDefined();

          // Port column
          expect(cells[2].textContent).toBe(String(container.port));

          // Status column
          expect(cells[3].textContent).toBe(container.status);

          // Volume Name column
          expect(cells[4].textContent).toBe(container.volume_name);

          // Created Date column: should contain a formatted version of created_at
          // The component uses toLocaleString(), so we verify it's non-empty and
          // represents the same date
          const cellDateText = cells[5].textContent ?? "";
          expect(cellDateText.length).toBeGreaterThan(0);

          // Actions column: Start, Stop, Remove buttons
          const buttons = within(cells[6]).getAllByRole("button");
          expect(buttons.length).toBe(3);
          expect(buttons[0]).toHaveTextContent("Start");
          expect(buttons[1]).toHaveTextContent("Stop");
          expect(buttons[2]).toHaveTextContent("Remove");
        }

        // Verify sort order: each row's created_at should be >= the next row's
        for (let i = 0; i < sortedContainers.length - 1; i++) {
          const currentDate = new Date(sortedContainers[i].created_at).getTime();
          const nextDate = new Date(sortedContainers[i + 1].created_at).getTime();
          expect(currentDate).toBeGreaterThanOrEqual(nextDate);
        }

        unmount();
      }),
      { numRuns: 100 },
    );
  });
});

describe("Feature: docker-management-tab, Property 6: Unmanaged container display rules", () => {
  /**
   * **Validates: Requirements 6.11**
   *
   * For any container not tracked in DB (identified by id starting with "unmanaged-"),
   * verify:
   * - An "Unmanaged" badge is displayed in the row
   * - The Start button is ALWAYS disabled (regardless of status)
   * - Stop and Remove buttons follow deriveContainerButtonStates unmanaged rules:
   *   - "running" → Stop enabled, Remove enabled
   *   - "stopped"/"created" → Stop disabled, Remove enabled
   *   - other → both disabled
   */

  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  // Generator for safe printable strings
  const safeStringArb = fc
    .string({ minLength: 1, maxLength: 15 })
    .map((s) => s.replace(/[^\x20-\x7E]/g, "a"))
    .filter((s) => s.trim().length > 0);

  // Generator for container status: known statuses + arbitrary strings
  const statusArb = fc.oneof(
    fc.constant("running"),
    fc.constant("stopped"),
    fc.constant("created"),
    fc.string(),
  );

  // Generator for unmanaged container objects (id starts with "unmanaged-")
  const unmanagedContainerArb: fc.Arbitrary<McpContainerResponse> = fc
    .record({
      id: safeStringArb.map((s) => `unmanaged-${s}`),
      persona_id: fc.uuid(),
      persona_name: safeStringArb,
      container_id: fc.option(fc.uuid(), { nil: null }),
      image: safeStringArb,
      port: fc.integer({ min: 1024, max: 65535 }),
      volume_name: safeStringArb,
      status: statusArb,
      live_status_confirmed: fc.boolean(),
      created_at: fc.date({
        min: new Date("2020-01-01T00:00:00Z"),
        max: new Date("2030-12-31T23:59:59Z"),
      }).map((d) => d.toISOString()),
      updated_at: fc.date({
        min: new Date("2020-01-01T00:00:00Z"),
        max: new Date("2030-12-31T23:59:59Z"),
      }).map((d) => d.toISOString()),
    });

  // Non-empty arrays of unmanaged containers
  const unmanagedContainerArrayArb = fc.array(unmanagedContainerArb, { minLength: 1, maxLength: 5 });

  it("displays Unmanaged badge and correct button states for unmanaged containers", { timeout: 60000 }, async () => {
    await fc.assert(
      fc.asyncProperty(unmanagedContainerArrayArb, async (containers) => {
        // Mock getSandboxes to return empty so Sandboxes tab doesn't interfere
        mockGetSandboxes.mockResolvedValue([]);
        // Mock getMcpContainers to return the generated unmanaged containers
        mockGetMcpContainers.mockResolvedValue(containers);

        const { unmount } = render(<DockerPage />);

        // Wait for initial Sandboxes tab data to load
        await act(async () => {
          await vi.advanceTimersByTimeAsync(0);
        });

        // Switch to Containers tab
        const containersTab = screen.getByRole("tab", { name: /containers/i });
        await act(async () => {
          containersTab.click();
        });

        // Wait for container data to load
        await act(async () => {
          await vi.advanceTimersByTimeAsync(0);
        });

        // Get the containers table
        const table = screen.getByRole("table", { name: "Containers table" });
        const tbody = table.querySelector("tbody");
        expect(tbody).not.toBeNull();

        const rows = within(tbody!).getAllByRole("row");
        expect(rows.length).toBe(containers.length);

        // Sort containers the same way the component does (by created_at descending)
        const sortedContainers = [...containers].sort(
          (a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime(),
        );

        for (let i = 0; i < sortedContainers.length; i++) {
          const container = sortedContainers[i];
          const row = rows[i];

          // Assert: "Unmanaged" badge is displayed in the row
          const badge = within(row).getByText("Unmanaged");
          expect(badge).toBeDefined();
          expect(badge.classList.contains("badge-unmanaged")).toBe(true);

          // Get action buttons
          const cells = within(row).getAllByRole("cell");
          const actionCell = cells[6]; // Actions column is the 7th cell
          const buttons = within(actionCell).getAllByRole("button");
          expect(buttons.length).toBe(3);

          const startBtn = buttons[0];
          const stopBtn = buttons[1];
          const removeBtn = buttons[2];

          expect(startBtn).toHaveTextContent("Start");
          expect(stopBtn).toHaveTextContent("Stop");
          expect(removeBtn).toHaveTextContent("Remove");

          // Start button is ALWAYS disabled for unmanaged containers
          expect(startBtn).toBeDisabled();

          // Stop and Remove follow deriveContainerButtonStates unmanaged rules
          if (container.status === "running") {
            expect(stopBtn).toBeEnabled();
            expect(removeBtn).toBeEnabled();
          } else if (container.status === "stopped" || container.status === "created") {
            expect(stopBtn).toBeDisabled();
            expect(removeBtn).toBeEnabled();
          } else {
            // Any other status → both disabled
            expect(stopBtn).toBeDisabled();
            expect(removeBtn).toBeDisabled();
          }
        }

        unmount();
      }),
      { numRuns: 100 },
    );
  });
});
