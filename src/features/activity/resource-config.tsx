import { currentLocale } from "../../shared/i18n/locale";
import { commonText } from "../../shared/i18n/i18n";
import type { ResourceConfigContext } from "../../shared/resources/config";
import { type AnyRow, list } from "../../shared/resources/config";
import type { ListConfig } from "../../shared/resources/types";
import { lazy, Suspense, useState } from "react";
import { Dialog } from "radix-ui";
import { Copy, Eye, X } from "lucide-react";
import { Button } from "../../shared/ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../shared/ui/tabs";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";

const JsonCodeEditor = lazy(() => import("../../shared/editor/CodeEditor").then(module => ({ default: module.CodeEditor })));

export interface Activity {
  id: string;
  actor_name: string;
  action: string;
  resource: string;
  resource_id: string;
  resource_name?: string | null;
  details: unknown;
  created_at: number;
}

function activityDetails(row: Activity): Record<string, unknown> {
  return row.details && typeof row.details === "object" && !Array.isArray(row.details)
    ? row.details as Record<string, unknown> : {};
}

const resourceLabels: Record<string, string> = {
  conversations: "conversations:nav", "ai-agents": "agents:title", "ai-models": "models:title",
  workflows: "workflow-domain:kind.nodes", "code-connections": "common-domain:nav.repositories",
  repositories: "common-domain:nav.repositories", "code-insights": "common-domain:nav.insights",
  "host-groups": "common-domain:nav.hostGroups", "cron-jobs": "common-domain:nav.cronJobs",
  "notification-channels": "notifications:notification-channels",
};

