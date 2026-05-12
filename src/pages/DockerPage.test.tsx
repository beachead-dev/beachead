import { render, screen, act, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { DockerPage } from "./DockerPage";

// Mock the API module
vi.mock("../lib/api", () => ({
  getSandboxes: vi.fn(),
  stopSandbox: vi.fn(),
  startSandbox: vi.fn(),
  removeSandbox: vi.fn(),
  getMcpContainers: vi.fn(),
  startContainer: vi.fn(),
  stopContainer: vi.fn(),
  removeContainer: vi.fn(),
}));

import { getSandboxes, stopSandbox, startSandbox, removeSandbox, getMcpContainers, startContainer, stopContainer, removeContainer } from "../lib/api";

const mockGetSandboxes = getSandboxes as ReturnType<typeof vi.fn>;
const mockStopSandbox = stopSandbox as ReturnType<typeof vi.fn>;
const mockStartSandbox = startSandbox as ReturnType<typeof vi.fn>;
const mockRemoveSandbox = removeSandbox as ReturnType<typeof vi.fn>;
const mockGetMcpContainers = getMcpContainers as ReturnType<typeof vi.fn>;
const mockStartContainer = startContainer as ReturnType<typeof vi.fn>;
const mockStopContainer = stopContainer as ReturnType<typeof vi.fn>;
const mockRemoveContainer = removeContainer as ReturnType<typeof vi.fn>;

describe("DockerPage — Sandboxes Tab", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows loading indicator while fetching", () => {
    mockGetSandboxes.mockReturnValue(new Promise(() => {}));
    render(<DockerPage />);
    expect(screen.getByText(/Loading sandboxes/)).toBeInTheDocument();
  });

  it("displays sandbox table with Name, Status, ID columns", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: "my-sandbox", id: "abc123", status: "running", managed: true },
      { name: "other-sandbox", id: "def456", status: "stopped", managed: true },
    ]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByText("my-sandbox")).toBeInTheDocument();
    expect(screen.getByText("Name")).toBeInTheDocument();
    expect(screen.getByText("Status")).toBeInTheDocument();
    expect(screen.getByText("ID")).toBeInTheDocument();
    expect(screen.getByText("abc123")).toBeInTheDocument();
    expect(screen.getByText("running")).toBeInTheDocument();
    expect(screen.getByText("other-sandbox")).toBeInTheDocument();
    expect(screen.getByText("def456")).toBeInTheDocument();
    expect(screen.getByText("stopped")).toBeInTheDocument();
  });

  it("shows placeholder for null/missing Name or ID values", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: null, id: null, status: "running", managed: true },
      { name: "has-name", id: null, status: null, managed: true },
    ]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByText("has-name")).toBeInTheDocument();
    const dashes = screen.getAllByText("\u2014");
    // null name (row 1) + null id (row 1) + null id (row 2) + null status (row 2) = 4
    expect(dashes.length).toBe(4);
  });

  it("shows empty state message when no sandboxes exist", async () => {
    mockGetSandboxes.mockResolvedValue([]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByText("No sandboxes found")).toBeInTheDocument();
  });

  it("shows error message on fetch failure", async () => {
    mockGetSandboxes.mockRejectedValue(new Error("Network error"));

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByText("Network error")).toBeInTheDocument();
  });

  it("calls getSandboxes with showAll=false by default", async () => {
    mockGetSandboxes.mockResolvedValue([]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockGetSandboxes).toHaveBeenCalledWith(false);
  });

  it("calls getSandboxes with showAll=true when toggle is checked", async () => {
    mockGetSandboxes.mockResolvedValue([]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByText("No sandboxes found")).toBeInTheDocument();
    mockGetSandboxes.mockClear();

    // Click the Show All toggle
    fireEvent.click(screen.getByRole("checkbox"));

    // The fetchFn ref updates, but polling continues on its existing interval.
    // Advance to the next poll tick to see the call with showAll=true.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(mockGetSandboxes).toHaveBeenCalledWith(true);
  });

  it("uses polling with 10-second interval", async () => {
    mockGetSandboxes.mockResolvedValue([]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(2);
  });

  it("shows action buttons for each sandbox row", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: "my-sandbox", id: "abc123", status: "running", managed: true },
    ]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByLabelText("Start sandbox my-sandbox")).toBeInTheDocument();
    expect(screen.getByLabelText("Stop sandbox my-sandbox")).toBeInTheDocument();
    expect(screen.getByLabelText("Remove sandbox my-sandbox")).toBeInTheDocument();
  });

  it("enables Stop and disables Start/Remove for running sandbox", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: "running-sbx", id: "r1", status: "running", managed: true },
    ]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByLabelText("Start sandbox running-sbx")).toBeDisabled();
    expect(screen.getByLabelText("Stop sandbox running-sbx")).toBeEnabled();
    expect(screen.getByLabelText("Remove sandbox running-sbx")).toBeDisabled();
  });

  it("enables Start/Remove and disables Stop for stopped sandbox", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: "stopped-sbx", id: "s1", status: "stopped", managed: true },
    ]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByLabelText("Start sandbox stopped-sbx")).toBeEnabled();
    expect(screen.getByLabelText("Stop sandbox stopped-sbx")).toBeDisabled();
    expect(screen.getByLabelText("Remove sandbox stopped-sbx")).toBeEnabled();
  });

  it("disables all buttons for unknown status", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: "unknown-sbx", id: "u1", status: "restarting", managed: true },
    ]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByLabelText("Start sandbox unknown-sbx")).toBeDisabled();
    expect(screen.getByLabelText("Stop sandbox unknown-sbx")).toBeDisabled();
    expect(screen.getByLabelText("Remove sandbox unknown-sbx")).toBeDisabled();
  });

  it("disables all buttons during pending action and re-enables on success", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: "my-sbx", id: "abc", status: "running", managed: true },
    ]);
    let resolveStop: (value: unknown) => void;
    mockStopSandbox.mockReturnValue(
      new Promise((resolve) => { resolveStop = resolve; }),
    );

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Click stop
    fireEvent.click(screen.getByLabelText("Stop sandbox my-sbx"));

    // All buttons should be disabled during pending action
    expect(screen.getByLabelText("Start sandbox my-sbx")).toBeDisabled();
    expect(screen.getByLabelText("Stop sandbox my-sbx")).toBeDisabled();
    expect(screen.getByLabelText("Remove sandbox my-sbx")).toBeDisabled();

    // Resolve the action
    await act(async () => {
      resolveStop!({ id: "abc", status: "stopped" });
      await vi.advanceTimersByTimeAsync(0);
    });

    // refresh() should have been called
    expect(mockGetSandboxes).toHaveBeenCalledTimes(2);
  });

  it("shows error message on action failure and re-enables buttons", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: "fail-sbx", id: "f1", status: "stopped", managed: true },
    ]);
    mockStartSandbox.mockRejectedValue(new Error("Server unavailable"));

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Click start
    await act(async () => {
      fireEvent.click(screen.getByLabelText("Start sandbox fail-sbx"));
      await vi.advanceTimersByTimeAsync(0);
    });

    // Error message should be displayed
    expect(screen.getByText(/Failed to start sandbox: Server unavailable/)).toBeInTheDocument();

    // Buttons should be re-enabled (based on status)
    expect(screen.getByLabelText("Start sandbox fail-sbx")).toBeEnabled();
  });

  it("calls removeSandbox when Remove is confirmed via dialog", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: "rm-sbx", id: "rm1", status: "stopped", managed: true },
    ]);
    mockRemoveSandbox.mockResolvedValue(undefined);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Click Remove to open the confirmation dialog
    fireEvent.click(screen.getByLabelText("Remove sandbox rm-sbx"));

    // Dialog should be visible
    expect(screen.getByText(/This will permanently remove the sandbox 'rm-sbx'/)).toBeInTheDocument();

    // Confirm removal
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Remove" }));
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockRemoveSandbox).toHaveBeenCalledWith("rm1");
  });

  it("does not call removeSandbox when dialog is cancelled", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: "cancel-sbx", id: "c1", status: "stopped", managed: true },
    ]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Click Remove to open the confirmation dialog
    fireEvent.click(screen.getByLabelText("Remove sandbox cancel-sbx"));

    // Dialog should be visible
    expect(screen.getByText(/This will permanently remove the sandbox 'cancel-sbx'/)).toBeInTheDocument();

    // Cancel
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    // Dialog should be closed and removeSandbox not called
    expect(screen.queryByText(/This will permanently remove the sandbox/)).not.toBeInTheDocument();
    expect(mockRemoveSandbox).not.toHaveBeenCalled();
  });
});

