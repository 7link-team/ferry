import { cn } from "cn";
import { Activity,Bell,FolderOpen,Info,Pencil,Plus,SquareTerminal,X } from "lucide-react";
import { Dialog } from "radix-ui";
import { useEffect,useMemo,useState } from "react";
import { HostMonitor } from "../../features/hosts/HostMonitor";
import { api,errorText } from "../../shared/api/api";
import { commonText,useCommonTranslation } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
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
}: {
  workspace: string;
  hostId: string;
  permissions: Set<string>;
  form: ReturnType<typeof formConfigs>[string] | undefined;
  alerts: ReturnType<typeof listConfigs>["alert-rules"] | undefined;
  alertForm: ReturnType<typeof formConfigs>[string] | undefined;
  alertEvents: ReturnType<typeof listConfigs>["alert-events"] | undefined;
}) {
  useCommonTranslation();
  // `?tab=` rather than component state: a shared link should reopen on the
  // terminal if that is what the sender was looking at.
  const [tab, setTab] = useQueryValue("tab", "info");
  const [alertView, setAlertView] = useState<"rules" | "events">("rules");
  const [creatingAlert, setCreatingAlert] = useState(false);
  const [alertRevision, setAlertRevision] = useState(0);
  const nodeAlertForm = useMemo(() => alertForm ? {
    ...alertForm,
    blank: { ...alertForm.blank, target_kind: "host", target_ref: hostId },
    sections: alertForm.sections.map(section => ({ ...section, fields: section.fields.filter(field => field.key !== "target_kind" && field.key !== "target_ref") })),
  } : undefined, [alertForm, hostId]);
  useEffect(() => setCreatingAlert(false), [workspace, hostId]);
  useEffect(() => {
    if (tab === "terminal" || tab === "files") location.replace(href({ workspace, group: "host", item: "hosts", id: hostId, mode: "workspace" }, { tool: tab }));
  }, [tab, workspace, hostId]);
  const canEnroll = permissions.has("hosts.enroll");
  const [connection, setConnection] = useState<{ workspace: string; hostId: string; kind: string }>();
  const [connectionError, setConnectionError] = useState("");
  useEffect(() => {
    setConnection(undefined); setConnectionError("");
    if (!permissions.has("hosts.read")) return;
    const controller = new AbortController();
    void api<{ connection: string }>(`/workspaces/${encodeURIComponent(workspace)}/nodes/${encodeURIComponent(hostId)}`, { signal: controller.signal })
      .then(host => { if (!controller.signal.aborted) setConnection({ workspace, hostId, kind: host.connection }); })
      .catch(cause => { if (!controller.signal.aborted) setConnectionError(errorText(cause)); });
    return () => controller.abort();
  }, [workspace, hostId, permissions]);
  const canConnect = canEnroll && connection?.workspace === workspace && connection.hostId === hostId && connection.kind === "runner";

  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3">
    <Dialog.Root open={creatingAlert} onOpenChange={setCreatingAlert}><Dialog.Portal>
      <Dialog.Overlay className="fixed inset-0 z-50 bg-black/40" />
      <Dialog.Content className="fixed left-1/2 top-1/2 z-50 max-h-[calc(100dvh-2rem)] w-[min(48rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 overflow-auto rounded-xl border bg-background p-6 shadow-lg">
        <div className="mb-1 flex items-center justify-between gap-3"><Dialog.Title className="text-lg font-semibold">{commonText("shell.newAlert")}</Dialog.Title><Dialog.Close asChild><Button type="button" size="icon" variant="ghost" aria-label={commonText("common.close")} title={commonText("common.close")}><X className="size-4" /></Button></Dialog.Close></div>
        <Dialog.Description className="mb-5 text-sm text-muted-foreground">{commonText("shell.newAlertHelp")}</Dialog.Description>
        {nodeAlertForm && <ResourceForm config={nodeAlertForm} workspace={workspace} permissions={permissions} createInPlace onCancel={() => setCreatingAlert(false)} onSaved={() => { setCreatingAlert(false); setAlertRevision(value => value + 1); }} />}
      </Dialog.Content>
    </Dialog.Portal></Dialog.Root>
    <div className="flex flex-wrap items-center justify-between gap-2 border-b">
      <div role="tablist" aria-label={commonText("shell.hostDetail")} className="flex gap-1">
        <Button type="button" role="tab" title={commonText("common.basic")} aria-selected={tab === "info"} variant="ghost" size="sm" className={cn("rounded-none border-b-2", tab === "info" ? "border-primary text-primary" : "border-transparent")} onClick={() => setTab("info")}><Info className="size-4" />{commonText("common.basic")}</Button>
        <Button type="button" role="tab" title={commonText("shell.monitor")} aria-selected={tab === "monitor"} variant="ghost" size="sm" className={cn("rounded-none border-b-2", tab === "monitor" ? "border-primary text-primary" : "border-transparent")} onClick={() => setTab("monitor")}><Activity className="size-4" />{commonText("shell.monitor")}</Button>
        {permissions.has("alerts.read") && <Button type="button" role="tab" title={commonText("shell.alerts")} aria-selected={tab === "alerts"} variant="ghost" size="sm" className={cn("rounded-none border-b-2", tab === "alerts" ? "border-primary text-primary" : "border-transparent")} onClick={() => setTab("alerts")}><Bell className="size-4" />{commonText("shell.alerts")}</Button>}
      </div>
      <div className="flex items-center gap-2">
      {permissions.has("hosts.terminal") && <a href={href({ workspace, group: "host", item: "hosts", id: hostId, mode: "workspace" }, { tool: "terminal" })} target="_blank" rel="noopener noreferrer" className="inline-flex size-8 items-center justify-center rounded border hover:bg-accent" aria-label={commonText("shell.openTerminal")} title={commonText("shell.openTerminal")}><SquareTerminal className="size-4" /></a>}
      {permissions.has("hosts.files") && <a href={href({ workspace, group: "host", item: "hosts", id: hostId, mode: "workspace" }, { tool: "files" })} target="_blank" rel="noopener noreferrer" className="inline-flex size-8 items-center justify-center rounded border hover:bg-accent" aria-label={commonText("shell.openFiles")} title={commonText("shell.openFiles")}><FolderOpen className="size-4" /></a>}
      {permissions.has("hosts.update") && <Button asChild variant="outline" size="sm"><a href={href({ workspace, group: "host", item: "hosts", id: hostId, mode: "edit" }, {})}><Pencil className="size-4" />{commonText("common.edit")}</a></Button>}
      </div>
    </div>
      <div role="tabpanel" aria-label={tab === "monitor" ? commonText("shell.monitor") : tab === "alerts" ? commonText("shell.alerts") : commonText("common.basic")} className={cn("min-h-0 overflow-auto pt-5", tab === "alerts" && "flex flex-1 flex-col")}>
        {tab === "monitor" ? <HostMonitor workspace={workspace} hostId={hostId} /> : tab === "alerts" && alerts && alertEvents ? <div className="flex min-h-0 flex-1 flex-col gap-3">
          <div className="flex items-center gap-2">
            <div role="tablist" aria-label={commonText("shell.hostAlerts")} className="flex gap-1">
              <Button type="button" role="tab" aria-selected={alertView === "rules"} size="sm" variant={alertView === "rules" ? "secondary" : "ghost"} onClick={() => setAlertView("rules")}>{commonText("shell.rules")}</Button>
              <Button type="button" role="tab" aria-selected={alertView === "events"} size="sm" variant={alertView === "events" ? "secondary" : "ghost"} onClick={() => setAlertView("events")}>{commonText("nav.alertEvents")}</Button>
            </div>
            <span className="flex-1" />
            {alertView === "rules" && alertForm && permissions.has("alerts.create") && <Button type="button" size="sm" onClick={() => setCreatingAlert(true)}><Plus className="size-4" />{commonText("shell.newAlert")}</Button>}
          </div>
          {alertView === "rules" ? <ResourceTable key={`${hostId}/rules/${alertRevision}`} config={{ ...alerts, fixed: { target_kind: "host", target_ref: hostId }, filters: (alerts.filters ?? []).filter(filter => filter.key !== "target_kind"), toolbar: { ...alerts.toolbar, create: false }, actions: alerts.actions?.map(action => action.to ? { ...action, to: (row: Parameters<NonNullable<typeof action.to>>[0]) => ({ group: "deploy", item: "alerts", ...action.to!(row) }) } : action) ?? [] }} workspace={workspace} permissions={permissions} />
            : <ResourceTable key={`${hostId}/events`} config={{ ...alertEvents, fixed: { target_kind: "host", target_ref: hostId }, filters: (alertEvents.filters ?? []).filter(filter => filter.key !== "rule_id"), actions: alertEvents.actions?.map(action => action.to ? { ...action, to: (row: Parameters<NonNullable<typeof action.to>>[0]) => ({ group: "deploy", item: "alert-events", ...action.to!(row) }) } : action) ?? [] }} workspace={workspace} permissions={permissions} />}
        </div> : <>
        {connectionError && <p role="alert" className="mb-3 text-sm text-destructive">{commonText("shell.agentUnknown")}{connectionError}</p>}
        <div className={cn("grid w-full min-w-0 items-start gap-6", canConnect && "2xl:grid-cols-[minmax(0,1.5fr)_minmax(22rem,1fr)]")}>
          {form && <ResourceForm config={form} workspace={workspace} permissions={permissions} showViewNavigation={false} />}
          {canConnect && <HostConnectionCommand workspace={workspace} hostId={hostId} compact />}
        </div>
        {connection?.workspace === workspace && connection.hostId === hostId && connection.kind === "runner" && <RunnerUpdateStatus workspace={workspace} hostId={hostId} />}
        </>}
      </div>
    </div>
  );
}
