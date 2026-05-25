import { render, screen, act, fireEvent } from "@testing-library/react";
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { RepoSyncPage } from "./RepoSyncPage";

// Mock the API module
vi.mock("../lib/api", () => ({
  getRepos: vi.fn(),
  scanWorkspaces: vi.fn(),
  enableRepo: vi.fn(),
  getCommits: vi.fn(),
  pullFromAgent: vi.fn(),
  fetchFromRemote: vi.fn(),
  pushToRemote: vi.fn(),
  pushToAgent: vi.fn(),
  getMirrorsDir: vi.fn().mockResolvedValue({ path: "/home/user/.local/share/beachead/mirrors" }),
}));

// Mock child components that have their own complex behavior
vi.mock("../components/CommitReviewModal", () => ({
  CommitReviewModal: ({ open, commits, onClose, onPushComplete }: {
    open: boolean;
    commits: { sha: string; message: string }[];
    onClose: () => void;
    onPushComplete: () => void;
  }) =>
    open ? (
      <div data-testid="commit-review-modal">
        <p>Review Commits</p>
        {commits.map((c) => (
          <span key={c.sha}>{c.message}</span>
        ))}
        <button onClick={onClose}>Cancel</button>
        <button onClick={onPushComplete}>Push</button>
      </div>
    ) : null,
}));

vi.mock("../components/RepoSettingsPanel", () => ({
  RepoSettingsPanel: () => <div data-testid="repo-settings-panel">Settings Panel</div>,
}));

vi.mock("../components/SecretScanWarningModal", () => ({
  SecretScanWarningModal: () => null,
  parseSecretScanError: () => null,
}));

import {
  getRepos,
  getCommits,
} from "../lib/api";

const mockGetRepos = getRepos as ReturnType<typeof vi.fn>;
const mockGetCommits = getCommits as ReturnType<typeof vi.fn>;

function makeManagedRepo(overrides: Partial<{
  id: string;
  persona_id: string;
  persona_name: string;
  workspace_path: string;
  mirror_path: string;
  remote_url: string | null;
  remote_provider: string | null;
  branch_strategy: string;
  branch_pattern: string | null;
  attribution_mode: string;
  sync_mode: string;
  secret_scan_mode: string;
  check_interval_seconds: number;
  sync_status: { workspace_ahead: number; mirror_ahead: number; remote_ahead: number };
  credential_status: string;
  mirror_exists: boolean;
  created_at: string;
  updated_at: string;
}> = {}) {
  return {
    id: "repo-1",
    persona_id: "p1",
    persona_name: "Alpha",
    workspace_path: "/home/user/projects/my-app",
    mirror_path: "/home/user/.local/share/beachead/mirrors/Alpha/my-app",
    remote_url: "https://github.com/user/my-app.git",
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
    ...overrides,
  };
}

