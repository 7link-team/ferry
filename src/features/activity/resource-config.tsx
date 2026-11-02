import { currentLocale } from "../../shared/i18n/locale";
import type { ResourceConfigContext } from "../../shared/resources/config";
import { type AnyRow, list } from "../../shared/resources/config";
import type { ListConfig } from "../../shared/resources/types";

export interface Activity {
  id: string;
  actor_name: string;
  action: string;
  resource: string;
  resource_id: string;
  details: unknown;
  created_at: number;
}

export function activityLists({}: ResourceConfigContext): Record<
  string,
  ListConfig<AnyRow>
> {
  return {
    activity: list({
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
        },
        {
          key: "resource",
          label: "资源",
          labelKey: "resource-domain:activity.resource",
          width: 140,
        },
        {
          key: "details",
          label: "详情",
          labelKey: "common-domain:common.details",
          render: (row: Activity) => (
            <code className="text-xs text-muted-foreground">
              {JSON.stringify(row.details)}
            </code>
          ),
        },
      ],
      toolbar: { create: false, import: false },
      empty: "还没有操作记录。",
      emptyKey: "resource-domain:activity.empty",
    }),
  };
}
