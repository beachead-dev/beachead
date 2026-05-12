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
}));

import { api } from "../lib/api";

const mockApi = api as {
  get: ReturnType<typeof vi.fn>;
  post: ReturnType<typeof vi.fn>;
  put: ReturnType<typeof vi.fn>;
  del: ReturnType<typeof vi.fn>;
};

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
