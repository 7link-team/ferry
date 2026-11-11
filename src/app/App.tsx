import { installIntegrationCatalog, type IntegrationManifest } from "../features/workflows/integrations";
import { TypedValueList } from "./pages/TypedValueList";
import { ConversationsWorkspace } from "./pages/ConversationsWorkspace";
import { cn } from "cn";
import { BrandIcon } from "../shared/site/BrandIcon";
import { MobileNavigation } from "./MobileNavigation";
import { ChevronDown, ChevronRight, LogOut } from "lucide-react";
import { LoadingSkeleton } from "../shared/ui/loading-skeleton";
import { useEffect,useMemo,useState } from "react";
import { useTranslation } from "react-i18next";
import { Dashboard } from "../features/dashboard/Dashboard";
import { Invitation } from "../features/workspaces/Invitation";
import { api,errorText,type Identity } from "../shared/api/api";
import { commonText,workflowText,useCommonTranslation } from "../shared/i18n/i18n";
import { LanguageSwitcher } from "../shared/i18n/LanguageSwitcher";
import { stripLocalePrefix } from "../shared/i18n/locale";
import { ThemeSwitcher } from "../shared/theme/ThemeSwitcher";
import { useAccountPreferences } from "../shared/theme/useAccountPreferences";
import { Button } from "../shared/ui/button";
import {
DropdownMenu,
DropdownMenuContent,
DropdownMenuItem,
DropdownMenuRadioGroup,
DropdownMenuRadioItem,
DropdownMenuSeparator,
DropdownMenuTrigger,
} from "../shared/ui/dropdown-menu";
import { Account,accountRoute } from "./Account";
import { Entry } from "./Entry";
import { findGroup,findItem,nav,type NavGroup,type NavItem } from "./nav";
import { CodeInsightDetail } from "./pages/CodeInsight";
import { DeploymentDetail } from "./pages/DeploymentDetail";
import { HostDetail } from "./pages/HostDetailPage";
import { HostList } from "./pages/HostListPage";
import { HostWorkspace } from "./pages/HostWorkspace";
import { PipelineDetail,PipelineList } from "./pages/PipelinePages";
import { PipelineRunDetail } from "./pages/PipelineRunDetail";
import { RepositoryConnectionDetail } from "./pages/RepositoryConnectionPage";
import { Platform,myRole } from "./Platform";
import { formConfigs,listConfigs } from "./resources";
import { ResourceForm } from "./resources/ResourceForm";
import { ResourceTable } from "./resources/ResourceTable";
import { href,isLoginPath,isPublicEntryPath,navigate,navigateToLogin,replacePath,safeLoginRedirect,useRoute } from "./router";
import { useDocumentTitle } from "./document-title";
import { useSiteSettings } from "../shared/site/settings";

