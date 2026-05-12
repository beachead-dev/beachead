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