describe("DockerPage — Polling Lifecycle", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("stops Sandboxes polling and starts Containers polling when switching tabs", async () => {
    mockGetSandboxes.mockResolvedValue([]);
    mockGetMcpContainers.mockResolvedValue([]);

    render(<DockerPage />);

    // Initial fetch for Sandboxes tab
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(1);
    expect(mockGetMcpContainers).not.toHaveBeenCalled();

    // Advance one poll interval — Sandboxes should poll again
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(2);
    expect(mockGetMcpContainers).not.toHaveBeenCalled();

    // Switch to Containers tab
    fireEvent.click(screen.getByRole("tab", { name: "Containers" }));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Containers should have fetched immediately
    expect(mockGetMcpContainers).toHaveBeenCalledTimes(1);

    // Reset sandbox call count to verify no more calls
    const sandboxCallsAtSwitch = mockGetSandboxes.mock.calls.length;

    // Advance one poll interval — only Containers should poll
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(mockGetMcpContainers).toHaveBeenCalledTimes(2);
    expect(mockGetSandboxes).toHaveBeenCalledTimes(sandboxCallsAtSwitch);
  });

  it("stops Containers polling and resumes Sandboxes polling when switching back", async () => {
    mockGetSandboxes.mockResolvedValue([]);
    mockGetMcpContainers.mockResolvedValue([]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Switch to Containers
    fireEvent.click(screen.getByRole("tab", { name: "Containers" }));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    const containerCallsBeforeSwitch = mockGetMcpContainers.mock.calls.length;

    // Switch back to Sandboxes
    fireEvent.click(screen.getByRole("tab", { name: "Sandboxes" }));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    const sandboxCallsAfterSwitch = mockGetSandboxes.mock.calls.length;

    // Advance one poll interval — only Sandboxes should poll
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(sandboxCallsAfterSwitch + 1);
    expect(mockGetMcpContainers).toHaveBeenCalledTimes(containerCallsBeforeSwitch);
  });

  it("stops all polling when component unmounts (navigating away)", async () => {
    mockGetSandboxes.mockResolvedValue([]);

    const { unmount } = render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(1);

    // Unmount simulates navigating away from /docker
    unmount();

    // Advance time — no more polling should occur
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30000);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(1);
  });

  it("resets poll timer after successful mutation + refresh", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: "sbx", id: "s1", status: "running", managed: true },
    ]);
    mockStopSandbox.mockResolvedValue({ id: "s1", status: "stopped" });

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(1);

    // Advance 7 seconds (not yet at 10s interval)
    await act(async () => {
      await vi.advanceTimersByTimeAsync(7000);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(1);

    // Perform a mutation (stop) — this calls refresh() which resets the timer
    await act(async () => {
      fireEvent.click(screen.getByLabelText("Stop sandbox sbx"));
      await vi.advanceTimersByTimeAsync(0);
    });

    // refresh() triggered an immediate re-fetch
    expect(mockGetSandboxes).toHaveBeenCalledTimes(2);

    // Advance 7 seconds from the refresh point — should NOT trigger another poll
    await act(async () => {
      await vi.advanceTimersByTimeAsync(7000);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(2);

    // Advance 3 more seconds (total 10s from refresh) — should trigger next poll
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(3);
  });
});

