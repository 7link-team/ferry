import { activityList } from "../features/activity/resource-config";
import { api } from "../shared/api/api";
import { commonText } from "../shared/i18n/i18n";
import { ResourceTable } from "./resources/ResourceTable";

const base = activityList();
const config = {
  ...base,
  resource: "platform-activity", endpoint: "/platform/monitor/activity", queryPrefix: "platform_activity",
  toolbar: { create: false, import: false, export: false },
  filters: [
    base.filters![0]!,
    { key: "scope", label: "Scope", labelKey: "resource-domain:monitor.activityScope", type: "select" as const, options: [
      { value: "platform", label: "Platform", labelKey: "resource-domain:monitor.activityPlatform" },
      { value: "workspace", label: "Workspace", labelKey: "resource-domain:platform.workspaces" },
    ] },
    { key: "workspace_id", label: "Workspace", labelKey: "resource-domain:platform.workspaces", type: "select" as const, options: async () => {
      const rows: {value:string;label:string}[]=[];
      let page=1;
      for (;;) {
        const result=await api<{items:{id:string;name:string}[];total:number}>(`/platform/workspaces?page=${page}&page_size=200`);
        rows.push(...result.items.map(row=>({value:row.id,label:row.name})));
        if (page*200>=result.total || !result.items.length) return rows;
        page++;
      }
    } },
    { key: "resource", label: "Resource", labelKey: "resource-domain:activity.resource", type: "text" as const },
    { key: "action", label: "Action", labelKey: "resource-domain:resource.action", type: "text" as const },
  ],
  columns: [base.columns[0]!,
    { key: "workspace_name", label: "Scope / workspace", labelKey: "resource-domain:monitor.activityScope", render: (row: Record<string, unknown>) => row.workspace_id === "0" ? commonText("resource-domain:monitor.activityPlatform") : String(row.workspace_name || row.workspace_id) },
    ...base.columns.slice(1),
  ],
};
export function PlatformActivity() {
  return <ResourceTable config={config} workspace="_platform" permissions={new Set()} />;
}
