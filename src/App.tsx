import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import { FolderTree, Loader2, LogOut, Moon, Palette, Pencil, Plus, Sun, Trash2 } from "lucide-react";
import { Dialog } from "radix-ui";
import { Input } from "@/components/ui/input";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "cn";
import { api, errorText, type Identity } from "./api";
import { DeploymentDetail } from "./components/DeploymentDetail";
import { PipelineRunDetail } from "./components/PipelineRunDetail";
import { RunnerConnection } from "./components/RunnerConnection";
import { HostFiles } from "./components/HostFiles";
import { ResourceForm } from "./components/ResourceForm";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ResourceTable } from "./components/ResourceTable";
import { Platform } from "./Platform";
import { Dashboard } from "./components/Dashboard";
import { Invitation } from "./components/Invitation";
import { Entry } from "./Entry";
import { findGroup, findItem, nav, type NavItem, type NavGroup } from "./nav";
import { formConfigs, listConfigs } from "./resources";
import { href, navigate, useQueryValue, useRoute } from "./router";
import { createResource, deleteResource, listResource, patchResource } from "./resource";
import { PALETTES, useTheme } from "./theme";

const HostTerminal = lazy(() =>
  import("./components/HostTerminal").then((module) => ({ default: module.HostTerminal })),
);

export function App() {
  const route = useRoute();
  useEffect(() => {
    const pipelineGroup = location.pathname.match(/^\/([^/]+)\/build\/(?:pipelines\/groups|pipeline-groups)(?:\/([^/]+))?/);
    const hostGroup = location.pathname.match(/^\/([^/]+)\/host\/host-groups(?:\/([^/]+))?/);
    const legacy = pipelineGroup ?? hostGroup;
    if (legacy) navigate({ workspace: legacy[1]!, group: pipelineGroup ? "build" : "host", item: pipelineGroup ? "pipelines" : "hosts", mode: "list" }, legacy[2] && legacy[2] !== "new" ? { group_id: legacy[2] } : {} , { replace: true });
  }, [route]);
  const [identity, setIdentity] = useState<Identity>();
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [setupRequired, setSetupRequired] = useState(false);
  const [deferredSetup, setDeferredSetup] = useState(false);

  const reload = () => {
    setLoading(true);
    Promise.all([api<Identity>("/me"), api<{ required: boolean }>("/setup", { cache: "no-store" })])
      .then(([value, setup]) => {
        setIdentity(value);
        setSetupRequired(setup.required);
        setError("");
      })
      .catch((cause: unknown) => {
        setIdentity(undefined);
        // 401 is the normal unauthenticated state, not an error to show.
        setError(cause instanceof Error && "status" in cause && cause.status === 401 ? "" : errorText(cause));
      })
      .finally(() => setLoading(false));
  };
  useEffect(reload, []);
  useEffect(() => {
    const query = new URLSearchParams(location.search);
    if (query.get("oauth_popup") === "1") {
      const message = { type: "repository-authorized", provider: query.get("provider"), status: query.get("authorization"), connection_id: query.get("connection_id") };
      if (window.opener) window.opener.postMessage(message, location.origin);
      const key = query.get("oauth_channel");
      if (key && /^[a-zA-Z0-9-]{1,64}$/.test(key)) {
        const channel = new BroadcastChannel(`repository-authorized-${key}`);
        channel.postMessage(message); channel.close();
      }
      window.close();
    }
  }, []);
  useEffect(() => {
    // A session that expires mid-use drops back to the entry screen without
    // changing the URL, so signing in again lands on the same page.
    const onUnauthorized = () => setIdentity(undefined);
    window.addEventListener("ferry:unauthorized", onUnauthorized);
    return () => window.removeEventListener("ferry:unauthorized", onUnauthorized);
  }, []);

  if (loading)
    return (
      <div className="grid h-full place-items-center text-muted-foreground">
        <Loader2 className="size-5 animate-spin" />
      </div>
    );

  // The URL is never replaced while unauthenticated: that is what makes a
  // shared link land on its target page after signing in (§5.3).
  if (!identity?.workspaces) return <Entry onReady={reload} error={error} />;
  if (setupRequired && !deferredSetup)
    return <Entry signedIn initialize onReady={() => { setDeferredSetup(true); reload(); }} error={error} />;
  if (route.workspace === "_platform" || (identity.user.is_superadmin && !route.workspace))
    return <Platform identity={identity} reload={reload} />;
  return <Workspace identity={identity} onSignedOut={reload} />;
}

