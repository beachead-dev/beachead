import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { PersonasPage } from "./PersonasPage";

// Mock the Tauri dialog plugin
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockResolvedValue(null),
}));

// Mock the API module
vi.mock("../lib/api", () => ({
  api: {
    get: vi.fn(),
    post: vi.fn(),
    put: vi.fn(),
    del: vi.fn(),
  },
  getRepos: vi.fn(),
  deleteRepo: vi.fn(),
}));

import { api } from "../lib/api";
import { getRepos, deleteRepo } from "../lib/api";

const mockApi = api as unknown as {
  get: ReturnType<typeof vi.fn>;
  post: ReturnType<typeof vi.fn>;
  put: ReturnType<typeof vi.fn>;
  del: ReturnType<typeof vi.fn>;
};

const mockGetRepos = getRepos as ReturnType<typeof vi.fn>;
const mockDeleteRepo = deleteRepo as ReturnType<typeof vi.fn>;

const mockAgents = [
  {
    id: "agent-1",
    name: "Claude Code",
    is_builtin: true,
    metadata: {
      required_secrets: [],
      auth_methods: ["api_key"],
      description: "Claude Code agent",
      supports_interactive_auth: false,
    },
  },
];

const mockPersonas = [
  {
    id: "persona-1",
    name: "Test Persona",
    agent_type_id: "agent-1",
    workspace_path: "/home/user/project",
    memory_enabled: false,
    agent_cli_args: [],
    mcp_servers: [],
    additional_workspaces: [
      {
        id: "ws-1",
        persona_id: "persona-1",
        path: "/home/user/shared",
        read_only: true,
        position: 0,
        label: "Shared Libs",
        created_at: "2024-01-01T00:00:00Z",
      },
    ],
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  },
];

function setupApiMocks(personas = mockPersonas) {
  mockApi.get.mockImplementation((path: string) => {
    if (path === "/api/personas") return Promise.resolve(personas);
    if (path === "/api/agents") return Promise.resolve(mockAgents);
    if (path === "/api/secrets") return Promise.resolve([]);
    if (path === "/api/mcp-containers") return Promise.resolve([]);
    return Promise.resolve([]);
  });
  mockApi.post.mockResolvedValue({});
  mockApi.put.mockResolvedValue({});
}

