import { cn } from "cn";
import { useMemo } from "react";
import type { PipelineRun } from "../../features/pipelines/pipeline";
import { commonText,useCommonTranslation } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { RemoteResourceHeading } from "../../shared/ui/resource-heading";
import { formConfigs,listConfigs } from "../resources";
import { ResourceForm } from "../resources/ResourceForm";
import { ResourceTable } from "../resources/ResourceTable";
import { useQueryValue } from "../router";
import { GroupSidebar } from "./GroupSidebar";

export function PipelineDetail({ workspace, pipelineId, permissions, form, runs }: {
  workspace: string;
  pipelineId: string;
  permissions: Set<string>;
  form: NonNullable<ReturnType<typeof formConfigs>[string]>;
  runs: NonNullable<ReturnType<typeof listConfigs>["pipeline-runs"]>;
}) {
  useCommonTranslation();
  const [tab, setTab] = useQueryValue("tab", "config");
  return <div className="flex min-h-0 flex-1 flex-col gap-3">
    <RemoteResourceHeading endpoint={`/workspaces/${encodeURIComponent(workspace)}/pipelines/${encodeURIComponent(pipelineId)}`} fallback={pipelineId} />
    <div role="tablist" aria-label={commonText("shell.pipelineDetail")} className="flex gap-1 border-b">
      {([["config", commonText("shell.configuration")], ["runs", commonText("shell.executionRecords")]] as const).map(([value, label]) => <Button key={value} type="button" role="tab" aria-selected={tab === value} variant="ghost" className={cn("rounded-none border-b-2", tab === value ? "border-primary text-primary" : "border-transparent")} onClick={() => setTab(value)}>{label}</Button>)}
    </div>
    {tab === "runs" ? <ResourceTable key={pipelineId} config={{ ...runs, fixed: { pipeline_id: pipelineId }, actions: [{ label: "查看", labelKey: "common.view", permission: "pipelines.read", to: (row: PipelineRun) => ({ group: "build", item: "runs", id: row.id, mode: "view" as const }) }] }} workspace={workspace} permissions={permissions} /> : <div className="min-h-0 overflow-auto"><ResourceForm config={form} workspace={workspace} permissions={permissions} showHeading={false} /></div>}
  </div>;
}

export function PipelineList({ config, workspace, permissions }: {
  config: NonNullable<ReturnType<typeof listConfigs>["pipelines"]>;
  workspace: string;
  permissions: Set<string>;
}) {
  useCommonTranslation();
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
    {permissions.has("pipeline-groups.read") && <GroupSidebar workspace={workspace} permissions={permissions} resource="pipeline-groups" noun={commonText("shell.pipeline")} active={groupId} onSelect={select} />}
    <div className="flex min-h-0 min-w-0 flex-1 flex-col"><ResourceTable key={groupId} config={tableConfig} workspace={workspace} permissions={permissions} /></div>
  </div>;
}