describe("DockerPage — Stale Data Indication", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows stale indicator when poll fails while retaining last data", async () => {
    mockGetSandboxes
      .mockResolvedValueOnce([
        { name: "my-sandbox", id: "abc123", status: "running", managed: true },
      ])
      .mockRejectedValueOnce(new Error("Network error"));

    render(<DockerPage />);

    // Initial fetch succeeds
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByText("my-sandbox")).toBeInTheDocument();
    expect(screen.queryByText("Data may be stale. Retrying…")).not.toBeInTheDocument();

    // Second poll fails
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    // Stale indicator should appear
    expect(screen.getByText("Data may be stale. Retrying…")).toBeInTheDocument();
    // Last data should still be displayed
    expect(screen.getByText("my-sandbox")).toBeInTheDocument();
    expect(screen.getByText("abc123")).toBeInTheDocument();
  });

  it("removes stale indicator on next successful poll", async () => {
    mockGetSandboxes
      .mockResolvedValueOnce([
        { name: "my-sandbox", id: "abc123", status: "running", managed: true },
      ])
      .mockRejectedValueOnce(new Error("Network error"))
      .mockResolvedValueOnce([
        { name: "my-sandbox", id: "abc123", status: "stopped", managed: true },
      ]);

    render(<DockerPage />);

    // Initial fetch succeeds
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.queryByText("Data may be stale. Retrying…")).not.toBeInTheDocument();

    // Second poll fails — stale indicator appears
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(screen.getByText("Data may be stale. Retrying…")).toBeInTheDocument();

    // Third poll succeeds — stale indicator disappears
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(screen.queryByText("Data may be stale. Retrying…")).not.toBeInTheDocument();
    // Updated data is shown
    expect(screen.getByText("stopped")).toBeInTheDocument();
  });

  it("continues polling on regular interval after failure", async () => {
    mockGetSandboxes
      .mockResolvedValueOnce([
        { name: "my-sandbox", id: "abc123", status: "running", managed: true },
      ])
      .mockRejectedValueOnce(new Error("Network error"))
      .mockResolvedValueOnce([
        { name: "my-sandbox", id: "abc123", status: "running", managed: true },
      ]);

    render(<DockerPage />);

    // Initial fetch
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(1);

    // Second poll at 10s (fails)
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(2);

    // Third poll at 20s (succeeds) — polling was not interrupted by the failure
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(mockGetSandboxes).toHaveBeenCalledTimes(3);
  });
});

