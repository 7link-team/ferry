import { useEffect, useState } from "react";
import { api, errorText } from "../../shared/api/api";
import { base, listResource } from "../../shared/api/resource";

export interface HostRow { id: string; name: string; address?: string }

/** Every host of the workspace, page by page. */
export async function allHosts(workspace: string, signal?: AbortSignal): Promise<HostRow[]> {
  const rows: HostRow[] = [];
  for (let page = 1; ; page += 1) {
    const result = await listResource<HostRow>(workspace, "hosts", { page, page_size: 200 }, signal);
    rows.push(...result.items);
    if (!result.items.length || page * result.page_size >= result.total) return rows;
  }
}

/** Host names by ID; empty without `hosts.read` or while loading (callers fall back to the ID). */
export function useHostNames(workspace: string, enabled: boolean): Map<string, string> {
  const [names, setNames] = useState(new Map<string, string>());
  useEffect(() => {
    if (!enabled) return;
    const controller = new AbortController();
    allHosts(workspace, controller.signal)
      .then(rows => setNames(new Map(rows.map(row => [row.id, row.name]))))
      .catch(() => { /* names are a convenience; IDs remain readable */ });
    return () => controller.abort();
  }, [workspace, enabled]);
  return names;
}

/** One entry per compiled instance (a stateless Swarm role is one entry named after the role). */
export interface InstanceHealth { name: string; role?: string; host_id?: string; host_name?: string; state: string; healthy?: number; expected?: number }
export interface HealthState { loading: boolean; error: string; placement?: InstanceHealth[] }

const REFRESH_MS = 10_000;

/** Per-instance health of a deployment from `application-health`, refreshed while visible. */
export function useInstanceHealth(workspace: string, id: string, enabled: boolean, reload: unknown): HealthState & { retry: () => void } {
  const [state, setState] = useState<HealthState>({ loading: enabled, error: "" });
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    if (!enabled) { setState({ loading: false, error: "" }); return; }
    let live = true;
    let request: AbortController | undefined;
    const load = async () => {
      if (document.hidden || request) return;
      const controller = new AbortController(); request = controller;
      try {
        const result = await api<{ placement?: InstanceHealth[] }>(`${base(workspace, "application-health")}/${encodeURIComponent(id)}`, { signal: controller.signal });
        if (live) setState({ loading: false, error: "", placement: result.placement ?? [] });
      } catch (error) {
        // Stale states would read as current: a failed refresh reports every instance unknown.
        if (live && !controller.signal.aborted) setState({ loading: false, error: errorText(error) });
      } finally { request = undefined; }
    };
    setState(previous => ({ ...previous, loading: !previous.placement, error: "" }));
    void load();
    const timer = setInterval(() => void load(), REFRESH_MS);
    // A refresh skipped while hidden runs as soon as the page is visible again.
    const visibility = () => { if (!document.hidden) void load(); };
    document.addEventListener("visibilitychange", visibility);
    return () => { live = false; clearInterval(timer); request?.abort(); document.removeEventListener("visibilitychange", visibility); };
  }, [workspace, id, enabled, reload, attempt]);
  return { ...state, retry: () => setAttempt(n => n + 1) };
}
