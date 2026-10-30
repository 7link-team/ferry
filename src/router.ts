/**
 * Path routing with every piece of view state in the URL, so a pasted link
 * reopens the same page, the same filters, the same page of rows and the same
 * scroll offset after login.
 *
 *   /workspaces/{workspaceId}/{resource}            list
 *   /workspaces/{workspaceId}/{resource}/new        create
 *   /workspaces/{workspaceId}/{resource}/{id}       detail (the form in read-only mode)
 *   /workspaces/{workspaceId}/{resource}/{id}/edit  edit
 *
 * Filters, `page`, `page_size` and `at` (scroll offset) live in the query and
 * are written with replaceState, so typing in a filter never grows history.
 */
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { nav } from "./nav";
import { resourceName } from "./resource";

export interface Route {
  workspace: string;
  group: string;
  item: string;
  id?: string;
  mode: "list" | "new" | "view" | "edit" | "workspace";
}

export function parse(pathname = location.pathname, search = location.search): Route {
  const segments = pathname.replace(/^\/+|\/+$/g, "").split("/");
  if (segments[0] === "platform") {
    return parse('/_platform/' + segments.slice(1).join('/'), search);
  }
  if (segments[0] === "workspaces") {
    const [, workspace = "", resource = "dashboard", detail = "", action = ""] = segments;
    const scope = new URLSearchParams(search).get("scope");
    const candidates = nav.flatMap(group => group.items
      .filter(item => resourceName(item.resource) === resource)
      .map(item => ({ group, item })));
    const selected = candidates.find(({ item }) => scope && item.params?.scope === scope)
      ?? candidates.find(({ item }) => !item.hidden) ?? candidates[0];
    const group = selected?.group ?? nav[0]!;
    const item = selected?.item ?? group.items[0]!;
    const mode = detail === "new" ? "new" : detail ? (action === "workspace" ? "workspace" : action === "edit" ? "edit" : "view") : "list";
    return { workspace, group: group.key, item: item.key, mode,
      ...(mode === "view" || mode === "edit" || mode === "workspace" ? { id: detail } : {}) };
  }
  const [workspace = "", group = "", item = "", third = "", fourth = "", fifth = ""] = pathname
    .replace(/^\/+|\/+$/g, "")
    .split("/");
  if (workspace === "_platform") {
    const mode = item === "new" ? "new" : item ? (third === "edit" ? "edit" : "view") : "list";
    return { workspace, group: "platform", item: ["users", "workspaces", "settings"].includes(group) ? group : "users",
      mode, ...(mode === "view" || mode === "edit" ? { id: item } : {}) };
  }
  const fallback = nav[0]!;
  const resolved = nav.find((g) => g.key === (group === "infrastructure" ? "host" : group)) ?? fallback;
  const legacyRepository = ["repository-auth", "api-tokens"].includes(item);
  const pipelineGroups = group === "build" && item === "pipelines" && third === "groups";
  const resolvedItem = pipelineGroups ? "pipeline-groups" : legacyRepository ? "repositories" :
    resolved.key === "host" && item === "nodes" ? "hosts" :
    resolved.items.find((i) => i.key === item)?.key ?? resolved.items[0]!.key;
  const detail = pipelineGroups ? fourth : third;
  const action = pipelineGroups ? fifth : fourth;
  const mode = legacyRepository ? "list" :
    detail === "new" ? "new" : detail ? (resolved.key === "host" && resolvedItem === "hosts" && action === "workspace" ? "workspace" : action === "edit" ? "edit" : "view") : "list";
  return {
    workspace,
    group: resolved.key,
    item: resolvedItem,
    ...(mode === "view" || mode === "edit" || mode === "workspace" ? { id: detail } : {}),
    mode,
  };
}