describe("RepoSyncPage — Repository List Rendering", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("displays repos grouped by persona and sorted alphabetically", async () => {
    mockGetRepos.mockResolvedValue([
      makeManagedRepo({ id: "r1", persona_name: "Beta", workspace_path: "/projects/zebra" }),
      makeManagedRepo({ id: "r2", persona_name: "Alpha", workspace_path: "/projects/banana" }),
      makeManagedRepo({ id: "r3", persona_name: "Beta", workspace_path: "/projects/apple" }),
      makeManagedRepo({ id: "r4", persona_name: "Alpha", workspace_path: "/projects/cherry" }),
    ]);

    render(<RepoSyncPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Both persona groups should be present
    expect(screen.getByText("Alpha")).toBeInTheDocument();
    expect(screen.getByText("Beta")).toBeInTheDocument();

    // Repos should be rendered with their folder names
    expect(screen.getByText("banana")).toBeInTheDocument();
    expect(screen.getByText("cherry")).toBeInTheDocument();
    expect(screen.getByText("apple")).toBeInTheDocument();
    expect(screen.getByText("zebra")).toBeInTheDocument();

    // Verify alphabetical ordering: Alpha group before Beta group
    const groupTitles = screen.getAllByRole("heading", { level: 3 });
    const personaNames = groupTitles.map((h) => h.textContent);
    expect(personaNames.indexOf("Alpha")).toBeLessThan(personaNames.indexOf("Beta"));
  });

  it("shows empty state message when no repos exist", async () => {
    mockGetRepos.mockResolvedValue([]);

    render(<RepoSyncPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(
      screen.getByText(/Repo Sync is not enabled for any repositories/),
    ).toBeInTheDocument();
  });

  it("shows loading indicator during initial fetch", () => {
    mockGetRepos.mockReturnValue(new Promise(() => {}));

    render(<RepoSyncPage />);

    expect(screen.getByText("Loading repositories…")).toBeInTheDocument();
  });

  it("shows sync status indicators for repos", async () => {
    mockGetRepos.mockResolvedValue([
      makeManagedRepo({
        id: "r1",
        sync_mode: "remote",
        sync_status: { workspace_ahead: 3, mirror_ahead: 2, remote_ahead: 1 },
      }),
    ]);

    render(<RepoSyncPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByText("3 ahead")).toBeInTheDocument();
    expect(screen.getByText(/2 ahead/)).toBeInTheDocument();
    expect(screen.getByText(/1 behind/)).toBeInTheDocument();
  });
});

describe("RepoSyncPage — Sync Button Disabled States", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("disables 'Push to remote' and 'Fetch from remote' when sync_mode is local_only", async () => {
    mockGetRepos.mockResolvedValue([
      makeManagedRepo({
        id: "r1",
        persona_name: "Local Persona",
        workspace_path: "/projects/local-repo",
        sync_mode: "local_only",
        remote_url: null,
      }),
    ]);

    render(<RepoSyncPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    const pushBtn = screen.getByLabelText("Push to remote for local-repo");
    const fetchBtn = screen.getByLabelText("Fetch from remote for local-repo");
    const pullBtn = screen.getByLabelText("Pull from agent for local-repo");
    const pushAgentBtn = screen.getByLabelText("Push to agent for local-repo");

    expect(pushBtn).toBeDisabled();
    expect(fetchBtn).toBeDisabled();
    expect(pullBtn).toBeEnabled();
    expect(pushAgentBtn).toBeEnabled();
  });

  it("enables all sync buttons when sync_mode is remote", async () => {
    mockGetRepos.mockResolvedValue([
      makeManagedRepo({
        id: "r1",
        persona_name: "Remote Persona",
        workspace_path: "/projects/remote-repo",
        sync_mode: "remote",
      }),
    ]);

    render(<RepoSyncPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    const pushBtn = screen.getByLabelText("Push to remote for remote-repo");
    const fetchBtn = screen.getByLabelText("Fetch from remote for remote-repo");
    const pullBtn = screen.getByLabelText("Pull from agent for remote-repo");
    const pushAgentBtn = screen.getByLabelText("Push to agent for remote-repo");

    expect(pushBtn).toBeEnabled();
    expect(fetchBtn).toBeEnabled();
    expect(pullBtn).toBeEnabled();
    expect(pushAgentBtn).toBeEnabled();
  });
});

describe("RepoSyncPage — Commit Review Modal", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("opens commit review modal with commits when 'Push to remote' is clicked", async () => {
    mockGetRepos.mockResolvedValue([
      makeManagedRepo({
        id: "repo-push",
        persona_name: "Dev",
        workspace_path: "/projects/push-test",
        sync_mode: "remote",
      }),
    ]);

    mockGetCommits.mockResolvedValue([
      {
        sha: "abc1234567890",
        message: "feat: add login",
        author: "dev@example.com",
        timestamp: "2024-06-01T12:00:00Z",
        files_changed: 3,
        insertions: 50,
        deletions: 10,
      },
      {
        sha: "def4567890123",
        message: "fix: typo in readme",
        author: "dev@example.com",
        timestamp: "2024-06-02T12:00:00Z",
        files_changed: 1,
        insertions: 1,
        deletions: 1,
      },
    ]);

    render(<RepoSyncPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Click "Push to remote"
    await act(async () => {
      fireEvent.click(screen.getByLabelText("Push to remote for push-test"));
      await vi.advanceTimersByTimeAsync(0);
    });

    // Modal should be open with commit messages
    expect(screen.getByTestId("commit-review-modal")).toBeInTheDocument();
    expect(screen.getByText("feat: add login")).toBeInTheDocument();
    expect(screen.getByText("fix: typo in readme")).toBeInTheDocument();
  });

  it("shows error when no commits to push", async () => {
    mockGetRepos.mockResolvedValue([
      makeManagedRepo({
        id: "repo-empty",
        persona_name: "Dev",
        workspace_path: "/projects/empty-push",
        sync_mode: "remote",
      }),
    ]);

    mockGetCommits.mockResolvedValue([]);

    render(<RepoSyncPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Click "Push to remote"
    await act(async () => {
      fireEvent.click(screen.getByLabelText("Push to remote for empty-push"));
      await vi.advanceTimersByTimeAsync(0);
    });

    // Should show error, not modal
    expect(screen.queryByTestId("commit-review-modal")).not.toBeInTheDocument();
    expect(screen.getByText("No commits to push.")).toBeInTheDocument();
  });

  it("closes commit review modal on cancel", async () => {
    mockGetRepos.mockResolvedValue([
      makeManagedRepo({
        id: "repo-cancel",
        persona_name: "Dev",
        workspace_path: "/projects/cancel-test",
        sync_mode: "remote",
      }),
    ]);

    mockGetCommits.mockResolvedValue([
      {
        sha: "aaa1111111111",
        message: "some commit",
        author: "dev@example.com",
        timestamp: "2024-06-01T12:00:00Z",
        files_changed: 1,
        insertions: 5,
        deletions: 0,
      },
    ]);

    render(<RepoSyncPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // Open modal
    await act(async () => {
      fireEvent.click(screen.getByLabelText("Push to remote for cancel-test"));
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByTestId("commit-review-modal")).toBeInTheDocument();

    // Click cancel
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(screen.queryByTestId("commit-review-modal")).not.toBeInTheDocument();
  });
});

describe("RepoSyncPage — Polling Behavior", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("polls getRepos at 10-second intervals", async () => {
    mockGetRepos.mockResolvedValue([]);

    render(<RepoSyncPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockGetRepos).toHaveBeenCalledTimes(1);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(mockGetRepos).toHaveBeenCalledTimes(2);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(10000);
    });

    expect(mockGetRepos).toHaveBeenCalledTimes(3);
  });

  it("stops polling when page is hidden and resumes when visible", async () => {
    mockGetRepos.mockResolvedValue([]);

    render(<RepoSyncPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockGetRepos).toHaveBeenCalledTimes(1);

    // Simulate page becoming hidden
    Object.defineProperty(document, "hidden", { value: true, writable: true });
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
    });

    // Advance time — should not poll while hidden
    const callsBeforeHidden = mockGetRepos.mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30000);
    });

    expect(mockGetRepos).toHaveBeenCalledTimes(callsBeforeHidden);

    // Simulate page becoming visible again
    Object.defineProperty(document, "hidden", { value: false, writable: true });
    await act(async () => {
      document.dispatchEvent(new Event("visibilitychange"));
    });

    // Should resume polling immediately
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockGetRepos.mock.calls.length).toBeGreaterThan(callsBeforeHidden);
  });

  it("stops polling on unmount", async () => {
    mockGetRepos.mockResolvedValue([]);

    const { unmount } = render(<RepoSyncPage />);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    expect(mockGetRepos).toHaveBeenCalledTimes(1);

    unmount();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(30000);
    });

    expect(mockGetRepos).toHaveBeenCalledTimes(1);
  });
});
