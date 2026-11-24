import { useMemo } from "react";
import { useCommonTranslation, workflowText } from "../../shared/i18n/i18n";
import { listConfigs } from "../resources";
import { ResourceTable } from "../resources/ResourceTable";
import { useQueryValue } from "../router";
import { GroupSidebar } from "./GroupSidebar";

export function WorkflowList({ config, workspace, permissions }: {
  config: NonNullable<ReturnType<typeof listConfigs>["workflows"]>;
  workspace: string;
  permissions: Set<string>;
}) {
  useCommonTranslation();
  const [groupId, setGroupId] = useQueryValue("group_id", "");
  const tableConfig = useMemo(() => ({
    ...config,
    fixed: { ...config.fixed, ...(groupId ? { group_id: groupId } : {}) },
    filters: permissions.has("workflow-groups.read")
      ? config.filters?.filter(filter => filter.key !== "group_id") ?? []
      : config.filters ?? [],
  }), [config, groupId, permissions]);
  const select = (id: string) => {
    const query = new URLSearchParams(location.search);
    query.delete("page");
    query.delete("at");
    history.replaceState(history.state, "", `${location.pathname}${query.size ? `?${query}` : ""}`);
    setGroupId(id);
  };
  return <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3 sm:flex-row">
    {permissions.has("workflow-groups.read") && <GroupSidebar workspace={workspace} permissions={permissions} resource="workflow-groups" noun={workflowText("workflows")} active={groupId} onSelect={select} />}
    <div className="flex min-h-0 min-w-0 flex-1 flex-col"><ResourceTable key={groupId} config={tableConfig} workspace={workspace} permissions={permissions} /></div>
  </div>;
}