export function href(route: Partial<Route>, query?: Record<string, string>) {
  const current = parse();
  const r = { ...current, ...route };
  const tail =
    r.mode === "new" ? "/new" : r.mode === "view" ? `/${r.id}` : r.mode === "edit" ? `/${r.id}/edit` : r.mode === "workspace" ? `/${r.id}/workspace` : "";
  const params = new URLSearchParams(query ? "" : location.search);
  for (const [key, value] of Object.entries(query ?? {})) if (value) params.set(key, value);
  const item = nav.find(group => group.key === r.group)?.items.find(item => item.key === r.item);
  for (const [key, value] of Object.entries(item?.params ?? {})) if (!query?.[key]) params.set(key, value);
  const base = r.workspace === "_platform" ? `/platform/${r.item}` :
    `/workspaces/${encodeURIComponent(r.workspace)}/${resourceName(item?.resource ?? r.item)}`;
  const search = params.toString();
  return `${base}${tail}${search ? `?${search}` : ""}`;
}

const listeners = new Set<() => void>();
const announce = () => listeners.forEach((fn) => fn());
const subscribe = (fn: () => void) => {
  listeners.add(fn);
  window.addEventListener("popstate", fn);
  return () => {
    listeners.delete(fn);
    window.removeEventListener("popstate", fn);
  };
};
const snapshot = () => location.pathname + location.search;

export function navigate(
  route: Partial<Route>,
  query?: Record<string, string>,
  options?: { replace?: boolean },
) {
  const target = href(route, query);
  if (target === snapshot()) return;
  history[options?.replace ? "replaceState" : "pushState"]({}, "", target);
  announce();
}

export function useRoute(): Route {
  useSyncExternalStore(subscribe, snapshot);
  return parse();
}

/** `useQueryValue("page", 1)` must give back a `number`, not the literal `1`. */
type Widen<T> = T extends number ? number : T extends string ? string : T;

/** A filter or pagination value that lives in the URL without adding history. */
export function useQueryValue<T extends string | number>(
  key: string,
  fallback: T,
): readonly [Widen<T>, (value: Widen<T>) => void] {
  const read = useCallback((): T => {
    const raw = new URLSearchParams(location.search).get(key);
    if (raw === null) return fallback;
    if (typeof fallback === "number")
      return (/^\d+$/.test(raw) && Number(raw) > 0 ? Number(raw) : fallback) as T;
    return raw as T;
  }, [key, fallback]);
  const [value, setValue] = useState<Widen<T>>(read as () => Widen<T>);
  useEffect(() => {
    const params = new URLSearchParams(location.search);
    if (value === fallback) params.delete(key);
    else params.set(key, String(value));
    const search = params.toString();
    const next = `${location.pathname}${search ? `?${search}` : ""}`;
    if (next !== snapshot()) history.replaceState(history.state, "", next);
  }, [key, value, fallback]);
  useSyncExternalStore(subscribe, snapshot);
  useEffect(() => setValue(read() as Widen<T>), [read]);
  return [value, setValue] as const;
}

/**
 * Keeps a scroll container's offset in `?at=` so a shared link lands where the
 * sender was looking. Restoration waits for `ready` — the rows have to exist
 * before the offset means anything.
 *
 * ponytail: pixels, not a row key. Pixels are wrong if the viewer's page size
 * or font differs; switch to the first visible row's id if that shows up.
 */
export function useScrollSync(ready: boolean) {
  const ref = useRef<HTMLDivElement>(null);
  const restored = useRef(false);
  useEffect(() => {
    const node = ref.current;
    if (!node) return;
    let timer = 0;
    const onScroll = () => {
      clearTimeout(timer);
      timer = window.setTimeout(() => {
        const params = new URLSearchParams(location.search);
        if (node.scrollTop > 8) params.set("at", String(Math.round(node.scrollTop)));
        else params.delete("at");
        const search = params.toString();
        history.replaceState(history.state, "", `${location.pathname}${search ? `?${search}` : ""}`);
      }, 150);
    };
    node.addEventListener("scroll", onScroll, { passive: true });
    return () => {
      clearTimeout(timer);
      node.removeEventListener("scroll", onScroll);
    };
  }, []);
  useEffect(() => {
    const node = ref.current;
    if (!node || !ready || restored.current) return;
    restored.current = true;
    const at = Number(new URLSearchParams(location.search).get("at") ?? 0);
    if (at > 0) node.scrollTo({ top: at });
  }, [ready]);
  return ref;
}