describe("PersonasPage - Additional Workspaces", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders primary workspace with 'Primary' badge", async () => {
    setupApiMocks();
    render(<PersonasPage />);

    // Wait for data to load, then open the create form
    await waitFor(() => {
      expect(screen.getByText("+ New Persona")).toBeInTheDocument();
    });

    await userEvent.click(screen.getByText("+ New Persona"));

    // The primary workspace label should contain a "Primary" badge
    expect(screen.getByText("Primary")).toBeInTheDocument();
    expect(screen.getByText("Primary")).toHaveClass("badge");
  });

  it('"Add Workspace" button appends new entry', async () => {
    setupApiMocks();
    render(<PersonasPage />);

    await waitFor(() => {
      expect(screen.getByText("+ New Persona")).toBeInTheDocument();
    });

    await userEvent.click(screen.getByText("+ New Persona"));

    // Initially no workspace entries
    expect(screen.queryByLabelText("Additional workspace 1 path")).not.toBeInTheDocument();

    // Click "Add Workspace"
    await userEvent.click(screen.getByText("+ Add Workspace"));

    // Now there should be one entry
    expect(screen.getByLabelText("Additional workspace 1 path")).toBeInTheDocument();

    // Add another
    await userEvent.click(screen.getByText("+ Add Workspace"));
    expect(screen.getByLabelText("Additional workspace 2 path")).toBeInTheDocument();
  });

  it("remove button removes entry", async () => {
    setupApiMocks();
    render(<PersonasPage />);

    await waitFor(() => {
      expect(screen.getByText("+ New Persona")).toBeInTheDocument();
    });

    await userEvent.click(screen.getByText("+ New Persona"));

    // Add two entries
    await userEvent.click(screen.getByText("+ Add Workspace"));
    await userEvent.click(screen.getByText("+ Add Workspace"));

    expect(screen.getByLabelText("Additional workspace 1 path")).toBeInTheDocument();
    expect(screen.getByLabelText("Additional workspace 2 path")).toBeInTheDocument();

    // Remove the first entry
    await userEvent.click(screen.getByLabelText("Remove additional workspace 1"));

    // Only one entry should remain (now labeled as workspace 1)
    expect(screen.getByLabelText("Additional workspace 1 path")).toBeInTheDocument();
    expect(screen.queryByLabelText("Additional workspace 2 path")).not.toBeInTheDocument();
  });

  it("read-only toggle displays badge", async () => {
    setupApiMocks();
    render(<PersonasPage />);

    await waitFor(() => {
      expect(screen.getByText("+ New Persona")).toBeInTheDocument();
    });

    await userEvent.click(screen.getByText("+ New Persona"));
    await userEvent.click(screen.getByText("+ Add Workspace"));

    // Initially no RO badge
    expect(screen.queryByText("RO")).not.toBeInTheDocument();

    // Toggle read-only on
    await userEvent.click(screen.getByLabelText("Additional workspace 1 read-only"));

    // RO badge should appear
    expect(screen.getByText("RO")).toBeInTheDocument();
  });

  it("duplicate path detection shows inline error", async () => {
    setupApiMocks();
    render(<PersonasPage />);

    await waitFor(() => {
      expect(screen.getByText("+ New Persona")).toBeInTheDocument();
    });

    await userEvent.click(screen.getByText("+ New Persona"));

    // Set primary workspace
    await userEvent.type(screen.getByLabelText(/Workspace Path/), "/home/user/project");

    // Add two additional workspaces with the same path
    await userEvent.click(screen.getByText("+ Add Workspace"));
    await userEvent.click(screen.getByText("+ Add Workspace"));

    await userEvent.type(
      screen.getByLabelText("Additional workspace 1 path"),
      "/home/user/shared"
    );
    await userEvent.type(
      screen.getByLabelText("Additional workspace 2 path"),
      "/home/user/shared"
    );

    // Should show duplicate error on the second entry
    await waitFor(() => {
      expect(screen.getByText("Duplicate workspace path")).toBeInTheDocument();
    });
  });

  it("label input respects 64-char limit", async () => {
    setupApiMocks();
    render(<PersonasPage />);

    await waitFor(() => {
      expect(screen.getByText("+ New Persona")).toBeInTheDocument();
    });

    await userEvent.click(screen.getByText("+ New Persona"));
    await userEvent.click(screen.getByText("+ Add Workspace"));

    const labelInput = screen.getByLabelText("Additional workspace 1 label");

    // The input has maxLength=64
    expect(labelInput).toHaveAttribute("maxLength", "64");

    // Type a string longer than 64 chars - the component slices to 64
    const longString = "a".repeat(80);
    await userEvent.type(labelInput, longString);

    // Value should be truncated to 64 characters
    expect((labelInput as HTMLInputElement).value.length).toBeLessThanOrEqual(64);
  });

  it("empty additional workspaces shows only 'Add Workspace' button", async () => {
    setupApiMocks([
      {
        ...mockPersonas[0]!,
        additional_workspaces: [],
      },
    ]);
    render(<PersonasPage />);

    await waitFor(() => {
      expect(screen.getByText("+ New Persona")).toBeInTheDocument();
    });

    // Open create form (which starts with empty additional workspaces)
    await userEvent.click(screen.getByText("+ New Persona"));

    // The "Add Workspace" button should be visible
    expect(screen.getByText("+ Add Workspace")).toBeInTheDocument();

    // No workspace entry fields should be present
    expect(screen.queryByLabelText("Additional workspace 1 path")).not.toBeInTheDocument();

    // No remove buttons should be present
    expect(screen.queryByLabelText(/Remove additional workspace/)).not.toBeInTheDocument();
  });
});