export function App() {
  useCommonTranslation();
  const route = useRoute();
  useDocumentTitle(route);
  const [identity, setIdentity] = useState<Identity>();
  useAccountPreferences(identity?.user.id);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [setupRequired, setSetupRequired] = useState(false);
  const [deferredSetup, setDeferredSetup] = useState(false);
  useEffect(() => {
    if (!identity?.user.id || isLoginPath()) return;
    void api("/monitor/pageview", {method:"POST", body:JSON.stringify({path:location.pathname})}).catch(() => {});
  }, [identity?.user.id, route.workspace, route.group, route.item, route.mode, route.id]);
  useEffect(() => {
    if (!identity) return;
    if (isPublicEntryPath() && !(route.workspace === "_initialize" && setupRequired && !deferredSetup)) {
      const target = safeLoginRedirect(new URLSearchParams(location.search).get("redirect"));
      replacePath(target ?? href(identity.workspaces[0] ? {workspace:identity.workspaces[0].id,group:"host",item:"hosts",mode:"list"} : {workspace:identity.user.is_superadmin?"_platform":"_workspaces",group:"platform",item:"workspaces",mode:"list"}, {}));
      return;
    }
    const current = identity.workspaces.find(workspace => workspace.id === route.workspace)
      ?? identity.workspaces.find(workspace => workspace.slug === route.workspace);
    if (!current && route.workspace !== "_platform") return;
    const query = Object.fromEntries(new URLSearchParams(location.search));
    if (route.item === "ai-models" && (query.scope === "ai" || /\/workflow\/credentials(?:\/|$)/.test(stripLocalePrefix(location.pathname)))) {
      delete query.scope;
      navigate({ ...route, workspace: current?.id ?? route.workspace }, query, { replace: true });
    } else if ((route.item === "host-groups" || route.item === "pipeline-groups") && (route.mode === "list" || route.mode === "view")) {
      if (route.id) query.group_id = route.id;
      navigate({ ...route, workspace: current?.id ?? route.workspace, item: route.item === "host-groups" ? "hosts" : "pipelines", mode: "list" }, query, { replace: true });
    } else if (!stripLocalePrefix(location.pathname).startsWith("/platform/") && !stripLocalePrefix(location.pathname).startsWith(`/workspaces/${current?.id}/`)) {
      navigate({ ...route, workspace: current?.id ?? route.workspace }, query, { replace: true });
    }
  }, [identity, route, setupRequired, deferredSetup]);

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
    if (!loading && !identity && !error && !isPublicEntryPath()) navigateToLogin();
  }, [loading, identity, error, route]);
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
    const onUnauthorized = () => { setIdentity(undefined); navigateToLogin(); };
    window.addEventListener("ferry:unauthorized", onUnauthorized);
    return () => window.removeEventListener("ferry:unauthorized", onUnauthorized);
  }, []);

  if (loading || (identity && isLoginPath()))
    return (
      <div className="h-full overflow-auto">
        <AppearanceActions floating />
        <LoadingSkeleton variant={isLoginPath() ? "form" : "page"} className={isLoginPath() ? "mx-auto max-w-md pt-24" : "pt-10"} />
      </div>
    );

  const signedOut = () => { setIdentity(undefined); navigateToLogin(false); reload(); };
  if (!identity?.workspaces) return <><AppearanceActions floating /><Entry onReady={reload} error={error} /></>;
  if (setupRequired && !deferredSetup)
    return <><AppearanceActions floating /><Entry signedIn initialize onReady={() => { setDeferredSetup(true); reload(); }} error={error} /></>;
  if (route.workspace === "_workspaces") return <div className="h-full overflow-auto"><AppearanceActions floating />{(identity.workspaces.length > 0 || identity.user.is_superadmin) && <Button className="absolute top-4 left-4" variant="outline" onClick={() => navigate(identity.workspaces.length ? {workspace:identity.workspaces[0]!.id,group:"host",item:"hosts",mode:"list"} : {workspace:"_platform",group:"platform",item:"workspaces",mode:"list"}, {})}>{commonText("shell.backWorkspace")}</Button>}<Entry signedIn onReady={reload} error={error} /></div>;
  if (route.workspace === "_account") return <Account identity={identity} reload={reload} headerActions={<AppearanceActions />} />;
  if (route.workspace === "_platform" || (identity.user.is_superadmin && !route.workspace))
    return <Platform identity={identity} reload={reload} onSignedOut={signedOut} headerActions={<WorkspaceHeaderActions identity={identity} onWorkspaces={() => navigate({workspace:"_workspaces"}, {})} onSignedOut={signedOut} />} />;
  if (route.group === "host" && route.item === "hosts" && route.mode === "workspace" && route.id) {
    const current = identity.workspaces.find(workspace => workspace.id === route.workspace)
      ?? identity.workspaces.find(workspace => workspace.slug === route.workspace);
    if (current) return <HostWorkspace key={`${route.workspace}/${route.id}`} workspace={current.id} hostId={route.id} permissions={new Set(current.permissions)} headerActions={<AppearanceActions />} />;
  }
  return <Workspace identity={identity} reload={reload} onSignedOut={signedOut} />;
}