describe("DockerPage — Containers Tab — Removal Dialog", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    // Default: sandboxes tab loads empty so we can switch to containers
    mockGetSandboxes.mockResolvedValue([]);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows confirmation dialog with volume checkbox when Remove is clicked", async () => {
    mockGetMcpContainers.mockResolvedValue([
      {
        id: "c1",
        persona_id: "p1",
        persona_name: "Test Persona",
        container_id: "docker-123",
        image: "beachead-memory-mcp:latest",
        port: 9001,
        volume_name: "beachead-memory-p1",
        status: "stopped",
        live_status_confirmed: true,
        created_at: "2024-01-01T00:00:00Z",
        updated_at: "2024-01-01T00:00:00Z",
      },
    ]);

    render(<DockerPage />);

    // Switch to Containers tab
    fireEvent.click(screen.getByRole("tab", { name: "Containers" }));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Click Remove
    fireEvent.click(screen.getByLabelText("Remove container Test Persona"));

    // Dialog should be visible with container name
    expect(screen.getByText(/This will permanently remove the container 'Test Persona'/)).toBeInTheDocument();

    // Volume checkbox should be present and unchecked by default
    const checkbox = screen.getByLabelText("Also delete associated Docker volume");
    expect(checkbox).toBeInTheDocument();
    expect(checkbox).not.toBeChecked();
  });

  it("calls removeContainer with deleteVolume=false when confirmed without checkbox", async () => {
    mockGetMcpContainers.mockResolvedValue([
      {
        id: "c1",
        persona_id: "p1",
        persona_name: "Test Persona",
        container_id: "docker-123",
        image: "beachead-memory-mcp:latest",
        port: 9001,
        volume_name: "beachead-memory-p1",
        status: "stopped",
        live_status_confirmed: true,
        created_at: "2024-01-01T00:00:00Z",
        updated_at: "2024-01-01T00:00:00Z",
      },
    ]);
    mockRemoveContainer.mockResolvedValue(undefined);

    render(<DockerPage />);

    fireEvent.click(screen.getByRole("tab", { name: "Containers" }));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Click Remove to open dialog
    fireEvent.click(screen.getByLabelText("Remove container Test Persona"));

    // Confirm without checking the volume checkbox
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Remove" }));
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockRemoveContainer).toHaveBeenCalledWith("c1", false);
  });

  it("calls removeContainer with deleteVolume=true when checkbox is checked", async () => {
    mockGetMcpContainers.mockResolvedValue([
      {
        id: "c1",
        persona_id: "p1",
        persona_name: "Test Persona",
        container_id: "docker-123",
        image: "beachead-memory-mcp:latest",
        port: 9001,
        volume_name: "beachead-memory-p1",
        status: "stopped",
        live_status_confirmed: true,
        created_at: "2024-01-01T00:00:00Z",
        updated_at: "2024-01-01T00:00:00Z",
      },
    ]);
    mockRemoveContainer.mockResolvedValue(undefined);

    render(<DockerPage />);

    fireEvent.click(screen.getByRole("tab", { name: "Containers" }));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Click Remove to open dialog
    fireEvent.click(screen.getByLabelText("Remove container Test Persona"));

    // Check the volume deletion checkbox
    fireEvent.click(screen.getByLabelText("Also delete associated Docker volume"));

    // Confirm
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Remove" }));
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockRemoveContainer).toHaveBeenCalledWith("c1", true);
  });

  it("does not call removeContainer when dialog is cancelled", async () => {
    mockGetMcpContainers.mockResolvedValue([
      {
        id: "c1",
        persona_id: "p1",
        persona_name: "Test Persona",
        container_id: "docker-123",
        image: "beachead-memory-mcp:latest",
        port: 9001,
        volume_name: "beachead-memory-p1",
        status: "stopped",
        live_status_confirmed: true,
        created_at: "2024-01-01T00:00:00Z",
        updated_at: "2024-01-01T00:00:00Z",
      },
    ]);

    render(<DockerPage />);

    fireEvent.click(screen.getByRole("tab", { name: "Containers" }));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Click Remove to open dialog
    fireEvent.click(screen.getByLabelText("Remove container Test Persona"));

    // Cancel
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    // Dialog should be closed
    expect(screen.queryByText(/This will permanently remove the container/)).not.toBeInTheDocument();
    expect(mockRemoveContainer).not.toHaveBeenCalled();
  });

  it("resets deleteVolume checkbox state after cancel", async () => {
    mockGetMcpContainers.mockResolvedValue([
      {
        id: "c1",
        persona_id: "p1",
        persona_name: "Test Persona",
        container_id: "docker-123",
        image: "beachead-memory-mcp:latest",
        port: 9001,
        volume_name: "beachead-memory-p1",
        status: "stopped",
        live_status_confirmed: true,
        created_at: "2024-01-01T00:00:00Z",
        updated_at: "2024-01-01T00:00:00Z",
      },
    ]);
    mockRemoveContainer.mockResolvedValue(undefined);

    render(<DockerPage />);

    fireEvent.click(screen.getByRole("tab", { name: "Containers" }));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Open dialog and check the volume checkbox
    fireEvent.click(screen.getByLabelText("Remove container Test Persona"));
    fireEvent.click(screen.getByLabelText("Also delete associated Docker volume"));

    // Cancel
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    // Re-open dialog — checkbox should be unchecked
    fireEvent.click(screen.getByLabelText("Remove container Test Persona"));
    const checkbox = screen.getByLabelText("Also delete associated Docker volume");
    expect(checkbox).not.toBeChecked();
  });
});

