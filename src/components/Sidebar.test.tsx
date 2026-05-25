import { render, screen, act } from "@testing-library/react";
import { MemoryRouter } from "react-router-dom";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { Sidebar } from "./Sidebar";

// Mock the ResizeHandle component (not relevant to these tests)
vi.mock("./ResizeHandle", () => ({
  ResizeHandle: () => null,
}));

// Mock image imports
vi.mock("../assets/logo-dark.png", () => ({ default: "logo-dark.png" }));
vi.mock("../assets/logo-light.png", () => ({ default: "logo-light.png" }));
vi.mock("../assets/icon-dark.png", () => ({ default: "icon-dark.png" }));
vi.mock("../assets/icon-light.png", () => ({ default: "icon-light.png" }));

// Mock the API module
vi.mock("../lib/api", () => ({
  api: {
    get: vi.fn(),
    put: vi.fn(),
  },
  RepoSyncStatusResponse: undefined,
}));

import { api } from "../lib/api";

const mockApi = api as unknown as {
  get: ReturnType<typeof vi.fn>;
  put: ReturnType<typeof vi.fn>;
};

function renderSidebar(initialRoute = "/sessions") {
  return render(
    <MemoryRouter initialEntries={[initialRoute]}>
      <Sidebar />
    </MemoryRouter>
  );
}

describe("Sidebar — Repo Sync link", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
    // Default: no pending, theme loads fine
    mockApi.get.mockImplementation((path: string) => {
      if (path === "/api/repo-sync/status") return Promise.resolve({ has_pending: false });
      if (path === "/api/system/settings/theme") return Promise.resolve({ key: "theme", value: "system" });
      return Promise.resolve({});
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("renders 'Repo Sync' navigation link", async () => {
    renderSidebar();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    const link = screen.getByRole("link", { name: /Repo Sync/i });
    expect(link).toBeInTheDocument();
    expect(link).toHaveAttribute("href", "/repo-sync");
  });

  it("positions 'Repo Sync' link after Sessions and before Docker", async () => {
    renderSidebar();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    const links = screen.getAllByRole("link").filter(
      (el) => el.closest(".sidebar-nav")
    );
    const labels = links.map((el) => el.textContent?.trim());

    const sessionsIdx = labels.indexOf("Sessions");
    const repoSyncIdx = labels.indexOf("Repo Sync");
    const dockerIdx = labels.indexOf("Docker");

    expect(sessionsIdx).toBeGreaterThanOrEqual(0);
    expect(repoSyncIdx).toBeGreaterThan(sessionsIdx);
    expect(dockerIdx).toBeGreaterThan(repoSyncIdx);
  });

  it("applies active class when on /repo-sync route", async () => {
    renderSidebar("/repo-sync");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    const link = screen.getByRole("link", { name: /Repo Sync/i });
    expect(link).toHaveClass("sidebar-link--active");
  });

  it("does not apply active class when on a different route", async () => {
    renderSidebar("/sessions");

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    const link = screen.getByRole("link", { name: /Repo Sync/i });
    expect(link).not.toHaveClass("sidebar-link--active");
  });
});

describe("Sidebar — Notification badge", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("shows notification badge when has_pending is true", async () => {
    mockApi.get.mockImplementation((path: string) => {
      if (path === "/api/repo-sync/status") return Promise.resolve({ has_pending: true });
      if (path === "/api/system/settings/theme") return Promise.resolve({ key: "theme", value: "system" });
      return Promise.resolve({});
    });

    renderSidebar();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    const badge = screen.getByLabelText("Pending sync available");
    expect(badge).toBeInTheDocument();
    expect(badge).toHaveClass("sidebar-link-badge");
  });

  it("hides notification badge when has_pending is false", async () => {
    mockApi.get.mockImplementation((path: string) => {
      if (path === "/api/repo-sync/status") return Promise.resolve({ has_pending: false });
      if (path === "/api/system/settings/theme") return Promise.resolve({ key: "theme", value: "system" });
      return Promise.resolve({});
    });

    renderSidebar();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.queryByLabelText("Pending sync available")).not.toBeInTheDocument();
  });

  it("updates badge when status changes on poll interval", async () => {
    // First call returns no pending, second call returns pending
    mockApi.get
      .mockImplementation((path: string) => {
        if (path === "/api/system/settings/theme") return Promise.resolve({ key: "theme", value: "system" });
        if (path === "/api/repo-sync/status") return Promise.resolve({ has_pending: false });
        return Promise.resolve({});
      });

    renderSidebar();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Initially no badge
    expect(screen.queryByLabelText("Pending sync available")).not.toBeInTheDocument();

    // Now change the mock to return pending
    mockApi.get.mockImplementation((path: string) => {
      if (path === "/api/system/settings/theme") return Promise.resolve({ key: "theme", value: "system" });
      if (path === "/api/repo-sync/status") return Promise.resolve({ has_pending: true });
      return Promise.resolve({});
    });

    // Advance to next poll (60s)
    await act(async () => {
      vi.advanceTimersByTime(60000);
    });

    // Allow promises to resolve
    await act(async () => {
      await Promise.resolve();
    });

    // Badge should now appear
    expect(screen.getByLabelText("Pending sync available")).toBeInTheDocument();
  });

  it("does not crash when status API fails", async () => {
    mockApi.get.mockImplementation((path: string) => {
      if (path === "/api/repo-sync/status") return Promise.reject(new Error("Network error"));
      if (path === "/api/system/settings/theme") return Promise.resolve({ key: "theme", value: "system" });
      return Promise.resolve({});
    });

    renderSidebar();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Should render without badge and without crashing
    expect(screen.queryByLabelText("Pending sync available")).not.toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Repo Sync/i })).toBeInTheDocument();
  });
});