function activityDescription(row: Activity) {
  const details = activityDetails(row);
  const fields: [string, string][] = [];
  const labels: Record<string, string> = {
    name: "activity.fieldName", title: "activity.fieldName", description: "activity.fieldDescription",
    execution_id: "activity.executionId", revision_id: "activity.revisionId", permissions: "activity.permissions",
    enabled: "activity.enabled", monitoring: "activity.monitoring", secret_rotated: "activity.secretRotated",
    lifecycle: "activity.state", group_id: "activity.groupId", repository_id: "activity.repositoryId",
    application_id: "activity.applicationId", target_kind: "activity.targetKind", target_ref: "activity.targetRef",
    draft_revision_id: "activity.revisionId", definition_changed: "activity.definitionChanged",
    appearance: "activity.appearanceChanged",
    state: "activity.state", turn_id: "activity.turnId", run_id: "activity.runId",
    message_id: "activity.messageId", request_id: "activity.requestId", allow: "activity.allowed",
    target_count: "activity.targetCount", agent_id: "activity.agentId", version: "activity.version",
  };
  const labelFor = (key: string) => labels[key] ? commonText(`resource-domain:${labels[key]}`) : key;
  const format = (key: string, value: unknown): string => {
    if (value === null) return commonText("common.notSet");
    if (value === "") return commonText("resource-domain:activity.emptyText");
    if (typeof value === "boolean") return commonText(value ? "common.yes" : "common.no");
    if (key === "state" && row.resource === "conversations" && typeof value === "string")
      return commonText(`conversations:${value}`, { defaultValue: value });
    const text = typeof value === "object" ? JSON.stringify(value, null, 2) : String(value);
    return text.length > 3000 ? `${text.slice(0, 3000)}… ${commonText("resource-domain:activity.moreInJson")}` : text;
  };
  for (const [key, value] of Object.entries(details)) {
    if (key === "changes" || (key === "membership" && row.resource === "conversations")
      || (key === "appearance" && row.resource === "conversations" && row.action === "update")) continue;
    fields.push([labelFor(key), format(key, value)]);
  }
  const changes = details.changes && typeof details.changes === "object" && !Array.isArray(details.changes)
    ? Object.entries(details.changes as Record<string, unknown>) : null;
  const membership = details.membership;
  let membershipRecorded = false;
  let membershipChanged = false;
  if (row.resource === "conversations" && row.action === "members" && membership && typeof membership === "object") {
    for (const kind of ["members", "agents"] as const) {
      const group = (membership as Record<string, unknown>)[kind];
      if (!group || typeof group !== "object") continue;
      for (const action of ["added", "removed"] as const) {
        const entries = (group as Record<string, unknown>)[action];
        if (!Array.isArray(entries)) continue;
        membershipRecorded = true;
        if (!entries.length) continue;
        membershipChanged = true;
        fields.push([commonText(`resource-domain:activity.${kind}.${action}`), entries.map(entry => {
          if (!entry || typeof entry !== "object") return "—";
          const name = typeof entry.name === "string" ? entry.name : "";
          const id = typeof entry.id === "string" || typeof entry.id === "number" ? String(entry.id) : "";
          return name && id ? `${name} (${id})` : name || id || "—";
        }).join(" · ")]);
      }
    }
  }
  const appearance = details.appearance;
  const background = row.resource === "conversations" && row.action === "update"
    && appearance && typeof appearance === "object" && !Array.isArray(appearance)
    ? appearance as Record<string, unknown> : null;
  return <div className="min-w-48 space-y-2">
    {changes && <div className="space-y-2">
      {changes.length === 0 && <p className="text-sm text-muted-foreground">{commonText("resource-domain:activity.noChanges")}</p>}
      {changes.slice(0, 50).map(([key, value]) => {
        if (!value || typeof value !== "object" || Array.isArray(value)) return <p key={key}>{labelFor(key)}: {format(key, value)}</p>;
        const change = value as Record<string, unknown>;
        return <section key={key} className="min-w-0 rounded-md border p-3 space-y-2">
          <h4 className="break-words text-sm font-medium">{labelFor(key)}</h4>
          <dl className="grid min-w-0 grid-cols-1 gap-3 sm:grid-cols-2">
            {(["before", "after"] as const).map(side => <div key={side} className="min-w-0">
              <dt className="text-xs text-muted-foreground">{commonText(`resource-domain:activity.${side}`)}</dt>
              <dd className="mt-1 whitespace-pre-wrap break-words text-sm [overflow-wrap:anywhere]">{change[`${side}_present`] === false
                ? commonText("resource-domain:activity.absentField") : format(key, change[side])}</dd>
            </div>)}
          </dl>
        </section>;
      })}
      {changes.length > 50 && <p>{commonText("resource-domain:activity.moreInJson")}</p>}
    </div>}
    {fields.length > 0 && <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-sm">
      {fields.slice(0, 50).map(([label, value], index) => <div key={index} className="contents"><dt className="max-w-40 break-words text-muted-foreground">{label}</dt><dd className="min-w-0 whitespace-pre-wrap break-words [overflow-wrap:anywhere]">{value}</dd></div>)}
    </dl>}
    {fields.length > 50 && <p>{commonText("resource-domain:activity.moreInJson")}</p>}
    {row.action === "members" && (!membershipRecorded || !membershipChanged) && <p className="text-sm text-muted-foreground">{membershipRecorded
      ? commonText("resource-domain:activity.membersUnchanged")
      : commonText("resource-domain:activity.detailsNotRecorded")}</p>}
    {background && <div className="space-y-1">
      <div className="font-medium">{commonText("resource-domain:activity.appearanceChanged")}</div>
      <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-1 text-sm">
        {typeof background.pattern === "string" && <>
          <dt className="text-muted-foreground">{commonText("resource-domain:activity.backgroundPattern")}</dt>
          <dd className="break-words">{commonText(`conversations:background_${background.pattern}`, { defaultValue: background.pattern })}</dd>
        </>}
        {typeof background.color === "string" && <>
          <dt className="text-muted-foreground">{commonText("conversations:backgroundColor")}</dt>
          <dd>{background.color || commonText("conversations:followTheme")}</dd>
        </>}
        {"image" in background && <>
          <dt className="text-muted-foreground">{commonText("resource-domain:activity.backgroundImage")}</dt>
          <dd className="break-all">{background.image == null ? commonText("common.notSet") : String(background.image)}</dd>
        </>}
      </dl>
    </div>}
    {Object.keys(details).length === 0 && row.action !== "members" && <p className="text-sm text-muted-foreground">{commonText("resource-domain:activity.detailsNotRecorded")}</p>}
    {!changes && row.action === "update" && Object.keys(details).length > 0 && <p className="text-xs text-muted-foreground">{commonText("resource-domain:activity.legacySnapshot")}</p>}
  </div>;
}