describe("DockerPage — Tab Switching Content", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("displays Sandboxes tab content by default", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: "sandbox-1", id: "s1", status: "running", managed: true },
    ]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Sandboxes table should be visible
    expect(screen.getByRole("table", { name: "Sandboxes table" })).toBeInTheDocument();
    expect(screen.getByText("sandbox-1")).toBeInTheDocument();
    // Containers table should not be visible
    expect(screen.queryByRole("table", { name: "Containers table" })).not.toBeInTheDocument();
  });

  it("shows Containers content when Containers tab is clicked", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: "sandbox-1", id: "s1", status: "running", managed: true },
    ]);
    mockGetMcpContainers.mockResolvedValue([
      {
        id: "c1",
        persona_id: "p1",
        persona_name: "Memory Bot",
        container_id: "docker-abc",
        port: 9001,
        volume_name: "beachead-memory-p1",
        status: "running",
        live_status_confirmed: true,
        created_at: "2024-06-01T12:00:00Z",
        updated_at: "2024-06-01T12:00:00Z",
      },
    ]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Switch to Containers tab
    fireEvent.click(screen.getByRole("tab", { name: "Containers" }));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Containers table should be visible with correct columns
    expect(screen.getByRole("table", { name: "Containers table" })).toBeInTheDocument();
    expect(screen.getByText("Memory Bot")).toBeInTheDocument();
    expect(screen.getByText("9001")).toBeInTheDocument();
    // Sandboxes table should not be visible
    expect(screen.queryByRole("table", { name: "Sandboxes table" })).not.toBeInTheDocument();
  });

  it("shows Sandboxes content again when switching back from Containers", async () => {
    mockGetSandboxes.mockResolvedValue([
      { name: "sandbox-1", id: "s1", status: "stopped", managed: true },
    ]);
    mockGetMcpContainers.mockResolvedValue([
      {
        id: "c1",
        persona_id: "p1",
        persona_name: "Memory Bot",
        container_id: "docker-abc",
        port: 9001,
        volume_name: "beachead-memory-p1",
        status: "running",
        live_status_confirmed: true,
        created_at: "2024-06-01T12:00:00Z",
        updated_at: "2024-06-01T12:00:00Z",
      },
    ]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Switch to Containers
    fireEvent.click(screen.getByRole("tab", { name: "Containers" }));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Switch back to Sandboxes
    fireEvent.click(screen.getByRole("tab", { name: "Sandboxes" }));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Sandboxes content should be visible again
    expect(screen.getByRole("table", { name: "Sandboxes table" })).toBeInTheDocument();
    expect(screen.getByText("sandbox-1")).toBeInTheDocument();
    expect(screen.queryByRole("table", { name: "Containers table" })).not.toBeInTheDocument();
  });
});