function Workspace({ identity, reload, onSignedOut }: { identity: Identity; reload: () => void; onSignedOut: () => void }) {
  useCommonTranslation();
  const { t } = useTranslation();
  const site = useSiteSettings();
  const route = useRoute();

  const current =
    identity.workspaces.find((w) => w.id === route.workspace)
      ?? identity.workspaces.find((w) => w.slug === route.workspace) ?? (!route.workspace ? identity.workspaces[0] : undefined);
  const workspaceId = current?.id ?? "";
  const needsNodeCatalog = route.item === "workflows" || route.item === "workflow-templates" || route.item === "integration-credentials" || (route.group === "workflow" && route.item === "credentials");
  const [nodeCatalogWorkspace, setNodeCatalogWorkspace] = useState("");
  const [nodeCatalogError, setNodeCatalogError] = useState("");
  const [nodeCatalogRetry, setNodeCatalogRetry] = useState(0);
  useEffect(() => {
    if (!workspaceId || !needsNodeCatalog || nodeCatalogWorkspace === workspaceId) return;
    const controller = new AbortController();
    setNodeCatalogError("");
    void api<{ nodes: IntegrationManifest[] }>(`/workspaces/${workspaceId}/workflow-node-catalog`, { signal: controller.signal })
      .then(value => {
        if (controller.signal.aborted) return;
        installIntegrationCatalog(value.nodes);
        setNodeCatalogWorkspace(workspaceId);
      }).catch(cause => { if (!controller.signal.aborted) setNodeCatalogError(errorText(cause)); });
    return () => controller.abort();
  }, [workspaceId, needsNodeCatalog, nodeCatalogWorkspace, nodeCatalogRetry]);

  // A URL naming a workspace the account cannot open is corrected once, in
  // place, rather than showing a dead page. Every hook runs before the early
  // return below, so the hook order never depends on the data.
  useEffect(() => {
    if (workspaceId && route.workspace !== workspaceId)
      navigate({ ...route, workspace: workspaceId }, Object.fromEntries(new URLSearchParams(location.search)), { replace: true });
  }, [route.workspace, workspaceId]);

  const [loadedPreferences, setPreferences] = useState<{workspace:string;value:{default_page_size:number;dictionaries:Record<string,{value:string;label:string;color?:string}[]>}}>();
  const preferences = loadedPreferences?.workspace === workspaceId ? loadedPreferences.value : undefined;
  const [preferencesError, setPreferencesError] = useState("");
  // Configuration edits/imports/deletions refresh preferences on returning to a list
  // or leaving maintenance. Ordinary navigation reuses this workspace's data.
  const preferencesPage = ["settings", "dictionaries"].includes(route.item) ? `${route.item}/${route.mode}` : "";
  useEffect(() => {
    let live=true; setPreferencesError("");
    const controller = new AbortController();
    if (workspaceId) void api<{default_page_size:number;dictionaries:Record<string,{value:string;label:string;color?:string}[]>}>(`/workspaces/${workspaceId}/preferences`, { signal: controller.signal }).then(value=>{if(live)setPreferences({workspace:workspaceId,value});}).catch(()=>{if(live)setPreferencesError(commonText("shell.preferencesFailed"));});
    return ()=>{live=false;controller.abort();};
  },[workspaceId,preferencesPage]);
  const permissions = useMemo(() => new Set(current?.permissions ?? []), [current?.permissions]);
  const lists = useMemo(() => listConfigs(workspaceId, permissions, reload, undefined, preferences?.dictionaries), [workspaceId, permissions, reload, preferences]);
  const forms = useMemo(() => formConfigs(workspaceId, permissions), [workspaceId, permissions, nodeCatalogWorkspace]);
  const [collapsedSections, setCollapsedSections] = useState<Record<string, boolean>>({});
  useEffect(() => {
    const section = findItem(route.group, route.item)?.section;
    if (section) setCollapsedSections(previous => previous[section] ? { ...previous, [section]: false } : previous);
  }, [route.group, route.item, route.workspace]);

  // Keep the form configuration stable while background preferences refresh.
  // ResourceForm reloads its draft when its configuration changes.
  const formConfig = useMemo(() => {
    const item = findItem(route.group, route.item);
    const config = item && (forms[item.key] ?? forms[item.resource]);
    return config ? { ...config, blank: { ...config.blank, ...(route.group === "workflow" && item?.key === "credentials" ? {scope:"integration",kind:"token"} : {}), ...item?.params } } : undefined;
  }, [forms, route.group, route.item]);

  // Nothing to show yet: send the user through workspace creation/joining.
  if (!current) return <><AppearanceActions floating />{identity.workspaces.length ? <UnavailablePage onBack={()=>navigate({workspace:identity.workspaces[0]!.id,group:"host",item:"hosts",mode:"list"}, {})} /> : <Entry onReady={reload} error="" signedIn />}</>;

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
    href: href({ workspace: current.id, group: groupKey, item: itemKey, mode: "list" }, params ?? {}),
    onClick: (event: React.MouseEvent) => {
      if (event.button !== 0 || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
      event.preventDefault();
      navigate({ workspace: current.id, group: groupKey, item: itemKey, mode: "list" }, params ?? {});
    },
  });

  const parentItem = item.parent ? findItem(group.key, item.parent)! : item;
  const visibleMenu = group.items.filter(i => menuVisible(group, i));
  const menuSections = [...new Set(visibleMenu.map(i => i.section ?? ""))].map(section => ({ section, items: visibleMenu.filter(i => (i.section ?? "") === section) }));
  const notificationArea = item.resource.startsWith("notification-");
  const relatedItems = group.items.filter(i => i.resource !== "pipeline-groups" && i.resource !== "host-groups" && i.resource !== "holes" && (notificationArea ? (i.key === (item.resource === "notification-channels" ? item.key : "notification-channels") || ["notification-templates", "notification-deliveries"].includes(i.key)) : (i.key === parentItem.key || i.parent === parentItem.key)) && (!i.permission || permissions.has(i.permission)));
  const listConfig = (lists[item.key] ?? lists[item.resource]);
  const nodeCatalogPending = needsNodeCatalog && nodeCatalogWorkspace !== workspaceId;
  const nodeCatalogFeedback = nodeCatalogError
    ? <div className="space-y-3 p-4"><p role="alert">{nodeCatalogError}</p><Button variant="outline" onClick={() => setNodeCatalogRetry(value => value + 1)}>{commonText("common.retry")}</Button></div>
    : <LoadingSkeleton variant="editor" />;

  if ((formConfig?.editor && route.mode !== "list") || (item.resource === "pipelines" && (route.mode === "new" || route.mode === "edit")) ||
      (item.resource === "pipeline-templates" && ["new", "edit", "view"].includes(route.mode)))
    return <div className="h-full min-h-0 min-w-0">
      {item.permission && !permissions.has(item.permission) ? <><AppearanceActions floating /><p role="alert">{commonText("shell.forbidden")}</p></> : nodeCatalogPending ? <><AppearanceActions floating />{nodeCatalogFeedback}</> : formConfig ? (
        <ResourceForm key={`${current.id}/${item.resource}/${route.id ?? "new"}/${route.mode}`} config={formConfig} workspace={current.id} permissions={permissions} pipelineWorkspace headerActions={<AppearanceActions />} />
      ) : <><AppearanceActions floating /><UnavailablePage onBack={() => navigate({ mode: "list" }, {})} /></>}
    </div>;

  return (
    <div className="grid h-full min-h-0 min-w-0 grid-cols-1 grid-rows-[auto_minmax(0,1fr)_auto] md:grid-rows-[auto_minmax(0,1fr)]">
      <header className="flex min-h-12 min-w-0 flex-wrap items-center gap-x-5 gap-y-1 border-b border-border bg-background px-4 py-1">
        <a className="flex min-w-0 max-w-full items-center gap-2 font-semibold tracking-tight sm:max-w-xs" {...to(nav[0]!.key, nav[0]!.items[0]!.key)}>
          <BrandIcon /><span className="min-w-0 break-words [overflow-wrap:anywhere]">{site.site_name || t("app.title")}</span>
        </a>
        <nav className="hidden min-w-0 max-w-full gap-1 overflow-x-auto md:flex" aria-label={t("app.modules")}>
          {groups.map((g) => (
            <a
              key={g.key}
              {...to(g.key, menuTarget(findGroup(g.key)!, g.items[0]!).key)}
              aria-current={g.key === group.key ? "page" : undefined}
              className={cn(
                "rounded-md px-3 py-1.5 text-sm font-medium transition-colors",
                g.key === group.key ? "sidebar-active-item bg-sidebar-primary font-semibold text-sidebar-primary-foreground" : "text-foreground hover:bg-accent hover:text-accent-foreground",
              )}
            >
              {g.label}
            </a>
          ))}
        </nav>
        <span className="flex-1" />
        <div className="flex min-w-0 max-w-full items-center gap-1">
          <WorkspaceSwitcher identity={identity} current={current} onWorkspaces={() => navigate({workspace:"_workspaces"}, {})} />
          {identity.user.is_superadmin && <Button variant="ghost" onClick={() => navigate({ workspace: "_platform", group: "platform", item: "workspaces", mode: "list" }, {})}>{t("app.platform")}</Button>}
        </div>
        <WorkspaceHeaderActions identity={identity} onSignedOut={onSignedOut} />
      </header>

      {item.resource === "conversations" ? <ConversationsWorkspace key={current.id} workspace={current.id} permissions={permissions} form={formConfig} /> : <div className="grid min-h-0 min-w-0 grid-cols-1 md:grid-cols-[180px_minmax(0,1fr)]">
        <nav
          className="hidden flex-col gap-0.5 overflow-y-auto border-r border-sidebar-border bg-sidebar text-sidebar-foreground p-3 md:flex"
          aria-label={group.label}
        >
          {menuSections.map(({ section, items }) => <div key={section} role={section ? "group" : undefined} aria-label={section ? commonText(section) : undefined} className={cn("flex flex-col gap-1", section && "mb-3 shrink-0 last:mb-0 max-sm:mr-4 max-sm:mb-0")}>
            {section && <button type="button" aria-expanded={!collapsedSections[section]} aria-controls={`menu-${section}`} onClick={() => setCollapsedSections(previous => ({ ...previous, [section]: !previous[section] }))} className="flex min-h-11 w-full items-center justify-between gap-2 rounded-md px-2.5 py-2 text-left text-sm font-semibold text-sidebar-foreground hover:bg-sidebar-accent focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-sidebar-ring max-sm:min-w-32">{commonText(section)}{collapsedSections[section] ? <ChevronRight className="size-4 shrink-0" /> : <ChevronDown className="size-4 shrink-0" />}</button>}
            <div id={section ? `menu-${section}` : undefined} hidden={!!section && !!collapsedSections[section]} className={cn("flex flex-col gap-0.5 max-sm:flex-row", section && "pl-2 max-sm:pl-0", section && collapsedSections[section] && "hidden")}>
            {items.map((i) => (
              <a
                key={i.key}
                {...to(group.key, menuTarget(group, i).key, menuTarget(group, i).params)}
                aria-current={i.key === parentItem.key ? "page" : undefined}
                className={cn(
                  "rounded-md px-2.5 py-1.5 text-sm whitespace-nowrap transition-colors",
                  section && "max-sm:inline-flex max-sm:min-h-11 max-sm:items-center",
                  i.key === parentItem.key ? "sidebar-active-item bg-sidebar-primary font-medium text-sidebar-primary-foreground" : "text-sidebar-foreground/90 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                )}
              >
                {i.label}
              </a>
            ))}
            </div>
          </div>)}
        </nav>

        {/* auto breadcrumbs / auto title / 1fr content — min-h-0 is what lets
            the content own the scrolling instead of the page. */}
        <main className="grid min-h-0 min-w-0 grid-rows-[auto_1fr] px-4 pt-3">
          <div>
          {route.mode === "list" && visibleMenu.length > 1 && <div className="mb-2 md:hidden"><DropdownMenu key={route.item}><DropdownMenuTrigger asChild><Button variant="outline" className="min-h-11 w-full justify-between"><span className="min-w-0 truncate">{group.label} · {parentItem.label}</span><ChevronDown className="size-4 shrink-0" /></Button></DropdownMenuTrigger><DropdownMenuContent align="start" collisionPadding={8} className="max-h-[60dvh] w-[min(24rem,calc(100vw-2rem))] overflow-y-auto">{visibleMenu.map(entry => <DropdownMenuItem asChild key={entry.key}><a {...to(group.key, menuTarget(group, entry).key, menuTarget(group, entry).params)} aria-current={entry.key === parentItem.key ? "page" : undefined} className="min-h-11 whitespace-normal break-words">{entry.section ? `${commonText(entry.section)} · ` : ""}{entry.label}</a></DropdownMenuItem>)}</DropdownMenuContent></DropdownMenu></div>}
          <nav aria-label={commonText("shell.breadcrumbs")}>
            <ol className="flex min-w-0 flex-wrap gap-1.5 text-sm break-words text-muted-foreground">
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
                    {route.mode === "new" ? item.resource === "hosts" ? commonText("common.add") : commonText("common.create") : route.mode === "edit" ? commonText("common.edit") : commonText("common.details")}
                  </li>
                </>
              )}
            </ol>
          </nav>
          <h1 className="mt-1.5 mb-3 text-xl font-semibold">
            {item.label}
            {route.mode === "new" && (item.resource === "hosts" ? commonText("shell.addSuffix") : commonText("shell.createSuffix"))}
            {route.mode === "edit" && commonText("shell.editSuffix")}
          </h1>

          {relatedItems.length > 1 && !["pipelines", "workflows", "credentials"].includes(parentItem.key) && <nav aria-label={commonText("shell.relatedPages", { name: notificationArea ? commonText("notifications:notification-channels") : parentItem.label })} className="mb-3 flex flex-wrap gap-3">
            {relatedItems.map(i => <a key={i.key} {...to(group.key, i.key, i.params)} aria-current={i.key === item.key ? "page" : undefined} className={cn("border-b-2 px-1 py-2 text-sm", i.key === item.key ? "border-primary text-primary" : "border-transparent text-muted-foreground hover:text-foreground")}>{i.label}</a>)}
          </nav>}
          {item.description && <p className="mb-3 max-w-4xl text-sm text-muted-foreground">{item.description}</p>}
          {route.mode === "list" && group.key === "workflow" && ["pipelines","workflows"].includes(parentItem.key) && <nav aria-label={workflowText("libraryTabs")} className="mb-3 flex gap-3">{[findItem("workflow",parentItem.key),findItem("workflow",parentItem.key==="workflows"?"workflow-templates":"pipeline-templates")].filter((entry):entry is NavItem=>!!entry&&canVisit(entry)).map(entry=><a key={entry.key} {...to("workflow",entry.key,entry.params)} aria-current={entry.key===item.key?"page":undefined} className={cn("border-b-2 px-1 py-2 text-sm",entry.key===item.key?"border-primary text-primary":"border-transparent text-muted-foreground hover:text-foreground")}>{entry.key===parentItem.key?workflowText("workflows"):workflowText("templates")}</a>)}</nav>}
          {preferencesError && <p role="alert">{preferencesError}</p>}
          </div>
          {item.permission && !permissions.has(item.permission) ? <p role="alert">{commonText("shell.forbidden")}</p> : nodeCatalogPending ? nodeCatalogFeedback : route.mode === "list" ? (
            item.resource === "dashboard" ? <Dashboard key={current.id} workspace={current.id} /> : listConfig ? (
              <div className="flex min-h-0 min-w-0 flex-col">
              {item.resource === "members" && permissions.has("members.create") && <Invitation key={current.id} workspace={current.id} />}
              {(preferences || preferencesError) ? item.resource === "pipelines" ? <PipelineList
                key={current.id}
                config={{ ...listConfig, defaultPageSize: preferences?.default_page_size ?? 20 }}
                workspace={current.id}
                permissions={permissions}
              /> : item.resource === "hosts" ? <HostList
                key={current.id}
                config={{ ...listConfig, defaultPageSize: preferences?.default_page_size ?? 20 }}
                workspace={current.id}
                permissions={permissions}
              /> : ["settings", "dictionaries"].includes(item.resource) ? <TypedValueList key={`${current.id}/${item.resource}`} config={{ ...listConfig, defaultPageSize: preferences?.default_page_size ?? 20 }} workspace={current.id} permissions={permissions} /> : <ResourceTable
                key={`${current.id}/${item.resource}/${JSON.stringify(item.params ?? {})}`}
                config={{ ...listConfig, defaultPageSize: preferences?.default_page_size ?? 20, ...(notificationArea ? { fixed: listConfig.fixed } : item.params ? { fixed: item.params } : {}) }}
                workspace={current.id}
                permissions={permissions}
              /> : <LoadingSkeleton label={commonText("shell.loadingPreferences")} />}
              </div>
            ) : (
              <UnavailablePage onBack={() => navigate({ mode: "list" }, {})} />
            )
          ) : (route.mode === "view" || route.mode === "edit") && item.resource === "code-insights" && route.id ? (
            <CodeInsightDetail key={`${current.id}/${route.id}`} workspace={current.id} insightId={route.id} permissions={permissions} />
          ) : (route.mode === "view" || (route.mode === "edit" && new URLSearchParams(location.search).get("tab") === "monitoring")) && item.resource === "code-connections" && route.id && formConfig ? (
            <RepositoryConnectionDetail key={`${current.id}/${route.id}`} workspace={current.id} connectionId={route.id} permissions={permissions} form={formConfig} />
          ) : route.mode === "view" && (item.resource === "deployments" || item.resource === "deployment-revisions") && route.id ? (
            <DeploymentDetail key={`${current.id}/${item.resource}/${route.id}`} workspace={current.id} {...(item.resource === "deployments" ? { deploymentId: route.id } : { revisionId: route.id })} permissions={permissions} />
          ) : route.mode === "view" && item.resource === "pipeline-runs" && route.id ? (
            <PipelineRunDetail key={`${current.id}/${route.id}`} workspace={current.id} runId={route.id} permissions={permissions} />
          ) : route.mode === "view" && item.resource === "pipelines" && route.id && formConfig && lists["pipeline-runs"] ? (
            <PipelineDetail key={`${current.id}/${route.id}`} workspace={current.id} pipelineId={route.id} permissions={permissions} form={formConfig} runs={lists["pipeline-runs"]} />
          ) : route.mode === "view" && item.resource === "hosts" && route.id ? (
            // A host's detail page is the form plus the two live panes; the
            // tab is in the URL so a terminal link can be shared directly.
            <HostDetail
              workspace={current.id}
              hostId={route.id}
              permissions={permissions}
              form={formConfig}
              alerts={lists["alert-rules"]}
              alertForm={forms["alert-rules"]}
              alertEvents={lists["alert-events"]}
              holes={lists.holes}
              holeForm={forms.holes}
            />
          ) : formConfig ? (
            <div className="min-h-0 overflow-auto">
              <ResourceForm
                key={`${current.id}/${route.group}/${item.resource}/${route.id ?? "new"}/${route.mode}`}
                {...(item.resource === "roles" || item.resource === "members" ? { onSaved: reload } : {})}
                config={formConfig}
                workspace={current.id}
                permissions={permissions}
              />
            </div>
          ) : (
            <UnavailablePage onBack={() => navigate({ mode: "list" }, {})} />
          )}
        </main>
      </div>}
      <MobileNavigation active={group.key} items={groups.map(entry => ({key:entry.key,label:entry.label,...to(entry.key,menuTarget(findGroup(entry.key)!,entry.items[0]!).key)}))} />
    </div>
  );
}