function actionLabel(row: Activity) {
  return commonText(`resource-domain:activity.${row.action}`, { defaultValue: row.action });
}
function resourceLabel(row: Activity) {
  return commonText(resourceLabels[row.resource] ?? `nav.${row.resource}`, { defaultValue: row.resource });
}

function ActivityDetails({ row }: { row: Activity }) {
  const [open, setOpen] = useState(false);
  const [copyState, setCopyState] = useState<"" | "copied" | "copyFailed">("");
  const details = activityDetails(row);
  const snapshot = typeof details.name === "string" ? details.name : typeof details.title === "string" ? details.title : "";
  const name = snapshot || row.resource_name;
  const raw = JSON.stringify(row, null, 2);
  const metadata = [
    [commonText("resource-domain:activity.actor"), row.actor_name || "—"],
    [commonText("resource-domain:activity.time"), Number.isFinite(row.created_at) ? new Date(row.created_at).toLocaleString(currentLocale()) : "—"],
    [commonText("resource-domain:resource.action"), actionLabel(row)],
    [commonText("resource-domain:activity.resource"), resourceLabel(row)],
    [commonText("resource-domain:activity.resourceId"), row.resource_id || "—"],
    [commonText("resource-domain:activity.recordId"), row.id],
  ];
  return <div className="space-y-2">
    {activityDescription(row)}
    <Dialog.Root open={open} onOpenChange={next => { setOpen(next); setCopyState(""); }}>
      <Dialog.Trigger asChild><Button size="sm" variant="outline"><Eye aria-hidden="true" />{commonText("resource-domain:activity.viewDetails")}</Button></Dialog.Trigger>
      {open && <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/50" />
        <Dialog.Content className="dialog-shell fixed left-1/2 top-1/2 z-50 h-[min(44rem,90dvh)] w-[min(58rem,calc(100vw-2rem))] -translate-x-1/2 -translate-y-1/2 rounded-xl border bg-background p-4 text-foreground shadow-lg sm:p-6">
          <header className="flex items-start gap-3 border-b pb-4">
            <div className="min-w-0 flex-1 space-y-1">
              <Dialog.Title className="text-lg font-semibold">{commonText("resource-domain:activity.detailTitle")}</Dialog.Title>
              <Dialog.Description className="line-clamp-3 break-words text-sm text-muted-foreground">{actionLabel(row)} · {resourceLabel(row)}{name ? ` · ${name}` : ""}</Dialog.Description>
              {!snapshot && name && <p className="text-xs text-muted-foreground">{commonText("resource-domain:activity.currentName")}</p>}
            </div>
            <Dialog.Close asChild><Button size="icon" variant="ghost" aria-label={commonText("common.close")}><X aria-hidden="true" /></Button></Dialog.Close>
          </header>
          <Tabs defaultValue="overview" className="dialog-form gap-3 py-4">
            <TabsList className="w-full shrink-0 sm:w-fit">
              <TabsTrigger value="overview">{commonText("resource-domain:activity.overview")}</TabsTrigger>
              <TabsTrigger value="json">{commonText("resource-domain:activity.rawJson")}</TabsTrigger>
            </TabsList>
            <TabsContent value="overview" className="dialog-body space-y-5">
              <dl className="grid grid-cols-1 gap-4 rounded-lg border bg-muted/30 p-4 sm:grid-cols-2">
                {metadata.map(([label, value]) => <div key={label} className="min-w-0 space-y-1"><dt className="text-xs text-muted-foreground">{label}</dt><dd className="break-words text-sm font-medium">{value}</dd></div>)}
              </dl>
              <section className="space-y-3"><h3 className="text-sm font-semibold">{commonText(`resource-domain:activity.${details.changes ? "changeDetails" : "recordedDetails"}`)}</h3>{activityDescription(row)}</section>
            </TabsContent>
            <TabsContent value="json" className="min-h-0 flex-1 data-[state=active]:flex data-[state=active]:flex-col">
              <div className="min-h-0 flex-1 overflow-hidden rounded-lg border">
                <Suspense fallback={<LoadingSkeleton variant="editor" />}><JsonCodeEditor value={raw} language="json" path={`audit-record-${row.id}.json`} label={commonText("resource-domain:activity.rawJson")} readOnly onChange={() => {}} /></Suspense>
              </div>
            </TabsContent>
          </Tabs>
          <footer className="dialog-footer">
            {copyState && <p role={copyState === "copyFailed" ? "alert" : "status"} className={copyState === "copyFailed" ? "w-full text-sm text-destructive" : "w-full text-sm text-muted-foreground"}>{commonText(`common.${copyState}`)}</p>}
            <Button className="flex-1" variant="outline" onClick={async () => { try { await navigator.clipboard.writeText(raw); setCopyState("copied"); } catch { setCopyState("copyFailed"); } }}><Copy aria-hidden="true" />{commonText("resource-domain:activity.copyJson")}</Button>
            <Dialog.Close asChild><Button className="flex-1">{commonText("common.close")}</Button></Dialog.Close>
          </footer>
        </Dialog.Content>
      </Dialog.Portal>}
    </Dialog.Root>
  </div>;
}