describe("DockerPage — Error State Recovery", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("clears error and shows data when fetch recovers after initial failure", async () => {
    // First fetch fails (no prior data)
    mockGetSandboxes
      .mockRejectedValueOnce(new Error("Server unavailable"))
      .mockResolvedValueOnce([
        { name: "recovered-sbx", id: "r1", status: "running", managed: true },
      ]);

    render(<DockerPage />);

    // Initial fetch fails
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Error should be displayed
    expect(screen.getByText("Server unavailable")).toBeInTheDocument();
    expect(screen.queryByText("recovered-sbx")).not.toBeInTheDocument();

    // Next poll succeeds
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    // Error should be cleared and data displayed
    expect(screen.queryByText("Server unavailable")).not.toBeInTheDocument();
    expect(screen.getByText("recovered-sbx")).toBeInTheDocument();
  });

  it("clears error on Containers tab when fetch recovers", async () => {
    mockGetSandboxes.mockResolvedValue([]);
    mockGetMcpContainers
      .mockRejectedValueOnce(new Error("Docker daemon unreachable"))
      .mockResolvedValueOnce([
        {
          id: "c1",
          persona_id: "p1",
          persona_name: "Recovered Container",
          container_id: "docker-xyz",
          port: 9002,
          volume_name: "beachead-memory-p1",
          status: "stopped",
          live_status_confirmed: true,
          created_at: "2024-06-01T12:00:00Z",
          updated_at: "2024-06-01T12:00:00Z",
        },
      ]);

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Switch to Containers tab
    fireEvent.click(screen.getByRole("tab", { name: "Containers" }));

    // Initial containers fetch fails
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByText("Docker daemon unreachable")).toBeInTheDocument();

    // Next poll succeeds
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(screen.queryByText("Docker daemon unreachable")).not.toBeInTheDocument();
    expect(screen.getByText("Recovered Container")).toBeInTheDocument();
  });
});