describe("PersonasPage - Mirror Cleanup on Persona Deletion", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows mirror cleanup dialog when deleting persona with managed repos", async () => {
    setupApiMocks();
    mockGetRepos.mockResolvedValue([
      {
        id: "repo-1",
        persona_id: "persona-1",
        persona_name: "Test Persona",
        workspace_path: "/home/user/project",
        mirror_path: "/home/user/.local/share/beachead/mirrors/Test Persona/project",
        remote_url: "https://github.com/user/project.git",
        remote_provider: "github",
        branch_strategy: "direct",
        branch_pattern: null,
        attribution_mode: "keep_agent",
        sync_mode: "remote",
        secret_scan_mode: "block",
        check_interval_seconds: 300,
        sync_status: { workspace_ahead: 0, mirror_ahead: 0, remote_ahead: 0 },
        credential_status: "configured",
        mirror_exists: true,
        created_at: "2024-01-01T00:00:00Z",
        updated_at: "2024-01-01T00:00:00Z",
      },
    ]);
    mockDeleteRepo.mockResolvedValue(undefined);
    mockApi.del.mockResolvedValue(undefined);

    render(<PersonasPage />);

    await waitFor(() => {
      expect(screen.getByText("+ New Persona")).toBeInTheDocument();
    });

    // Click delete on the persona
    await userEvent.click(screen.getByLabelText("Delete Test Persona"));

    // Should show the mirror cleanup dialog with mirror path
    await waitFor(() => {
      expect(screen.getByText(/managed/)).toBeInTheDocument();
      expect(screen.getByText(/mirror directories/i)).toBeInTheDocument();
    });

    // Should show the mirror path
    expect(screen.getByText("/home/user/.local/share/beachead/mirrors/Test Persona/project")).toBeInTheDocument();

    // Should show both options: "Keep mirrors" and "Delete mirrors"
    expect(screen.getByRole("button", { name: /Keep mirrors/i })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Delete mirrors/i })).toBeInTheDocument();
  });

  it("calls deleteRepo with deleteMirror=true when 'Delete mirrors' is clicked", async () => {
    setupApiMocks();
    mockGetRepos.mockResolvedValue([
      {
        id: "repo-1",
        persona_id: "persona-1",
        persona_name: "Test Persona",
        workspace_path: "/home/user/project",
        mirror_path: "/home/user/.local/share/beachead/mirrors/Test Persona/project",
        remote_url: "https://github.com/user/project.git",
        remote_provider: "github",
        branch_strategy: "direct",
        branch_pattern: null,
        attribution_mode: "keep_agent",
        sync_mode: "remote",
        secret_scan_mode: "block",
        check_interval_seconds: 300,
        sync_status: { workspace_ahead: 0, mirror_ahead: 0, remote_ahead: 0 },
        credential_status: "configured",
        mirror_exists: true,
        created_at: "2024-01-01T00:00:00Z",
        updated_at: "2024-01-01T00:00:00Z",
      },
    ]);
    mockDeleteRepo.mockResolvedValue(undefined);
    mockApi.del.mockResolvedValue(undefined);

    render(<PersonasPage />);

    await waitFor(() => {
      expect(screen.getByText("+ New Persona")).toBeInTheDocument();
    });

    await userEvent.click(screen.getByLabelText("Delete Test Persona"));

    await waitFor(() => {
      expect(screen.getByRole("button", { name: /Delete mirrors/i })).toBeInTheDocument();
    });

    await userEvent.click(screen.getByRole("button", { name: /Delete mirrors/i }));

    await waitFor(() => {
      expect(mockDeleteRepo).toHaveBeenCalledWith("repo-1", true);
    });
  });

  it("shows simple delete dialog when persona has no managed repos", async () => {
    setupApiMocks();
    mockGetRepos.mockResolvedValue([]);
    mockApi.del.mockResolvedValue(undefined);

    render(<PersonasPage />);

    await waitFor(() => {
      expect(screen.getByText("+ New Persona")).toBeInTheDocument();
    });

    await userEvent.click(screen.getByLabelText("Delete Test Persona"));

    // Should show simple confirmation without mirror options
    await waitFor(() => {
      expect(screen.getByText(/Are you sure you want to delete this persona/)).toBeInTheDocument();
    });

    expect(screen.queryByRole("button", { name: /Keep mirrors/i })).not.toBeInTheDocument();
    expect(screen.queryByRole("button", { name: /Delete mirrors/i })).not.toBeInTheDocument();
    // The simple dialog has a "Delete" button (not "Keep mirrors" / "Delete mirrors")
    const deleteBtn = screen.getAllByRole("button").find(
      (btn) => btn.textContent === "Delete"
    );
    expect(deleteBtn).toBeInTheDocument();
  });
});
