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
import { useEffect,useSyncExternalStore } from "react";
import { announce,subscribe,snapshot } from "../shared/routing/url-state";
export { useQueryValue,useScrollSync } from "../shared/routing/url-state";
import { resourceName } from "../shared/api/resource";
import { currentLocale,localizedPath,pathLocale,stripLocalePrefix } from "../shared/i18n/locale";
import type { Route } from "../shared/routing/types";
import { nav } from "./nav";
import { commitRoute } from "../shared/api/progress";

export type { Route } from "../shared/routing/types";

const movedRepositoryItems = new Set(["repositories", "code-insights", "repository-events", "credentials", "repository-auth", "api-tokens"]);

export function parse(pathname = location.pathname, search = location.search): Route {
  const locale = pathLocale(pathname);
  const route = parseRoute(stripLocalePrefix(pathname), search);
  return locale ? { ...route, locale } : route;
}

function parseRoute(pathname: string, search: string): Route {
  const publicPage = pathname.replace(/\/$/, "");
  if (["/login", "/register", "/initialize"].includes(publicPage)) return {workspace:`_${publicPage.slice(1)}`,group:"entry",item:publicPage.slice(1),mode:"list"};
  if (pathname.replace(/\/$/, "") === "/workspaces") return { workspace: "_workspaces", group: "workspace", item: "join", mode: "list" };
  const segments = pathname.replace(/^\/+|\/+$/g, "").split("/");
  if (segments[0] === "platform") {
    return parseRoute('/_platform/' + segments.slice(1).join('/'), search);
  }
  // The signed-in user's own profile: a single record, always edited in place.
  if (segments[0] === "account") return { workspace: "_account", group: "account", item: "profile", id: "1", mode: "edit" };
  if (segments[0] === "workspaces") {
    const [, workspace = "", resource = "dashboard", detail = "", action = ""] = segments;
    const scope = new URLSearchParams(search).get("scope");
    if (resource === "credentials" && scope === "ai") return {workspace,group:"ops",item:"ai-models",mode:detail === "new" ? "new" : "list"};
    const workflowKind = new URLSearchParams(search).get("workflow_kind");
    const candidates = nav.flatMap(group => group.items
      .filter(item => resourceName(item.resource) === resource)
      .map(item => ({ group, item })));
    const selected = candidates.find(({item})=>workflowKind && item.params?.workflow_kind===workflowKind)
      ?? candidates.find(({ item }) => scope && item.params?.scope === scope)
      ?? (resource === "credentials" ? candidates.find(({ group, item }) => group.key === "workflow" && item.key === "credentials" && !item.params?.scope) : undefined)
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
    return { workspace, group: "platform", item: ["users", "workspaces", "settings", "server-nodes", "monitor", "marketplace-sources"].includes(group) ? group : "workspaces",
      mode, ...(mode === "view" || mode === "edit" ? { id: item } : {}) };
  }
  if (group === "workflow" && item === "credentials") return {workspace,group:"ops",item:"ai-models",mode:third === "new" ? "new" : "list"};
  const fallback = nav[0]!;
  const groupKey = group === "build" && movedRepositoryItems.has(item) ? "repository" : group === "build" ? "workflow" : group === "infrastructure" ? "host" : group;
  const resolved = nav.find((g) => g.key === groupKey) ?? fallback;
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
  const groupKey = r.group === "build" && movedRepositoryItems.has(r.item) ? "repository" : r.group === "build" ? "workflow" : r.group;
  const item = nav.find(group => group.key === groupKey)?.items.find(item => item.key === r.item);
  for (const [key, value] of Object.entries(item?.params ?? {})) if (!query?.[key]) params.set(key, value);
  if (["_login","_register","_initialize"].includes(r.workspace)) return localizedPath(`/${r.workspace.slice(1)}`, r.locale ?? currentLocale());
  if (r.workspace === "_workspaces") return localizedPath("/workspaces", r.locale ?? currentLocale());
  if (r.workspace === "_account") return localizedPath("/account", r.locale ?? currentLocale());
  const base = r.workspace === "_platform" ? `/platform/${r.item}` :
    `/workspaces/${encodeURIComponent(r.workspace)}/${resourceName(item?.resource ?? r.item)}`;
  const search = params.toString();
  return `${localizedPath(`${base}${tail}`, r.locale ?? currentLocale())}${search ? `?${search}` : ""}`;
}

export const isPublicEntryPath = () => ["/login","/register","/initialize"].includes(stripLocalePrefix(location.pathname).replace(/\/$/, ""));
export const isLoginPath = () => stripLocalePrefix(location.pathname).replace(/\/$/, "") === "/login";

/** Only same-origin relative destinations; never recursively enter login. */
export function safeLoginRedirect(value: string | null): string | undefined {
  if (!value?.startsWith("/") || value.startsWith("//") || /[\\\u0000-\u0020]/.test(value)) return;
  try {
    const url = new URL(value, location.origin);
    if (url.origin !== location.origin || stripLocalePrefix(url.pathname).replace(/\/$/, "") === "/login") return;
    return url.pathname + url.search + url.hash;
  } catch { return; }
}

export function replacePath(target: string) {
  history.replaceState({}, "", target);
  announce();
}

export function navigateToLogin(preserveDestination = true) {
  if (isLoginPath() && preserveDestination) return;
  const destination = preserveDestination ? safeLoginRedirect(location.pathname + location.search + location.hash) : undefined;
  const query = destination ? `?${new URLSearchParams({redirect: destination})}` : "";
  replacePath(localizedPath("/login", currentLocale()) + query);
}

export function navigate(
  route: Partial<Route>,
  query?: Record<string, string>,
  options?: { replace?: boolean },
) {
  window.dispatchEvent(new Event("ferry:before-navigate"));
  const target = href(route, query);
  if (target === snapshot()) return;
  history[options?.replace ? "replaceState" : "pushState"]({}, "", target);
  announce();
}

export function useRoute(): Route {
  const url = useSyncExternalStore(subscribe, snapshot);
  useEffect(() => { commitRoute(url); }, [url]);
  return parse();
}
