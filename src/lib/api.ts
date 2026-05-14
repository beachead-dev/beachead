const BASE_URL = "http://127.0.0.1:9876";

export class ApiError extends Error {
  constructor(
    public status: number,
    public statusText: string,
    public body: unknown,
  ) {
    // Extract the backend error message if available
    const backendMessage =
      body && typeof body === "object" && "error" in body
        ? (body as { error: { message?: string } }).error?.message
        : null;
    super(backendMessage || `API error ${status}: ${statusText}`);
    this.name = "ApiError";
  }
}

async function handleResponse<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const text = await response.text();
    let body: unknown;
    try {
      body = JSON.parse(text);
    } catch {
      body = text;
    }
    throw new ApiError(response.status, response.statusText, body);
  }
  const text = await response.text();
  if (!text) return undefined as T;
  return JSON.parse(text) as T;
}

export async function get<T>(path: string): Promise<T> {
  const response = await fetch(`${BASE_URL}${path}`, {
    method: "GET",
    headers: { "Content-Type": "application/json" },
  });
  return handleResponse<T>(response);
}

export async function post<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(`${BASE_URL}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  return handleResponse<T>(response);
}

export async function put<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(`${BASE_URL}${path}`, {
    method: "PUT",
    headers: { "Content-Type": "application/json" },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  return handleResponse<T>(response);
}

export async function del<T>(path: string): Promise<T> {
  const response = await fetch(`${BASE_URL}${path}`, {
    method: "DELETE",
    headers: { "Content-Type": "application/json" },
  });
  return handleResponse<T>(response);
}

export async function getText(path: string): Promise<string> {
  const response = await fetch(`${BASE_URL}${path}`, {
    method: "GET",
  });
  if (!response.ok) {
    const text = await response.text();
    let body: unknown;
    try {
      body = JSON.parse(text);
    } catch {
      body = text;
    }
    throw new ApiError(response.status, response.statusText, body);
  }
  return response.text();
}

export async function postForBlob(path: string, body?: unknown): Promise<Blob> {
  const response = await fetch(`${BASE_URL}${path}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: body !== undefined ? JSON.stringify(body) : undefined,
  });
  if (!response.ok) {
    const text = await response.text();
    let errBody: unknown;
    try {
      errBody = JSON.parse(text);
    } catch {
      errBody = text;
    }
    throw new ApiError(response.status, response.statusText, errBody);
  }
  return response.blob();
}

export const api = { get, getText, post, postForBlob, put, del };

// Docker Management Types

export interface SandboxInfo {
  name: string | null;
  id: string | null;
  agent: string | null;
  status: string | null;
  managed: boolean;
}

export interface SandboxActionResponse {
  id: string;
  status: string;
}

export interface SandboxStartResponse {
  id: string;
}

export interface McpContainerResponse {
  id: string;
  persona_id: string;
  persona_name: string;
  container_id: string | null;
  image: string;
  port: number;
  volume_name: string;
  status: string;
  live_status_confirmed: boolean;
  created_at: string;
  updated_at: string;
}

// Docker Management API Methods

export async function getSandboxes(
  showAll?: boolean,
): Promise<SandboxInfo[]> {
  const params = showAll ? "?show_all=true" : "";
  return get<SandboxInfo[]>(`/api/sandboxes${params}`);
}

export async function stopSandbox(
  id: string,
): Promise<SandboxActionResponse> {
  return post<SandboxActionResponse>(`/api/sandboxes/${id}/stop`);
}

export async function startSandbox(
  id: string,
): Promise<SandboxStartResponse> {
  return post<SandboxStartResponse>(`/api/sandboxes/${id}/start`);
}

export async function removeSandbox(id: string): Promise<void> {
  return del<void>(`/api/sandboxes/${id}`);
}

export async function getMcpContainers(
  showAll?: boolean,
): Promise<McpContainerResponse[]> {
  const params = showAll ? "?show_all=true" : "";
  return get<McpContainerResponse[]>(`/api/mcp-containers${params}`);
}

export async function startContainer(
  id: string,
): Promise<McpContainerResponse> {
  return post<McpContainerResponse>(`/api/mcp-containers/${id}/start`);
}

export async function stopContainer(
  id: string,
): Promise<McpContainerResponse> {
  return post<McpContainerResponse>(`/api/mcp-containers/${id}/stop`);
}

