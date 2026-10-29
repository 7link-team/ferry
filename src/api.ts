export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

export async function api<T>(path: string, options: RequestInit = {}): Promise<T> {
  let response: Response;
  try {
    response = await fetch(`/api${path}`, {
      ...options,
      credentials: "same-origin",
      headers: {
        ...(options.body instanceof FormData ? {} : { "Content-Type": "application/json" }),
        "X-Ferry-Request": "1",
        ...options.headers,
      },
    });
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") throw error;
    throw new ApiError("无法连接 Ferry 服务，请检查网络后重试。", 0);
  }
  if (response.status === 401 && path !== "/sessions")
    window.dispatchEvent(new Event("ferry:unauthorized"));
  const plainText = response.headers?.get("content-type")?.startsWith("text/plain") ?? false;
  const data = plainText ? {} : await response.json().catch(() => ({}));
  const detail = !response.ok && plainText ? (await response.text()).trim() : "";
  if (!response.ok) throw new ApiError(data.error ?? (detail && detail.length <= 160 ? detail : "操作失败，请重试。"), response.status);
  return data as T;
}

export const errorText = (error: unknown) =>
  error instanceof Error ? error.message : "操作失败，请重试。";

export interface Workspace {
  id: string;
  slug: string;
  name: string;
  role_name: string;
  permissions: string[];
}

export interface Identity {
  user: { id: string; email: string; name: string; is_superadmin: boolean };
  workspaces: Workspace[];
}

export interface Status {
  ok: boolean;
  version: string;
  protocol: number;
  instance_id: string;
  server_mode: boolean;
}