describe("DockerPage — Container Action → Refresh → Poll Reset", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    mockGetSandboxes.mockResolvedValue([]);
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("refreshes container list and resets poll timer after stop action", async () => {
    mockGetMcpContainers.mockResolvedValue([
      {
        id: "c1",
        persona_id: "p1",
        persona_name: "Active Container",
        container_id: "docker-abc",
        port: 9001,
        volume_name: "beachead-memory-p1",
        status: "running",
        live_status_confirmed: true,
        created_at: "2024-06-01T12:00:00Z",
        updated_at: "2024-06-01T12:00:00Z",
      },
    ]);
    mockStopContainer.mockResolvedValue({
      id: "c1",
      persona_id: "p1",
      persona_name: "Active Container",
      container_id: "docker-abc",
      port: 9001,
      volume_name: "beachead-memory-p1",
      status: "stopped",
      live_status_confirmed: true,
      created_at: "2024-06-01T12:00:00Z",
      updated_at: "2024-06-01T12:00:00Z",
    });

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Switch to Containers tab
    fireEvent.click(screen.getByRole("tab", { name: "Containers" }));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockGetMcpContainers).toHaveBeenCalledTimes(1);

    // Advance 7 seconds (not yet at 10s interval)
    await act(async () => {
      await vi.advanceTimersByTimeAsync(7000);
    });

    expect(mockGetMcpContainers).toHaveBeenCalledTimes(1);

    // Perform stop action — triggers refresh() which resets the timer
    await act(async () => {
      fireEvent.click(screen.getByLabelText("Stop container Active Container"));
      await vi.advanceTimersByTimeAsync(0);
    });

    // refresh() triggered an immediate re-fetch
    expect(mockGetMcpContainers).toHaveBeenCalledTimes(2);

    // Advance 7 seconds from the refresh point — should NOT trigger another poll
    await act(async () => {
      await vi.advanceTimersByTimeAsync(7000);
    });

    expect(mockGetMcpContainers).toHaveBeenCalledTimes(2);

    // Advance 3 more seconds (total 10s from refresh) — should trigger next poll
    await act(async () => {
      await vi.advanceTimersByTimeAsync(3000);
    });

    expect(mockGetMcpContainers).toHaveBeenCalledTimes(3);
  });

  it("refreshes container list and resets poll timer after start action", async () => {
    mockGetMcpContainers.mockResolvedValue([
      {
        id: "c2",
        persona_id: "p2",
        persona_name: "Stopped Container",
        container_id: "docker-def",
        port: 9002,
        volume_name: "beachead-memory-p2",
        status: "stopped",
        live_status_confirmed: true,
        created_at: "2024-06-01T12:00:00Z",
        updated_at: "2024-06-01T12:00:00Z",
      },
    ]);
    mockStartContainer.mockResolvedValue({
      id: "c2",
      persona_id: "p2",
      persona_name: "Stopped Container",
      container_id: "docker-def",
      port: 9002,
      volume_name: "beachead-memory-p2",
      status: "running",
      live_status_confirmed: true,
      created_at: "2024-06-01T12:00:00Z",
      updated_at: "2024-06-01T12:00:00Z",
    });

    render(<DockerPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Switch to Containers tab
    fireEvent.click(screen.getByRole("tab", { name: "Containers" }));

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockGetMcpContainers).toHaveBeenCalledTimes(1);

    // Perform start action
    await act(async () => {
      fireEvent.click(screen.getByLabelText("Start container Stopped Container"));
      await vi.advanceTimersByTimeAsync(0);
    });

    // refresh() triggered an immediate re-fetch
    expect(mockGetMcpContainers).toHaveBeenCalledTimes(2);

    // Full 10s from refresh should trigger next poll
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(mockGetMcpContainers).toHaveBeenCalledTimes(3);
  });
});