export function activityList(): ListConfig<AnyRow> {
  return list({
      resource: "activity",
      rowKey: (row: Activity) => row.id,
      filters: [
        {
          key: "q",
          label: "操作人/资源",
          labelKey: "resource-domain:activity.search",
          type: "text",
          placeholder: "输入关键字",
          placeholderKey: "resource-domain:resource.keyword",
        },
        {
          key: "action",
          label: "动作",
          labelKey: "resource-domain:resource.action",
          type: "select",
          options: [
            {
              value: "create",
              label: "新增",
              labelKey: "resource-domain:activity.create",
            },
            {
              value: "update",
              label: "修改",
              labelKey: "resource-domain:activity.update",
            },
            {
              value: "delete",
              label: "删除",
              labelKey: "common-domain:common.delete",
            },
            {
              value: "bulk-delete",
              label: "批量删除",
              labelKey: "resource-domain:resource.bulkDelete",
            },
          ],
        },
      ],
      columns: [
        {
          key: "created_at",
          label: "时间",
          labelKey: "resource-domain:activity.time",
          width: 180,
          sortable: true,
          render: (row: Activity) =>
            new Date(row.created_at).toLocaleString(currentLocale()),
        },
        {
          key: "actor_name",
          label: "操作人",
          labelKey: "resource-domain:activity.actor",
          width: 120,
        },
        {
          key: "action",
          label: "动作",
          labelKey: "resource-domain:resource.action",
          width: 100,
          render: actionLabel,
        },
        {
          key: "resource",
          label: "资源",
          labelKey: "resource-domain:activity.resource",
          width: 140,
          render: resourceLabel,
        },
        {
          key: "resource_id",
          label: "Target resource",
          labelKey: "resource-domain:activity.target",
          render: (row: Activity) => {
            const details = activityDetails(row);
            const snapshot = typeof details.name === "string" ? details.name : typeof details.title === "string" ? details.title : "";
            const name = snapshot || row.resource_name;
            return <div className="min-w-40 space-y-1">
              {name && <div className="max-w-80 break-words font-medium">{name}</div>}
              {!snapshot && name && <div className="text-xs text-muted-foreground">{commonText("resource-domain:activity.currentName")}</div>}
              <code className="break-all text-xs text-muted-foreground">{row.resource_id && row.resource_id !== "0" ? row.resource_id : "—"}</code>
            </div>;
          },
        },
        {
          key: "details",
          label: "详情",
          labelKey: "common-domain:common.details",
          render: (row: Activity) => <ActivityDetails row={row} />,
        },
      ],
      toolbar: { create: false, import: false },
      empty: "还没有操作记录。",
      emptyKey: "resource-domain:activity.empty",
    });
}

export function activityLists({}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return { activity: activityList() };
}
