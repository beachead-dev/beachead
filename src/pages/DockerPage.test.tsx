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

import { getSandboxes, stopSandbox, startSandbox, removeSandbox, getMcpContainers, removeContainer } from "../lib/api";

const mockGetSandboxes = getSandboxes as ReturnType<typeof vi.fn>;
const mockStopSandbox = stopSandbox as ReturnType<typeof vi.fn>;
const mockStartSandbox = startSandbox as ReturnType<typeof vi.fn>;
const mockRemoveSandbox = removeSandbox as ReturnType<typeof vi.fn>;
const mockGetMcpContainers = getMcpContainers as ReturnType<typeof vi.fn>;
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