function Workspace({ identity, onSignedOut }: { identity: Identity; onSignedOut: () => void }) {
  const route = useRoute();
  const theme = useTheme();
  const [joining, setJoining] = useState(false);

  const current =
    identity.workspaces.find((w) => w.slug === route.workspace) ?? identity.workspaces[0];
  const slug = current?.slug ?? "";

  // A URL naming a workspace the account cannot open is corrected once, in
  // place, rather than showing a dead page. Every hook runs before the early
  // return below, so the hook order never depends on the data.
  useEffect(() => {
    if (slug && route.workspace !== slug)
      navigate({ ...route, workspace: slug }, {}, { replace: true });
  }, [route.workspace, slug]);

  const [preferences, setPreferences] = useState<{default_page_size:number;dictionaries:Record<string,{value:string;label:string}[]>}>();
  const [preferencesError, setPreferencesError] = useState("");
  useEffect(() => {
    let live=true; setPreferences(undefined); setPreferencesError("");
    if (slug) void api<{default_page_size:number;dictionaries:Record<string,{value:string;label:string}[]>}>(`/workspaces/${slug}/preferences`).then(value=>{if(live)setPreferences(value);}).catch(()=>{if(live)setPreferencesError("工作空间显示配置加载失败，暂用默认值。");});
    return ()=>{live=false;};
  },[slug,route.item,route.mode]);
  const permissions = useMemo(() => new Set(current?.permissions ?? []), [current?.permissions]);
  const lists = useMemo(() => listConfigs(slug, permissions, onSignedOut, preferences?.dictionaries["host-state"]), [slug, permissions, onSignedOut, preferences]);
  const forms = useMemo(() => formConfigs(slug, permissions), [slug, permissions]);

  // Keep the form configuration stable while background preferences refresh.
  // ResourceForm reloads its draft when its configuration changes.
  const formConfig = useMemo(() => {
    const item = findItem(route.group, route.item);
    const config = item && forms[item.resource];
    return config ? { ...config, blank: { ...config.blank, ...item?.params } } : undefined;
  }, [forms, route.group, route.item]);

  // Nothing to show yet: send the user through workspace creation/joining.
  if (joining) return <div className="h-full"><Button className="absolute top-4 left-4" variant="outline" onClick={() => setJoining(false)}>返回工作空间</Button><Entry onReady={() => { setJoining(false); onSignedOut(); }} error="" signedIn /></div>;
  if (!current) return <Entry onReady={onSignedOut} error="" signedIn />;

  const group = findGroup(route.group)!;
  const item = findItem(route.group, route.item)!;

  const canVisit = (i: NavItem) => !i.permission || permissions.has(i.permission);
  const menuTarget = (g: NavGroup, i: NavItem) => canVisit(i) ? i : g.items.find(child => child.parent === i.key && canVisit(child)) ?? i;
  const menuVisible = (g: NavGroup, i: NavItem) => !i.hidden && (canVisit(i) || g.items.some(child => child.parent === i.key && canVisit(child)));
  const groups = nav
    .map((g) => ({
      ...g,
      items: g.items.filter((i) => menuVisible(g, i)),
    }))
    .filter((g) => g.items.length);

  const to = (groupKey: string, itemKey: string, params?: Record<string, string>) => ({
    href: href({ workspace: current.slug, group: groupKey, item: itemKey, mode: "list" }, params ?? {}),
    onClick: (event: React.MouseEvent) => {
      if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
      event.preventDefault();
      navigate({ workspace: current.slug, group: groupKey, item: itemKey, mode: "list" }, params ?? {});
    },
  });

  const parentItem = item.parent ? findItem(group.key, item.parent)! : item;
  const relatedItems = group.items.filter(i => i.resource !== "pipeline-groups" && i.resource !== "host-groups" && (i.key === parentItem.key || i.parent === parentItem.key) && (!i.permission || permissions.has(i.permission)));
  const listConfig = lists[item.resource];

  if (item.resource === "pipelines" && (route.mode === "new" || route.mode === "edit"))
    return <div className="h-full min-h-0 min-w-0">
      {item.permission && !permissions.has(item.permission) ? <p role="alert">当前角色没有访问此页面的权限。</p> : formConfig ? (
        <ResourceForm key={`${current.slug}/${item.resource}/${route.id ?? "new"}/${route.mode}`} config={formConfig} workspace={current.slug} permissions={permissions} pipelineWorkspace />
      ) : <UnavailablePage onBack={() => navigate({ mode: "list" }, {})} />}
    </div>;

  return (
    <div className="grid h-full min-w-0 grid-cols-1 grid-rows-[48px_1fr]">
      <header className="flex min-w-0 items-center gap-5 overflow-x-auto border-b border-border bg-card px-4">
        <a className="font-semibold tracking-tight" {...to(nav[0]!.key, nav[0]!.items[0]!.key)}>
          Ferry
        </a>
        <nav className="flex gap-1" aria-label="模块">
          {groups.map((g) => (
            <a
              key={g.key}
              {...to(g.key, menuTarget(findGroup(g.key)!, g.items[0]!).key)}
              aria-current={g.key === group.key ? "page" : undefined}
              className={cn(
                "rounded-md px-3 py-1.5 text-sm text-muted-foreground hover:text-foreground",
                g.key === group.key && "bg-accent text-foreground",
              )}
            >
              {g.label}
            </a>
          ))}
        </nav>
        <span className="flex-1" />
        {identity.user.is_superadmin && <Button variant="ghost" onClick={() => navigate({ workspace: "_platform", group: "platform", item: "users", mode: "list" }, {})}>平台管理</Button>}

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="sm" aria-label="主题">
              <Palette />
              {PALETTES.find((p) => p.value === theme.palette)?.label}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {PALETTES.map((palette) => (
              <DropdownMenuItem key={palette.value} onClick={() => theme.set({ palette: palette.value })}>
                {palette.label}
                {palette.value === theme.palette && " ✓"}
              </DropdownMenuItem>
            ))}
          </DropdownMenuContent>
        </DropdownMenu>
        <Button
          variant="ghost"
          size="icon"
          aria-label="深浅模式"
          onClick={() => theme.set({ mode: theme.resolved === "dark" ? "light" : "dark" })}
        >
          {theme.resolved === "dark" ? <Moon /> : <Sun />}
        </Button>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="sm">
              {current.name} · {identity.user.name}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            {identity.workspaces.map((w) => (
              <DropdownMenuItem
                key={w.slug}
                onClick={() =>
                  navigate({ workspace: w.slug, group: route.group, item: route.item, mode: "list" }, {})
                }
              >
                {w.name}
                {w.slug === current.slug && " ✓"}
              </DropdownMenuItem>
            ))}
            <DropdownMenuItem onClick={() => setJoining(true)}>创建/加入工作空间</DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => void api("/sessions", { method: "DELETE" }).then(onSignedOut)}
            >
              <LogOut /> 退出登录
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </header>

      <div className="grid min-h-0 min-w-0 grid-cols-[180px_minmax(0,1fr)] max-sm:grid-cols-1 max-sm:grid-rows-[auto_minmax(0,1fr)]">
        <nav
          className="flex flex-col gap-0.5 overflow-y-auto border-r border-border bg-card p-3 max-sm:flex-row max-sm:overflow-x-auto max-sm:border-r-0 max-sm:border-b"
          aria-label={group.label}
        >
          {group.items
            .filter((i) => menuVisible(group, i))
            .map((i) => (
              <a
                key={i.key}
                {...to(group.key, menuTarget(group, i).key, menuTarget(group, i).params)}
                aria-current={i.key === parentItem.key ? "page" : undefined}
                className={cn(
                  "rounded-md px-2.5 py-1.5 text-sm whitespace-nowrap text-muted-foreground hover:text-foreground",
                  i.key === parentItem.key && "bg-accent text-foreground",
                )}
              >
                {i.label}
              </a>
            ))}
        </nav>

        {/* auto breadcrumbs / auto title / 1fr content — min-h-0 is what lets
            the content own the scrolling instead of the page. */}
        <main className="grid min-h-0 min-w-0 grid-rows-[auto_1fr] px-4 pt-3">
          <div>
          <nav aria-label="面包屑">
            <ol className="flex gap-1.5 text-sm text-muted-foreground">
              <li>
                <a className="hover:text-foreground" {...to(nav[0]!.key, nav[0]!.items[0]!.key)}>
                  {current.name}
                </a>
              </li>
              <li aria-hidden>/</li>
              <li>
                <a className="hover:text-foreground" {...to(group.key, group.items[0]!.key)}>
                  {group.label}
                </a>
              </li>
              <li aria-hidden>/</li>
              {item.parent && <>
                <li><a className="hover:text-foreground" {...to(group.key, parentItem.key, parentItem.params)}>{parentItem.label}</a></li>
                <li aria-hidden>/</li>
              </>}
              <li>
                {route.mode === "list" ? (
                  <span aria-current="page" className="text-foreground">
                    {item.label}
                  </span>
                ) : (
                  <a className="hover:text-foreground" {...to(group.key, item.key, item.params)}>
                    {item.label}
                  </a>
                )}
              </li>
              {route.mode !== "list" && (
                <>
                  <li aria-hidden>/</li>
                  <li aria-current="page" className="text-foreground">
                    {route.mode === "new" ? item.resource === "hosts" ? "添加" : "创建" : route.mode === "edit" ? "编辑" : "详情"}
                  </li>
                </>
              )}
            </ol>
          </nav>
          <h1 className="mt-1.5 mb-3 text-xl font-semibold">
            {item.label}
            {route.mode === "new" && (item.resource === "hosts" ? " · 添加" : " · 创建")}
            {route.mode === "edit" && " · 编辑"}
          </h1>

          {relatedItems.length > 1 && parentItem.key !== "pipelines" && <nav aria-label={`${parentItem.label}页面`} className="mb-3 flex flex-wrap gap-3">
            {relatedItems.map(i => <a key={i.key} {...to(group.key, i.key, i.params)} aria-current={i.key === item.key ? "page" : undefined} className={cn("border-b-2 px-1 py-2 text-sm", i.key === item.key ? "border-primary text-primary" : "border-transparent text-muted-foreground hover:text-foreground")}>{i.label}</a>)}
          </nav>}
          {item.description && <p className="mb-3 max-w-4xl text-sm text-muted-foreground">{item.description}</p>}
          {preferencesError && <p role="alert">{preferencesError}</p>}
          </div>
          {item.permission && !permissions.has(item.permission) ? <p role="alert">当前角色没有访问此页面的权限。</p> : route.mode === "list" ? (
            item.resource === "dashboard" ? <Dashboard key={current.slug} workspace={current.slug} /> : listConfig ? (
              <div className="flex min-h-0 min-w-0 flex-col">
              {item.resource === "members" && permissions.has("members.create") && <Invitation key={current.slug} workspace={current.slug} />}
              {(preferences || preferencesError) ? item.resource === "pipelines" ? <PipelineList
                key={current.slug}
                config={{ ...listConfig, defaultPageSize: preferences?.default_page_size ?? 20 }}
                workspace={current.slug}
                permissions={permissions}
              /> : item.resource === "hosts" ? <HostList
                key={current.slug}
                config={{ ...listConfig, defaultPageSize: preferences?.default_page_size ?? 20 }}
                workspace={current.slug}
                permissions={permissions}
              /> : <ResourceTable
                key={`${current.slug}/${item.resource}/${JSON.stringify(item.params ?? {})}`}
                config={{ ...listConfig, defaultPageSize: preferences?.default_page_size ?? 20, ...(item.params ? { fixed: item.params } : {}) }}
                workspace={current.slug}
                permissions={permissions}
              /> : <p role="status">正在加载工作空间配置…</p>}
              </div>
            ) : (
              <UnavailablePage onBack={() => navigate({ mode: "list" }, {})} />
            )
          ) : route.mode === "view" && (item.resource === "deployments" || item.resource === "deployment-revisions") && route.id ? (
            <DeploymentDetail key={`${current.slug}/${item.resource}/${route.id}`} workspace={current.slug} {...(item.resource === "deployments" ? { deploymentId: route.id } : { revisionId: route.id })} permissions={permissions} />
          ) : route.mode === "view" && item.resource === "pipeline-runs" && route.id ? (
            <PipelineRunDetail key={`${current.slug}/${route.id}`} workspace={current.slug} runId={route.id} permissions={permissions} />
          ) : route.mode === "view" && item.resource === "hosts" && route.id ? (
            // A host's detail page is the form plus the two live panes; the
            // tab is in the URL so a terminal link can be shared directly.
            <HostDetail
              workspace={current.slug}
              hostId={route.id}
              permissions={permissions}
              form={formConfig}
            />
          ) : formConfig ? (
            <div className="min-h-0 overflow-auto">
              <ResourceForm
                key={`${current.slug}/${route.group}/${item.resource}/${route.id ?? "new"}/${route.mode}`}
                {...(item.resource === "roles" || item.resource === "members" ? { onSaved: onSignedOut } : {})}
                config={formConfig}
                workspace={current.slug}
                permissions={permissions}
              />
            </div>
          ) : (
            <UnavailablePage onBack={() => navigate({ mode: "list" }, {})} />
          )}
        </main>
      </div>
    </div>
  );
}

