import { cn } from "cn";
import { Bell,FolderTree,History,Info } from "lucide-react";
import { Tabs } from "radix-ui";
import { useEffect,useMemo,useState } from "react";
import { api,errorText } from "../../shared/api/api";
import { base } from "../../shared/api/resource";
import { commonText,useCommonTranslation } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { ConnectionInsights } from "../pages/CodeInsight";
import { RepositoryEventHistory,RepositoryMonitor } from "../pages/RepositoryMonitor";
import { formConfigs } from "../resources";
import { ResourceForm } from "../resources/ResourceForm";
import { ResourceTable } from "../resources/ResourceTable";
import { useQueryValue,useRoute } from "../router";

export function RepositoryConnectionDetail({ workspace, connectionId, permissions, form }: {
  workspace: string; connectionId: string; permissions: Set<string>;
  form: ReturnType<typeof formConfigs>[string];
}) {
  useCommonTranslation();
  const route = useRoute();
  const [tab, setTab] = useQueryValue("tab", "info");
  const [connection, setConnection] = useState<{ provider?: string }>({});
  const [connectionError, setConnectionError] = useState("");
  useEffect(() => {
    const controller = new AbortController();
    setConnection({});
    setConnectionError("");
    void api<{ provider: string }>(`${base(workspace, "code-connections")}/${encodeURIComponent(connectionId)}`, { signal: controller.signal })
      .then(row => { if (!controller.signal.aborted) setConnection(row); })
      .catch(cause => { if (!controller.signal.aborted) setConnectionError(errorText(cause)); });
    return () => controller.abort();
  }, [workspace, connectionId]);
  const repositories = useMemo(() => ({
    resource: "connection-repositories", permissionResource: "repositories",
    endpoint: `${base(workspace, "code-connections")}/${encodeURIComponent(connectionId)}/repositories`,
    pagination: "next" as const,
    rowKey: (row: { url: string; name: string; full_name: string; default_branch: string }) => row.url,
    grouping: { label: "namespace", labelKey: "repository.namespace", key: (row: { full_name: string }) => row.full_name.includes("/") ? row.full_name.slice(0, row.full_name.lastIndexOf("/")) : "" },
    columns: [
      { key: "full_name", label: "name", labelKey: "repository.name", render: (row: { name: string; full_name: string }) => row.full_name || row.name },
      { key: "url", label: "address", labelKey: "repository.address" },
      { key: "default_branch", label: "branch", labelKey: "repository.branch", render: (row: { default_branch: string }) => row.default_branch || "—" },
    ],
    toolbar: { create: false, import: false, export: false },
    emptyKey: "repository.empty",
  }), [workspace, connectionId]);
  const activeTab = tab === "repositories" || tab === "monitoring" || tab === "events" || tab === "insights" ? tab : tab === "analysis" ? "insights" : "info";
  if (connectionError) return <p role="alert" className="text-destructive">{connectionError}</p>;
  if (route.mode === "edit") return connection.provider ? <RepositoryMonitor workspace={workspace} connectionId={connectionId} provider={connection.provider} permissions={permissions} /> : <LoadingSkeleton />;
  return <Tabs.Root value={activeTab} onValueChange={setTab} className="flex min-h-0 flex-1 flex-col gap-4">
    <Tabs.List aria-label={commonText("shell.repositoryDetail")} className="flex flex-wrap gap-1 border-b">
      {([["info", commonText("common.basic"), Info], ["repositories", commonText("shell.repositoryList"), FolderTree], ["monitoring", commonText("shell.monitorNotifications"), Bell], ["events", commonText("shell.eventRecords"), History], ["insights", commonText("nav.insights"), FolderTree]] as const).map(([value, label, Icon]) =>
        <Tabs.Trigger key={value} value={value} asChild><Button variant="ghost" size="sm" className={cn("rounded-none border-b-2", activeTab === value ? "border-primary text-primary" : "border-transparent")}><Icon className="size-4" />{label}</Button></Tabs.Trigger>)}
    </Tabs.List>
    <Tabs.Content value="info" className="min-h-0 flex-1 overflow-auto"><ResourceForm config={form} workspace={workspace} permissions={permissions} /></Tabs.Content>
    <Tabs.Content value="repositories" className="flex min-h-0 flex-1 flex-col overflow-auto"><ResourceTable config={repositories} workspace={workspace} permissions={permissions} /></Tabs.Content>
    <Tabs.Content value="monitoring" className="flex min-h-0 flex-1 flex-col overflow-auto"><RepositoryMonitor workspace={workspace} connectionId={connectionId} provider={connection.provider} permissions={permissions} /></Tabs.Content>
    <Tabs.Content value="events" className="flex min-h-0 flex-1 flex-col overflow-auto"><RepositoryEventHistory workspace={workspace} connectionId={connectionId} provider={connection.provider} permissions={permissions} /></Tabs.Content>
    <Tabs.Content value="insights" className="flex min-h-0 flex-1 flex-col overflow-auto"><ConnectionInsights workspace={workspace} connectionId={connectionId} permissions={permissions} /></Tabs.Content>
  </Tabs.Root>;
}
