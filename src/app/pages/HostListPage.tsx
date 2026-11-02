import { useMemo } from "react";
import { commonText,useCommonTranslation } from "../../shared/i18n/i18n";
import { listConfigs } from "../resources";
import { ResourceTable } from "../resources/ResourceTable";
import { useQueryValue } from "../router";
import { GroupSidebar } from "./GroupSidebar";

export function HostList({ config, workspace, permissions }: {
  config: NonNullable<ReturnType<typeof listConfigs>["hosts"]>;
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
    query.delete("page"); query.delete("at");
    history.replaceState(history.state, "", `${location.pathname}${query.size ? `?${query}` : ""}`);
    setGroupId(id);
  };
  return <div className="flex min-h-0 min-w-0 flex-1 flex-col gap-3 sm:flex-row">
    {permissions.has("host-groups.read") && <GroupSidebar workspace={workspace} permissions={permissions} resource="host-groups" noun={commonText("shell.node")} active={groupId} onSelect={select} />}
    <div className="flex min-h-0 min-w-0 flex-1 flex-col"><ResourceTable key={groupId} config={tableConfig} workspace={workspace} permissions={permissions} /></div>
  </div>;
}