function PipelineList({ config, workspace, permissions }: {
  config: NonNullable<ReturnType<typeof listConfigs>["pipelines"]>;
  workspace: string;
  permissions: Set<string>;
}) {
  const [groupId, setGroupId] = useQueryValue("group_id", "");
  const tableConfig = useMemo(() => ({
    ...config,
    fixed: { ...config.fixed, ...(groupId ? { group_id: groupId } : {}) },
    filters: config.filters?.filter(filter => filter.key !== "group_id") ?? [],
  }), [config, groupId]);
  const select = (id: string) => {
    const query = new URLSearchParams(location.search);
    query.delete("page");
    query.delete("at");
    history.replaceState(history.state, "", `${location.pathname}${query.size ? `?${query}` : ""}`);
    setGroupId(id);
  };
  return <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3 sm:flex-row">
    {permissions.has("pipeline-groups.read") && <GroupSidebar workspace={workspace} permissions={permissions} resource="pipeline-groups" noun="流水线" active={groupId} onSelect={select} />}
    <div className="flex min-h-0 min-w-0 flex-1 flex-col"><ResourceTable key={groupId} config={tableConfig} workspace={workspace} permissions={permissions} /></div>
  </div>;
}

function HostList({ config, workspace, permissions }: {
  config: NonNullable<ReturnType<typeof listConfigs>["hosts"]>;
  workspace: string;
  permissions: Set<string>;
}) {
  const [groupId, setGroupId] = useQueryValue("group_id", "");
  const tableConfig = useMemo(() => ({
    ...config,
    fixed: { ...config.fixed, ...(groupId ? { group_id: groupId } : {}) },
    filters: config.filters?.filter(filter => filter.key !== "group_id") ?? [],
  }), [config, groupId]);
  const select = (id: string) => {
    const query = new URLSearchParams(location.search);
    query.delete("page"); query.delete("at");
    history.replaceState(history.state, "", `${location.pathname}${query.size ? `?${query}` : ""}`);
    setGroupId(id);
  };
  return <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3 sm:flex-row">
    {permissions.has("host-groups.read") && <GroupSidebar workspace={workspace} permissions={permissions} resource="host-groups" noun="主机" active={groupId} onSelect={select} />}
    <div className="flex min-h-0 min-w-0 flex-1 flex-col"><ResourceTable key={groupId} config={tableConfig} workspace={workspace} permissions={permissions} /></div>
  </div>;
}

