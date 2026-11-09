import { cn } from "cn";
import { Activity,Bell,FolderOpen,Info,Link2,Pencil,Plus,SquareTerminal } from "lucide-react";
import { useEffect,useState } from "react";
import { HostMonitor } from "../../features/hosts/HostMonitor";
import { api,errorText } from "../../shared/api/api";
import { commonText,useCommonTranslation } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { ResourceHeading } from "../../shared/ui/resource-heading";
import { formConfigs,listConfigs } from "../resources";
import { ResourceForm } from "../resources/ResourceForm";
import { ResourceTable } from "../resources/ResourceTable";
import { href,useQueryValue } from "../router";
import { HostConnectionCommand,RunnerUpdateStatus } from "./RunnerConnection";

export function HostDetail({
  workspace,
  hostId,
  permissions,
  form,
  alerts,
  alertForm,
  alertEvents,
  holes,
  holeForm,
}: {
  workspace: string;
  hostId: string;
  permissions: Set<string>;
  form: ReturnType<typeof formConfigs>[string] | undefined;
  alerts: ReturnType<typeof listConfigs>["alert-rules"] | undefined;
  alertForm: ReturnType<typeof formConfigs>[string] | undefined;
  alertEvents: ReturnType<typeof listConfigs>["alert-events"] | undefined;
  holes?: ReturnType<typeof listConfigs>["holes"] | undefined;
  holeForm?: ReturnType<typeof formConfigs>[string] | undefined;
}) {
  useCommonTranslation();
  // `?tab=` rather than component state: a shared link should reopen on the
  // terminal if that is what the sender was looking at.
  const [tab, setTab] = useQueryValue("tab", "info");
  const [alertView, setAlertView] = useState<"rules" | "events">("rules");
  useEffect(() => {
    if (tab === "terminal" || tab === "files") location.replace(href({ workspace, group: "host", item: "hosts", id: hostId, mode: "workspace" }, { tool: tab }));
  }, [tab, workspace, hostId]);
  const canEnroll = permissions.has("hosts.enroll");
  const [connection, setConnection] = useState<{ workspace: string; hostId: string; kind: string; name: string }>();
  const [connectionError, setConnectionError] = useState("");
  useEffect(() => {
    setConnection(undefined); setConnectionError("");
    if (!permissions.has("hosts.read")) return;
    const controller = new AbortController();
    void api<{ connection: string; name: string }>(`/workspaces/${encodeURIComponent(workspace)}/nodes/${encodeURIComponent(hostId)}`, { signal: controller.signal })
      .then(host => { if (!controller.signal.aborted) setConnection({ workspace, hostId, kind: host.connection, name: host.name }); })
      .catch(cause => { if (!controller.signal.aborted) setConnectionError(errorText(cause)); });
    return () => controller.abort();
  }, [workspace, hostId, permissions]);
  const canConnect = canEnroll && connection?.workspace === workspace && connection.hostId === hostId && connection.kind === "runner";

  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3">
    <ResourceHeading name={connection?.name || hostId} />

    <div className="flex flex-wrap items-center justify-between gap-2 border-b">
      <div role="tablist" aria-label={commonText("shell.hostDetail")} className="flex gap-1">
        <Button type="button" role="tab" title={commonText("common.basic")} aria-selected={tab === "info"} variant="ghost" size="sm" className={cn("rounded-none border-b-2", tab === "info" ? "border-primary text-primary" : "border-transparent")} onClick={() => setTab("info")}><Info className="size-4" />{commonText("common.basic")}</Button>
        <Button type="button" role="tab" title={commonText("shell.monitor")} aria-selected={tab === "monitor"} variant="ghost" size="sm" className={cn("rounded-none border-b-2", tab === "monitor" ? "border-primary text-primary" : "border-transparent")} onClick={() => setTab("monitor")}><Activity className="size-4" />{commonText("shell.monitor")}</Button>
        {permissions.has("alerts.read") && <Button type="button" role="tab" title={commonText("shell.alerts")} aria-selected={tab === "alerts"} variant="ghost" size="sm" className={cn("rounded-none border-b-2", tab === "alerts" ? "border-primary text-primary" : "border-transparent")} onClick={() => setTab("alerts")}><Bell className="size-4" />{commonText("shell.alerts")}</Button>}
        {permissions.has("holes.read") && <Button type="button" role="tab" title={commonText("holes:title")} aria-selected={tab === "holes"} variant="ghost" size="sm" className={cn("rounded-none border-b-2", tab === "holes" ? "border-primary text-primary" : "border-transparent")} onClick={() => setTab("holes")}><Link2 className="size-4" />{commonText("holes:title")}</Button>}
      </div>
      <div className="flex items-center gap-2">
      {permissions.has("hosts.terminal") && <a href={href({ workspace, group: "host", item: "hosts", id: hostId, mode: "workspace" }, { tool: "terminal" })} target="_blank" rel="noopener noreferrer" className="inline-flex size-8 items-center justify-center rounded border hover:bg-accent" aria-label={commonText("shell.openTerminal")} title={commonText("shell.openTerminal")}><SquareTerminal className="size-4" /></a>}
      {permissions.has("hosts.files") && <a href={href({ workspace, group: "host", item: "hosts", id: hostId, mode: "workspace" }, { tool: "files" })} target="_blank" rel="noopener noreferrer" className="inline-flex size-8 items-center justify-center rounded border hover:bg-accent" aria-label={commonText("shell.openFiles")} title={commonText("shell.openFiles")}><FolderOpen className="size-4" /></a>}
      {permissions.has("hosts.update") && <Button asChild variant="outline" size="sm"><a href={href({ workspace, group: "host", item: "hosts", id: hostId, mode: "edit" }, {})}><Pencil className="size-4" />{commonText("common.edit")}</a></Button>}
      </div>
    </div>
      <div role="tabpanel" aria-label={tab === "monitor" ? commonText("shell.monitor") : tab === "alerts" ? commonText("shell.alerts") : tab === "holes" ? commonText("holes:title") : commonText("common.basic")} className={cn("min-h-0 min-w-0 overflow-auto pt-5", (tab === "alerts" || tab === "holes") && "flex flex-1 flex-col")}>
        {tab === "monitor" ? <HostMonitor workspace={workspace} hostId={hostId} /> : tab === "holes" && holes && permissions.has("holes.read") ? <div className="flex min-h-[32rem] min-w-0 flex-1 flex-col gap-3">
          {holeForm && permissions.has("holes.create") && <div className="flex justify-end"><Button asChild size="sm"><a href={href({workspace,group:"host",item:"holes",mode:"new"},{host_id:hostId,return_to:location.pathname+location.search+location.hash})}><Plus className="size-4" />{commonText("holes:create")}</a></Button></div>}
          <ResourceTable key={`${hostId}/holes`} config={{ ...holes, fixed: { host_id: hostId }, filters: (holes.filters ?? []).filter(filter => filter.key !== "host_id"), toolbar: { ...holes.toolbar, create: false }, actions: holes.actions?.map(action => action.to ? { ...action, to: (row: Parameters<NonNullable<typeof action.to>>[0]) => ({ group: "host", item: "holes", ...action.to!(row) }) } : action) ?? [] }} workspace={workspace} permissions={permissions} />
        </div> : tab === "alerts" && alerts && alertEvents ? <div className="flex min-h-0 flex-1 flex-col gap-3">
          <div className="flex items-center gap-2">
            <div role="tablist" aria-label={commonText("shell.hostAlerts")} className="flex gap-1">
              <Button type="button" role="tab" aria-selected={alertView === "rules"} size="sm" variant={alertView === "rules" ? "secondary" : "ghost"} onClick={() => setAlertView("rules")}>{commonText("shell.rules")}</Button>
              <Button type="button" role="tab" aria-selected={alertView === "events"} size="sm" variant={alertView === "events" ? "secondary" : "ghost"} onClick={() => setAlertView("events")}>{commonText("nav.alertEvents")}</Button>
            </div>
            <span className="flex-1" />
            {alertView === "rules" && alertForm && permissions.has("alerts.create") && <Button asChild size="sm"><a href={href({workspace,group:"deploy",item:"alerts",mode:"new"},{target_kind:"host",target_ref:hostId,return_to:location.pathname+location.search+location.hash})}><Plus className="size-4" />{commonText("shell.newAlert")}</a></Button>}
          </div>
          {alertView === "rules" ? <ResourceTable key={`${hostId}/rules`} config={{ ...alerts, fixed: { target_kind: "host", target_ref: hostId }, filters: (alerts.filters ?? []).filter(filter => filter.key !== "target_kind"), toolbar: { ...alerts.toolbar, create: false }, actions: alerts.actions?.map(action => action.to ? { ...action, to: (row: Parameters<NonNullable<typeof action.to>>[0]) => ({ group: "deploy", item: "alerts", ...action.to!(row) }) } : action) ?? [] }} workspace={workspace} permissions={permissions} />
            : <ResourceTable key={`${hostId}/events`} config={{ ...alertEvents, fixed: { target_kind: "host", target_ref: hostId }, filters: (alertEvents.filters ?? []).filter(filter => filter.key !== "rule_id"), actions: alertEvents.actions?.map(action => action.to ? { ...action, to: (row: Parameters<NonNullable<typeof action.to>>[0]) => ({ group: "deploy", item: "alert-events", ...action.to!(row) }) } : action) ?? [] }} workspace={workspace} permissions={permissions} />}
        </div> : <>
        {connectionError && <p role="alert" className="mb-3 text-sm text-destructive">{commonText("shell.agentUnknown")}{connectionError}</p>}
        <div className={cn("grid w-full min-w-0 items-start gap-6", canConnect && "2xl:grid-cols-[minmax(0,1.5fr)_minmax(22rem,1fr)]")}>
          {form && <ResourceForm config={form} workspace={workspace} permissions={permissions} showViewNavigation={false} showHeading={false} />}
          {canConnect && <HostConnectionCommand workspace={workspace} hostId={hostId} compact />}
        </div>
        {connection?.workspace === workspace && connection.hostId === hostId && connection.kind === "runner" && <RunnerUpdateStatus workspace={workspace} hostId={hostId} canUpdate={permissions.has("hosts.update")} />}
        </>}
      </div>
    </div>
  );
}
