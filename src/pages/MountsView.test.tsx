import { render, screen, waitFor } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { MountsView } from "./SessionsPage";

describe("MountsView", () => {
  let originalFetch: typeof globalThis.fetch;

  beforeEach(() => {
    originalFetch = globalThis.fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  function mockPersonaApi(persona: { id: string; [key: string]: unknown }) {
    globalThis.fetch = vi.fn().mockImplementation((url: string) => {
      if (typeof url === "string" && url.includes(`/api/personas/${persona.id}`)) {
        return Promise.resolve({
          ok: true,
          text: () => Promise.resolve(JSON.stringify(persona)),
        });
      }
      return Promise.resolve({
        ok: true,
        text: () => Promise.resolve(JSON.stringify([])),
      });
    });
  }

  it("displays workspace list with correct badges", async () => {
    const persona = {
      id: "persona-1",
      name: "Test Persona",
      agent_type_id: "claude-code",
      workspace_path: "/home/user/project",
      additional_workspaces: [
        {
          id: "ws-1",
          path: "/home/user/shared-libs",
          read_only: true,
          position: 0,
          label: "Shared Libs",
        },
        {
          id: "ws-2",
          path: "/home/user/data",
          read_only: false,
          position: 1,
          label: null,
        },
      ],
    };

    mockPersonaApi(persona);
    render(<MountsView personaId="persona-1" />);

    // Wait for persona data to load
    await waitFor(() => {
      expect(screen.getByText("Primary")).toBeInTheDocument();
    });

    // Verify primary workspace is shown
    expect(screen.getByText("/home/user/project")).toBeInTheDocument();
    expect(screen.getByText("Primary")).toBeInTheDocument();

    // Verify additional workspaces with correct badges
    expect(screen.getByText("Shared Libs")).toBeInTheDocument();
    expect(screen.getByText("/home/user/shared-libs")).toBeInTheDocument();
    expect(screen.getByText("Read-Only")).toBeInTheDocument();

    expect(screen.getByText("/home/user/data")).toBeInTheDocument();
    // Two Read-Write badges: one for primary, one for the second additional workspace
    const readWriteBadges = screen.getAllByText("Read-Write");
    expect(readWriteBadges.length).toBe(2);
  });

  it("shows label as primary text with path as tooltip", async () => {
    const persona = {
      id: "persona-2",
      name: "Labeled Persona",
      agent_type_id: "claude-code",
      workspace_path: "/home/user/main",
      additional_workspaces: [
        {
          id: "ws-3",
          path: "/home/user/very/long/path/to/shared/resources",
          read_only: false,
          position: 0,
          label: "Resources",
        },
      ],
    };

    mockPersonaApi(persona);
    render(<MountsView personaId="persona-2" />);

    // Wait for mounts data
    await waitFor(() => {
      expect(screen.getByText("Resources")).toBeInTheDocument();
    });

    // Label is shown as primary text
    expect(screen.getByText("Resources")).toBeInTheDocument();

    // Path is shown as secondary text
    const pathElement = screen.getByText("/home/user/very/long/path/to/shared/resources");
    expect(pathElement).toBeInTheDocument();

    // The mount-path span should have the title attribute with the full path (tooltip)
    const mountPathSpan = pathElement.closest(".mount-path");
    expect(mountPathSpan).toHaveAttribute("title", "/home/user/very/long/path/to/shared/resources");
  });

  it("shows primary workspace with Primary badge", async () => {
    const persona = {
      id: "persona-3",
      name: "Simple Persona",
      agent_type_id: "claude-code",
      workspace_path: "/opt/workspace",
      additional_workspaces: [],
    };

    mockPersonaApi(persona);
    render(<MountsView personaId="persona-3" />);

    // Wait for mounts data
    await waitFor(() => {
      expect(screen.getByText("Primary")).toBeInTheDocument();
    });

    // Primary workspace shown with badge
    expect(screen.getByText("/opt/workspace")).toBeInTheDocument();
    expect(screen.getByText("Primary")).toBeInTheDocument();
    expect(screen.getByText("Read-Write")).toBeInTheDocument();

    // No additional workspaces message
    expect(screen.getByText("No additional workspaces configured.")).toBeInTheDocument();
  });
});