type GroupNode = { id: string; name: string; description: string; parent_id?: string; version: number };
function GroupSidebar({ workspace, permissions, resource, noun, active, onSelect }: {
  workspace: string; permissions: Set<string>; resource: "host-groups" | "pipeline-groups"; noun: string; active: string; onSelect: (id: string) => void;
}) {
  const [groups, setGroups] = useState<GroupNode[]>([]);
  const [revision, setRevision] = useState(0);
  const [error, setError] = useState("");
  const [editing, setEditing] = useState<{ group?: GroupNode; parentId: string }>();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const canCreate = permissions.has(`${resource}.create`);
  useEffect(() => {
    let live = true;
    const load = async () => {
      const rows: GroupNode[] = [];
      for (let page = 1; ; page++) {
        const result = await listResource<GroupNode>(workspace, resource, { page, page_size: 200 });
        rows.push(...result.items);
        if (page * result.page_size >= result.total) break;
      }
      if (live) { setGroups(rows); setError(""); }
    };
    void load().catch(cause => { if (live) setError(errorText(cause)); });
    return () => { live = false; };
  }, [workspace, resource, revision]);
  const open = (group?: GroupNode, parentId = "") => { setEditing({ ...(group ? { group } : {}), parentId }); setName(group?.name ?? ""); setDescription(group?.description ?? ""); setError(""); };
  const renderGroup = (group: GroupNode, depth: number): React.ReactNode => <div key={group.id}>
    <div className={cn("group flex min-w-0 items-center rounded", active === group.id ? "bg-accent" : "hover:bg-accent")} style={{ paddingLeft: depth * 12 }}>
      <button type="button" aria-current={active === group.id ? "page" : undefined} className="min-w-0 flex-1 truncate px-2 py-1.5 text-left text-sm" title={group.name} onClick={() => onSelect(group.id)}>{group.name}</button>
      {resource === "host-groups" && canCreate && <Button variant="ghost" size="icon" className="size-7 shrink-0" aria-label={`在${group.name}下新建分组`} title="新建子分组" onClick={() => open(undefined, group.id)}><Plus className="size-3.5" /></Button>}
      {permissions.has(`${resource}.update`) && <Button variant="ghost" size="icon" className="size-7 shrink-0" aria-label={`编辑分组 ${group.name}`} title="编辑分组" onClick={() => open(group, group.parent_id ?? "")}><Pencil className="size-3.5" /></Button>}
      {permissions.has(`${resource}.delete`) && <Button variant="ghost" size="icon" className="size-7 shrink-0" aria-label={`删除分组 ${group.name}`} title="删除分组" onClick={async () => {
        if (!confirm(`删除分组“${group.name}”？`)) return;
        try { await deleteResource(workspace, resource, group.id); if (active === group.id) onSelect(""); setRevision(value => value + 1); }
        catch (cause) { setError(errorText(cause)); }
      }}><Trash2 className="size-3.5" /></Button>}
    </div>
    {resource === "host-groups" && groups.filter(child => child.parent_id === group.id).map(child => renderGroup(child, depth + 1))}
  </div>;
  return <aside aria-label={`${noun}分组`} className="flex w-full shrink-0 flex-col gap-1 border-b pb-3 sm:w-56 sm:overflow-y-auto sm:border-r sm:border-b-0 sm:pr-3">
    <div className="mb-1 flex items-center gap-1 px-2 text-sm font-medium"><FolderTree className="size-4" />分组<span className="flex-1" />
      {canCreate && <Button variant="ghost" size="icon" className="size-7" aria-label="新建分组" title="新建分组" onClick={() => open()}><Plus className="size-4" /></Button>}
    </div>
    <button type="button" aria-current={active === "" ? "page" : undefined} className={cn("rounded px-2 py-1.5 text-left text-sm", active === "" ? "bg-accent font-medium" : "hover:bg-accent")} onClick={() => onSelect("")}>全部{noun}</button>
    <button type="button" aria-current={active === "0" ? "page" : undefined} className={cn("rounded px-2 py-1.5 text-left text-sm", active === "0" ? "bg-accent font-medium" : "hover:bg-accent")} onClick={() => onSelect("0")}>未分组</button>
    {groups.filter(group => resource !== "host-groups" || !group.parent_id || group.parent_id === "0" || !groups.some(parent => parent.id === group.parent_id)).map(group => renderGroup(group, 0))}
    {error && !editing && <p role="alert" className="px-2 text-xs text-destructive">{error}</p>}
    <Dialog.Root open={!!editing} onOpenChange={value => { if (!value && !busy) setEditing(undefined); }}><Dialog.Portal><Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" /><Dialog.Content className="fixed left-1/2 top-1/2 z-50 w-[min(28rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-lg border bg-background p-6 shadow-lg">
      <Dialog.Title className="text-lg font-semibold">{editing?.group ? "编辑分组" : "新建分组"}</Dialog.Title>
      <Dialog.Description className="sr-only">设置分组名称和说明</Dialog.Description>
      <form className="mt-4 space-y-4" onSubmit={async event => {
        event.preventDefault(); if (!editing || !name.trim()) return;
        setBusy(true); setError("");
        try {
          const values = { name: name.trim(), description, ...(resource === "host-groups" ? { parent_id: editing.parentId } : {}) };
          if (editing.group) await patchResource(workspace, resource, editing.group.id, { ...values, version: editing.group.version });
          else await createResource(workspace, resource, values);
          setEditing(undefined); setRevision(value => value + 1);
        } catch (cause) { setError(errorText(cause)); }
        finally { setBusy(false); }
      }}>
        <label className="block space-y-2 text-sm">名称<Input value={name} maxLength={60} required onChange={event => setName(event.target.value)} /></label>
        {resource === "host-groups" && <label className="block space-y-2 text-sm">上级分组<select className="w-full rounded-md border border-input bg-background px-3 py-2" value={editing?.parentId ?? ""} onChange={event => setEditing(current => current ? { ...current, parentId: event.target.value } : current)}><option value="">顶级分组</option>{groups.filter(group => group.id !== editing?.group?.id).map(group => <option key={group.id} value={group.id}>{group.name}</option>)}</select></label>}
        <label className="block space-y-2 text-sm">说明<Input value={description} onChange={event => setDescription(event.target.value)} /></label>
        {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
        <div className="flex justify-end gap-2"><Button type="button" variant="outline" onClick={() => setEditing(undefined)}>取消</Button><Button type="submit" disabled={busy || !name.trim()}>{busy ? "保存中…" : "保存"}</Button></div>
      </form>
    </Dialog.Content></Dialog.Portal></Dialog.Root>
  </aside>;
}

function HostDetail({
  workspace,
  hostId,
  permissions,
  form,
}: {
  workspace: string;
  hostId: string;
  permissions: Set<string>;
  form: ReturnType<typeof formConfigs>[string] | undefined;
}) {
  // `?tab=` rather than component state: a shared link should reopen on the
  // terminal if that is what the sender was looking at.
  const [tab, setTab] = useQueryValue("tab", "info");
  const canEnroll = permissions.has("hosts.enroll");
  const [connection, setConnection] = useState<{ workspace: string; hostId: string; kind: string }>();
  const [connectionError, setConnectionError] = useState("");
  useEffect(() => {
    setConnection(undefined); setConnectionError("");
    if (!canEnroll) return;
    const controller = new AbortController();
    void api<{ connection: string }>(`/workspaces/${encodeURIComponent(workspace)}/hosts/${encodeURIComponent(hostId)}`, { signal: controller.signal })
      .then(host => { if (!controller.signal.aborted) setConnection({ workspace, hostId, kind: host.connection }); })
      .catch(cause => { if (!controller.signal.aborted) setConnectionError(errorText(cause)); });
    return () => controller.abort();
  }, [workspace, hostId, canEnroll]);
  const canConnect = canEnroll && connection?.workspace === workspace && connection.hostId === hostId && connection.kind === "runner";

  const panes = [
    { value: "info", label: "信息", enabled: true },
    { value: "connect", label: "接入 Agent", enabled: canConnect },
    { value: "terminal", label: "终端", enabled: permissions.has("hosts.terminal") },
    { value: "files", label: "文件", enabled: permissions.has("hosts.files") },
  ].filter((pane) => pane.enabled);
  const active = panes.some((pane) => pane.value === tab) ? tab : "info";

  return (
    <Tabs value={active} onValueChange={setTab} className="grid min-h-0 grid-rows-[auto_1fr] gap-3">
      <TabsList>
        {panes.map((pane) => (
          <TabsTrigger key={pane.value} value={pane.value}>
            {pane.label}
          </TabsTrigger>
        ))}
      </TabsList>
      <TabsContent value="info" className="min-h-0 overflow-auto">
        {connectionError && <p role="alert" className="mb-3 text-sm text-destructive">无法确认 Agent 接入方式：{connectionError}</p>}
        {form && (
          <ResourceForm config={form} workspace={workspace} permissions={permissions} />
        )}
      </TabsContent>
      <TabsContent value="connect" className="min-h-0 overflow-auto">
        {active === "connect" && canConnect && <div className="max-w-2xl p-1"><RunnerConnection workspace={workspace} hostId={hostId} inline /></div>}
      </TabsContent>
      {/* Mounted only while selected: an unseen tab must not hold a pty open. */}
      <TabsContent value="terminal" className="min-h-0">
        {active === "terminal" && (
          <Suspense fallback={<div role="status">正在加载终端…</div>}>
            <HostTerminal workspace={workspace} hostId={hostId} />
          </Suspense>
        )}
      </TabsContent>
      <TabsContent value="files" className="min-h-0">
        {active === "files" && <HostFiles workspace={workspace} hostId={hostId} canTerminal={permissions.has("hosts.terminal")} />}
      </TabsContent>
    </Tabs>
  );
}

const UnavailablePage = ({ onBack }: { onBack: () => void }) => (
  <div role="alert"><p>此页面不支持当前操作，请返回列表选择可用操作。</p><Button variant="outline" onClick={onBack}>返回列表</Button></div>
);
