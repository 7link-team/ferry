import { i18n } from "../i18n/i18n";
import { localeHeaders } from "../i18n/locale";
import { beginActivity } from "./progress";

export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
  ) {
    super(message);
  }
}

/** Raw same-origin API requests retain streaming bodies and response handling. */
export async function localizedFetch(input: string, options: RequestInit = {}): Promise<Response> {
  const finish = beginActivity("request", input);
  try {
    // Raw streams finish their initial request at headers, without consuming the body.
    return await fetch(input, { ...options, headers: localeHeaders(options.headers) });
  } finally { finish(); }
}

export async function api<T>(path: string, options: RequestInit = {}): Promise<T> {
  const finish = beginActivity("request", `/api${path}`);
  try { return await requestApi<T>(path, options); }
  finally { finish(); }
}

async function requestApi<T>(path: string, options: RequestInit): Promise<T> {
  let response: Response;
  try {
    const headers = new Headers(options.headers);
    if (!(options.body instanceof FormData) && !headers.has("Content-Type")) headers.set("Content-Type", "application/json");
    if (!headers.has("X-Ferry-Request")) headers.set("X-Ferry-Request", "1");
    response = await fetch(`/api${path}`, {
      ...options,
      credentials: "same-origin",
      headers: localeHeaders(headers),
    });
  } catch (error) {
    if (error instanceof DOMException && error.name === "AbortError") throw error;
    throw new ApiError(i18n.t("errors.network"), 0);
  }
  if (response.status === 401 && path !== "/sessions")
    window.dispatchEvent(new Event("ferry:unauthorized"));
  const plainText = response.headers?.get("content-type")?.startsWith("text/plain") ?? false;
  const data = plainText ? {} : await response.json().catch(() => ({}));
  const detail = !response.ok && plainText ? (await response.text()).trim() : "";
  if (!response.ok) throw new ApiError(data.error ?? (detail && detail.length <= 160 ? detail : i18n.t("errors.operation")), response.status);
  return data as T;
}

export const errorText = (error: unknown) =>
  error instanceof Error ? error.message : i18n.t("errors.operation");

export interface Workspace {
  id: string;
  slug: string;
  name: string;
  role_name: string;
  /** owner: created it; member: joined with role_name; none: a platform administrator who has not joined. */
  relation?: "owner" | "member" | "none";
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