export async function removeContainer(
  id: string,
  deleteVolume: boolean,
): Promise<void> {
  return del<void>(
    `/api/mcp-containers/${id}?delete_volume=${deleteVolume}`,
  );
}

// Repo Sync Types

export interface SyncStatus {
  workspace_ahead: number;
  mirror_ahead: number;
  remote_ahead: number;
}

export interface ManagedRepoResponse {
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
  sync_status: SyncStatus;
  credential_status: string;
  mirror_exists: boolean;
  created_at: string;
  updated_at: string;
}

export interface DetectedRepo {
  workspace_path: string;
  persona_id: string;
  persona_name: string;
  has_remotes: boolean;
  remote_url: string | null;
}

export interface CommitInfo {
  sha: string;
  message: string;
  author: string;
  timestamp: string;
  files_changed: number;
  insertions: number;
  deletions: number;
}

export interface SyncResult {
  commits: number;
}

export interface PushResult {
  branch: string;
  commits: number;
}

export interface RepoSyncStatusResponse {
  has_pending: boolean;
}

export interface MirrorsDirResponse {
  path: string;
}

export interface EnableRepoRequest {
  persona_id: string;
  workspace_path: string;
  remote_url?: string;
}

export interface UpdateRepoRequest {
  remote_url?: string;
  remote_provider?: string;
  branch_strategy?: string;
  branch_pattern?: string;
  attribution_mode?: string;
  sync_mode?: string;
  secret_scan_mode?: string;
  check_interval_seconds?: number;
}

export interface SetCredentialsRequest {
  username: string;
  secret: string;
  credential_type: "token" | "username_password";
}

export interface PushToRemoteRequest {
  commit_shas: string[];
  squash: boolean;
  squash_message?: string;
}

// Repo Sync API Methods

export async function getRepos(): Promise<ManagedRepoResponse[]> {
  return get<ManagedRepoResponse[]>("/api/repo-sync/repos");
}

export async function enableRepo(
  req: EnableRepoRequest,
): Promise<ManagedRepoResponse> {
  return post<ManagedRepoResponse>("/api/repo-sync/repos", req);
}

export async function updateRepo(
  id: string,
  req: UpdateRepoRequest,
): Promise<ManagedRepoResponse> {
  return put<ManagedRepoResponse>(`/api/repo-sync/repos/${id}`, req);
}

export async function deleteRepo(
  id: string,
  deleteMirror?: boolean,
): Promise<void> {
  const params = deleteMirror ? "?delete_mirror=true" : "";
  return del<void>(`/api/repo-sync/repos/${id}${params}`);
}

export async function scanWorkspaces(): Promise<DetectedRepo[]> {
  return post<DetectedRepo[]>("/api/repo-sync/scan");
}

export async function pullFromAgent(id: string): Promise<SyncResult> {
  return post<SyncResult>(`/api/repo-sync/repos/${id}/pull-from-agent`);
}

export async function pushToRemote(
  id: string,
  req: PushToRemoteRequest,
): Promise<PushResult> {
  return post<PushResult>(`/api/repo-sync/repos/${id}/push-to-remote`, req);
}

export async function fetchFromRemote(id: string): Promise<SyncResult> {
  return post<SyncResult>(`/api/repo-sync/repos/${id}/fetch-from-remote`);
}

export async function pushToAgent(id: string): Promise<SyncResult> {
  return post<SyncResult>(`/api/repo-sync/repos/${id}/push-to-agent`);
}

export async function getCommits(id: string): Promise<CommitInfo[]> {
  return get<CommitInfo[]>(`/api/repo-sync/repos/${id}/commits`);
}

export async function setCredentials(
  id: string,
  req: SetCredentialsRequest,
): Promise<{ status: string }> {
  return put<{ status: string }>(`/api/repo-sync/repos/${id}/credentials`, req);
}

export async function deleteCredentials(id: string): Promise<void> {
  return del<void>(`/api/repo-sync/repos/${id}/credentials`);
}

export async function getRepoSyncStatus(): Promise<RepoSyncStatusResponse> {
  return get<RepoSyncStatusResponse>("/api/repo-sync/status");
}

export async function getMirrorsDir(): Promise<MirrorsDirResponse> {
  return get<MirrorsDirResponse>("/api/repo-sync/settings/mirrors-dir");
}

export async function setMirrorsDir(path: string): Promise<MirrorsDirResponse> {
  return put<MirrorsDirResponse>("/api/repo-sync/settings/mirrors-dir", { path });
}
