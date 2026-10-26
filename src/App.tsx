import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import { Loader2, LogOut, Moon, Palette, Sun } from "lucide-react";
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
import { HostFiles } from "./components/HostFiles";
import { ResourceForm } from "./components/ResourceForm";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { ResourceTable } from "./components/ResourceTable";
import { Platform } from "./Platform";
import { Dashboard } from "./components/Dashboard";
import { Invitation } from "./components/Invitation";
import { Entry } from "./Entry";
import { findGroup, findItem, nav } from "./nav";
import { formConfigs, listConfigs } from "./resources";
import { href, navigate, useQueryValue, useRoute } from "./router";
import { PALETTES, useTheme } from "./theme";

const HostTerminal = lazy(() =>
  import("./components/HostTerminal").then((module) => ({ default: module.HostTerminal })),
);

export function App() {
  const route = useRoute();
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

  const permissions = useMemo(() => new Set(current?.permissions ?? []), [current?.permissions]);
  const lists = useMemo(() => listConfigs(slug, permissions, onSignedOut), [slug, permissions, onSignedOut]);
  const forms = useMemo(() => formConfigs(slug, permissions), [slug, permissions]);

  // Nothing to show yet: send the user through workspace creation/joining.
  if (joining) return <div className="h-full"><Button className="absolute top-4 left-4" variant="outline" onClick={() => setJoining(false)}>返回工作空间</Button><Entry onReady={() => { setJoining(false); onSignedOut(); }} error="" signedIn /></div>;
  if (!current) return <Entry onReady={onSignedOut} error="" signedIn />;

  const group = findGroup(route.group)!;
  const item = findItem(route.group, route.item)!;

  const groups = nav
    .map((g) => ({
      ...g,
      items: g.items.filter((i) => !i.permission || permissions.has(i.permission)),
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

  const listConfig = lists[item.resource];
  const formConfig = forms[item.resource];

  return (
    <div className="grid h-full grid-rows-[48px_1fr]">
      <header className="flex items-center gap-5 border-b border-border bg-card px-4">
        <a className="font-semibold tracking-tight" {...to(nav[0]!.key, nav[0]!.items[0]!.key)}>
          Ferry
        </a>
        <nav className="flex gap-1" aria-label="模块">
          {groups.map((g) => (
            <a
              key={g.key}
              {...to(g.key, g.items[0]!.key)}
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

      <div className="grid min-h-0 grid-cols-[180px_1fr] max-sm:grid-cols-1">
        <nav
          className="flex flex-col gap-0.5 overflow-y-auto border-r border-border bg-card p-3 max-sm:flex-row max-sm:overflow-x-auto max-sm:border-r-0 max-sm:border-b"
          aria-label={group.label}
        >
          {group.items
            .filter((i) => !i.permission || permissions.has(i.permission))
            .map((i) => (
              <a
                key={i.key}
                {...to(group.key, i.key, i.params)}
                aria-current={i.key === item.key ? "page" : undefined}
                className={cn(
                  "rounded-md px-2.5 py-1.5 text-sm whitespace-nowrap text-muted-foreground hover:text-foreground",
                  i.key === item.key && "bg-accent text-foreground",
                )}
              >
                {i.label}
              </a>
            ))}
        </nav>

        {/* auto breadcrumbs / auto title / 1fr content — min-h-0 is what lets
            the content own the scrolling instead of the page. */}
        <main className="grid min-h-0 grid-rows-[auto_auto_1fr] px-4 pt-3">
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
                    {route.mode === "new" ? "创建" : route.mode === "edit" ? "编辑" : "详情"}
                  </li>
                </>
              )}
            </ol>
          </nav>
          <h1 className="mt-1.5 mb-3 text-xl font-semibold">
            {item.label}
            {route.mode === "new" && " · 创建"}
            {route.mode === "edit" && " · 编辑"}
          </h1>

          {item.permission && !permissions.has(item.permission) ? <p role="alert">当前角色没有访问此页面的权限。</p> : route.mode === "list" ? (
            item.resource === "dashboard" ? <Dashboard key={current.slug} workspace={current.slug} /> : listConfig ? (
              <div className="flex min-h-0 flex-col">
              {item.resource === "members" && permissions.has("members.create") && <Invitation key={current.slug} workspace={current.slug} />}
              <ResourceTable
                key={`${current.slug}/${item.resource}/${JSON.stringify(item.params ?? {})}`}
                config={{ ...listConfig, ...(item.params ? { fixed: item.params } : {}) }}
                workspace={current.slug}
                permissions={permissions}
              />
              </div>
            ) : (
              <Placeholder resource={item.resource} />
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
                config={{ ...formConfig, blank: { ...formConfig.blank, ...item.params } }}
                workspace={current.slug}
                permissions={permissions}
              />
            </div>
          ) : (
            <Placeholder resource={item.resource} />
          )}
        </main>
      </div>
    </div>
  );
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
  const panes = [
    { value: "info", label: "信息", enabled: true },
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
        {form && (
          <ResourceForm config={form} workspace={workspace} permissions={permissions} />
        )}
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
        {active === "files" && <HostFiles workspace={workspace} hostId={hostId} />}
      </TabsContent>
    </Tabs>
  );
}

const Placeholder = ({ resource }: { resource: string }) => (
  <p className="text-muted-foreground">
    资源 <code className="rounded bg-muted px-1.5 py-0.5 text-sm">{resource}</code> 还未实现。
    加一份 <code className="rounded bg-muted px-1.5 py-0.5 text-sm">src/resources.tsx</code> 配置
    和对应的服务端处理即可，不需要新的页面组件。
  </p>
);
