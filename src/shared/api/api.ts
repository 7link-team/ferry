import { i18n } from "../i18n/i18n";
import { localeHeaders } from "../i18n/locale";
import { beginActivity } from "./progress";
import { clearSession, deferUnauthorized, observeSession, restoreInitialSession, waitForSessionRenewal } from "./session";

export interface AvatarValue {
  style: "portrait" | "adventurer" | "notionists" | "lorelei" | "pixel-art" | "pixel" | "initials" | "emoji" | "image";
  seed?: string | undefined;
  gender?: "neutral" | "male" | "female" | undefined;
  emoji?: string;
  image?: string;
}

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
    if (input.startsWith("/api/")) await waitForSessionRenewal(input.slice(4));
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
    await waitForSessionRenewal(path);
    options.signal?.throwIfAborted();
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
  if (response.status === 401 && (!options.method || options.method === "GET") && await restoreInitialSession(path))
    return requestApi<T>(path, options);
  if (response.status === 401 && path !== "/sessions" && !deferUnauthorized())
    window.dispatchEvent(new Event("ferry:unauthorized"));
  const plainText = response.headers?.get("content-type")?.startsWith("text/plain") ?? false;
  const data = plainText ? {} : await response.json().catch(() => ({}));
  if (response.ok && path === "/me") observeSession(data.session);
  if (response.ok && path === "/sessions" && options.method === "DELETE") clearSession();
  if (!response.ok) throw new ApiError(data.error ?? (plainText ? await plainError(response) : undefined) ?? failure(response.status), response.status);
  return data as T;
}

/** A generic, localized failure that still says which HTTP status came back. */
function failure(status: number) {
  return status ? i18n.t("errors.http", { status }) : i18n.t("errors.operation");
}

/**
 * Plain-text error bodies are shown only when Ferry wrote them: its localized
 * responses carry Content-Language. Proxy pages and framework rejections are not
 * user wording; they stay in the console.
 */
async function plainError(response: Response) {
  const text = (await response.text().catch(() => "")).trim();
  try {
    const parsed = JSON.parse(text) as { error?: unknown };
    if (typeof parsed?.error === "string" && parsed.error) return parsed.error;
  } catch { /* not Ferry's JSON error shape */ }
  const own = !!response.headers?.get("content-language");
  if (own && text && text.length <= 300 && !/<[a-z!/]/i.test(text)) return text;
  if (text) console.debug(`HTTP ${response.status} response body`, text.slice(0, 500));
  return undefined;
}

export const errorText = (error: unknown) =>
  error instanceof Error ? error.message : i18n.t("errors.operation");

export interface Workspace {
  avatar?: AvatarValue | null;
  timezone?: string | null;
  id: string;
  slug: string;
  name: string;
  role_name: string;
  /** owner: created it; member: joined with role_name; none: a platform administrator who has not joined. */
  relation?: "owner" | "member" | "none";
  permissions: string[];
}

export interface Identity {
  user: { id: string; email: string; name: string; is_superadmin: boolean; avatar?: AvatarValue | null | undefined };
  workspaces: Workspace[];
}

export interface Status {
  ok: boolean;
  version: string;
  protocol: number;
  instance_id: string;
  server_mode: boolean;
}
