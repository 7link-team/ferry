/**
 * The client half of the list contract in
 * docs/ferry-运维平台产品与技术方案.md §6. One place builds resource URLs and
 * reads the envelope, so a resource page never assembles a query string.
 */
import { api } from "./api";

export interface Envelope<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

export const base = (workspace: string, resource: string) =>
  `/workspaces/${encodeURIComponent(workspace)}/${resource}`;

/** Drops empty values so a cleared filter leaves the URL instead of sending `=`. */
export function queryString(params: Record<string, string | number | undefined>) {
  const search = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (value === undefined || value === "" || value === null) continue;
    search.set(key, String(value));
  }
  const text = search.toString();
  return text ? `?${text}` : "";
}

export const listResource = <T>(
  workspace: string,
  resource: string,
  params: Record<string, string | number | undefined>,
  signal?: AbortSignal,
  endpoint?: string,
) =>
  api<Envelope<T>>(`${endpoint ?? base(workspace, resource)}${queryString(params)}`, signal ? { signal } : {});

export const getResource = <T>(workspace: string, resource: string, id: string, endpoint?: string) =>
  api<T>(`${endpoint ?? base(workspace, resource)}/${encodeURIComponent(id)}`);

export const createResource = <T>(workspace: string, resource: string, body: unknown, endpoint?: string) =>
  api<T>(endpoint ?? base(workspace, resource), { method: "POST", body: JSON.stringify(body) });

export const patchResource = <T>(
  workspace: string,
  resource: string,
  id: string,
  body: unknown,
  endpoint?: string,
) =>
  api<T>(`${endpoint ?? base(workspace, resource)}/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: JSON.stringify(body),
  });

export const deleteResource = (workspace: string, resource: string, id: string) =>
  api<void>(`${base(workspace, resource)}/${encodeURIComponent(id)}`, { method: "DELETE" });

/** Non-CRUD verbs live under /actions/, per the REST contract.
 *  Collection-level: POST /{resource}/actions/{verb} */
export const runAction = <T>(
  workspace: string,
  resource: string,
  action: string,
  body: unknown,
) =>
  api<T>(`${base(workspace, resource)}/actions/${action}`, {
    method: "POST",
    body: JSON.stringify(body),
  });

/** Item-level: POST /{resource}/{id}/actions/{verb} */
export const runItemAction = <T>(
  workspace: string,
  resource: string,
  id: string,
  action: string,
  body: unknown,
) =>
  api<T>(`${base(workspace, resource)}/${encodeURIComponent(id)}/actions/${action}`, {
    method: "POST",
    body: JSON.stringify(body),
  });

/**
 * Page numbers with gaps: `1 2 3 … 98 99`. Always shows the first and last
 * page plus a window around the current one; `null` is a gap.
 */
export function pageRange(page: number, pageCount: number, window = 1): (number | null)[] {
  if (pageCount <= 1) return pageCount === 1 ? [1] : [];
  const keep = new Set<number>([1, pageCount]);
  for (let i = page - window; i <= page + window; i++) {
    if (i >= 1 && i <= pageCount) keep.add(i);
  }
  // Without this the control changes width as you page through, which makes
  // the next page number move out from under the cursor.
  if (page - window <= 3) for (let i = 2; i <= Math.min(3 + window, pageCount); i++) keep.add(i);
  if (page + window >= pageCount - 2)
    for (let i = Math.max(pageCount - 2 - window, 1); i < pageCount; i++) keep.add(i);

  const sorted = [...keep].sort((a, b) => a - b);
  const result: (number | null)[] = [];
  let previous = 0;
  for (const value of sorted) {
    if (previous && value - previous > 1) result.push(null);
    result.push(value);
    previous = value;
  }
  return result;
}

export const PAGE_SIZES = [10, 20, 50, 100] as const;
/** Auto-refresh choices in seconds; 0 is off. */
export const REFRESH_INTERVALS = [0, 10, 30, 60] as const;