const UnavailablePage = ({ onBack }: { onBack: () => void }) => (
  <div role="alert"><p>{commonText("shell.unavailable")}</p><Button variant="outline" onClick={onBack}>{commonText("common.backList")}</Button></div>
);

function AppearanceActions({ floating = false }: { floating?: boolean }) {
  return <div className={floating ? "fixed end-3 top-2 z-40 flex items-center gap-1 rounded-md bg-card" : "flex shrink-0 items-center gap-1"}><LanguageSwitcher /><ThemeSwitcher /></div>;
}

function WorkspaceHeaderActions({ identity, onWorkspaces, onSignedOut }: { identity: Identity; onWorkspaces?: () => void; onSignedOut: () => void }) {
  const { t } = useTranslation();
  return <>
        <AppearanceActions />

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="sm">
              {identity.user.name}
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end">
            <DropdownMenuItem onClick={() => navigate(accountRoute, {})}>{t("app.account")}</DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => void api("/sessions", { method: "DELETE" }).then(onSignedOut)}
            >
              <LogOut /> {t("app.signOut")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        {onWorkspaces && <WorkspaceSwitcher identity={identity} onWorkspaces={onWorkspaces} />}
  </>;
}

function WorkspaceSwitcher({ identity, current, onWorkspaces }: { identity: Identity; current?: Identity["workspaces"][number]; onWorkspaces: () => void }) {
  const { t } = useTranslation();
  const route = useRoute();
  return (
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button variant="ghost" size="sm" className="min-w-0 max-w-[min(18rem,calc(100vw-2rem))] shrink gap-1.5" aria-label={workflowText("switchWorkspace")}>
              <span className="truncate">{current?.name ?? workflowText("switchWorkspace")}</span>
              <ChevronDown className="size-4 shrink-0" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" collisionPadding={8} className="flex max-h-[min(60dvh,var(--radix-dropdown-menu-content-available-height),28rem)] w-[min(22rem,var(--radix-dropdown-menu-content-available-width))] flex-col overflow-hidden">
            <DropdownMenuRadioGroup
              className="min-h-0 overflow-y-auto overscroll-contain"
              value={current?.id}
              onValueChange={(workspace) => {
                if (workspace !== current?.id)
                  navigate({ workspace, group: current ? route.group : nav[0]!.key, item: current ? route.item : nav[0]!.items[0]!.key, mode: "list" }, {});
              }}
            >
              {identity.workspaces.map((w) => (
                <DropdownMenuRadioItem key={w.id} value={w.id} className="flex-wrap">
                  <span className="min-w-0 flex-1 basis-32 break-words">{w.name}</span>
                  <span className="min-w-0 break-words text-xs text-muted-foreground">{myRole(w.relation, w.role_name)}</span>
                </DropdownMenuRadioItem>
              ))}
            </DropdownMenuRadioGroup>
            <DropdownMenuSeparator className="shrink-0" />
            <DropdownMenuItem className="min-h-11 shrink-0 justify-center whitespace-normal text-center" onClick={() => onWorkspaces()}>{t("app.workspaces")}</DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
  );
}
