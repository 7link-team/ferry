import {
  deleteResource,
} from "../../shared/api/resource";
import { insightText } from "../../shared/i18n/i18n";
import { currentLocale } from "../../shared/i18n/locale";
import type { FormConfig, ListConfig } from "../../shared/resources/types";

export type Row = Record<string, unknown>;

export const time = (value: unknown) =>
  typeof value === "number" && value > 0
    ? new Date(value).toLocaleString(currentLocale())
    : "—";

export const coverage = (value: unknown) =>
  value === 0 ? insightText("完整历史") : time(value);

export const updateMode = (value: unknown) =>
  value === "event" ? insightText("事件更新") : insightText("定时更新");

export function codeInsightList(
  workspace: string,
  connectionId?: string,
): ListConfig<Row> {
  return {
    resource: "code-insights",
    rowKey: (row) => String(row.id),
    ...(connectionId ? { fixed: { connection_id: connectionId } } : {}),
    filters: [
      {
        key: "q",
        label: "名称",
        labelKey: "insight-domain:名称",
        type: "text",
      },
    ],
    columns: [
      { key: "name", label: "洞察名称", labelKey: "insight-domain:洞察名称" },
      {
        key: "repositories",
        label: "仓库数",
        labelKey: "insight-domain:仓库数",
        render: (row) =>
          Array.isArray(row.repositories)
            ? row.repositories.length
            : String(row.repository_count ?? "—"),
      },
      {
        key: "update_mode",
        label: "更新方式",
        labelKey: "insight-domain:更新方式",
        render: (row) => updateMode(row.update_mode),
      },
      {
        key: "enabled",
        label: "持续采集",
        labelKey: "insight-domain:持续采集",
        render: (row) =>
          row.enabled ? insightText("已启用") : insightText("已停用"),
      },
    ],
    actions: [
      {
        label: "查看",
        labelKey: "insight-domain:查看",
        permission: "code-insights.read",
        to: (row) => ({
          group: "repository",
          item: "code-insights",
          mode: "view",
          id: String(row.id),
        }),
        query: () => ({}),
      },
      {
        label: "编辑",
        labelKey: "insight-domain:编辑",
        permission: "code-insights.update",
        to: (row) => ({
          group: "repository",
          item: "code-insights",
          mode: "edit",
          id: String(row.id),
        }),
        query: () => ({}),
      },
      {
        label: "删除",
        labelKey: "insight-domain:删除",
        permission: "code-insights.delete",
        destructive: true,
        action: async (row) => {
          await deleteResource(workspace, "code-insights", String(row.id));
        },
      },
    ],
    toolbar: {
      create: connectionId
        ? false
        : { label: "创建洞察", labelKey: "insight-domain:创建洞察" },
      import: false,
      export: false,
    },
    empty:
      "尚无代码洞察。创建洞察并选择仓库后，即可独立管理作者映射和分析看板。",
    emptyKey:
      "insight-domain:尚无代码洞察。创建洞察并选择仓库后，即可独立管理作者映射和分析看板。",
  };
}

export function codeInsightForm(_workspace: string): FormConfig<Row> {
  return {
    resource: "code-insights",
    width: "full",
    presentation: "sections",
    incompleteMessage: "请先加入至少一个仓库。",
    incompleteMessageKey: "insight-domain:请先加入至少一个仓库。",
    heading: (row) => String(row.name ?? insightText("代码洞察")),
    blank: {
      name: "",
      description: "",
      enabled: true,
      update_mode: "scheduled",
      schedule: "0 */4 * * *",
      timezone: "",
      full_history: false,
      initial_days: 90,
      reconcile_minutes: 1440,
      aliases: [],
      repositories: [],
    },
    pollMs: () => 15000,
    sections: [
      {
        title: "基本信息",
        titleKey: "insight-domain:基本信息",
        fields: [
          {
            key: "name",
            label: "洞察名称",
            labelKey: "insight-domain:洞察名称",
            type: "text",
            required: true,
            span: 2,
          },
        ],
      },
      {
        title: "仓库范围",
        titleKey: "insight-domain:仓库范围",
        fullWidth: true,
        fields: [
          {
            key: "repositories",
            label: "选择仓库",
            labelKey: "insight-domain:选择仓库",
            type: "connection-repositories",
            required: true,
            span: 2,
          },
        ],
      },
      {
        title: "采集策略",
        titleKey: "insight-domain:采集策略",
        fields: [
          {
            key: "update_mode",
            label: "更新方式",
            labelKey: "insight-domain:更新方式",
            type: "select",
            required: true,
            options: [
              {
                value: "scheduled",
                label: "定时更新",
                labelKey: "insight-domain:定时更新",
              },
              {
                value: "event",
                label: "事件更新",
                labelKey: "insight-domain:事件更新",
              },
            ],
            help: "定时更新按计划采集；事件更新在仓库发生变化时采集。",
            helpKey:
              "insight-domain:定时更新按计划采集；事件更新在仓库发生变化时采集。",
          },
          {
            key: "schedule",
            label: "执行周期",
            labelKey: "insight-domain:执行周期",
            type: "cron",
            required: true,
            span: 2,
            when: (row) => row.update_mode === "scheduled",
            help: "默认每 4 小时更新一次。",
            helpKey: "insight-domain:默认每 4 小时更新一次。",
          },
          {
            key: "timezone",
            label: "时区",
            labelKey: "insight-domain:时区",
            type: "text",
            placeholder: "UTC",
            when: (row) => row.update_mode === "scheduled",
            help: "留空按 UTC；Cron 按所选时区执行。可填写 IANA 时区，例如 Asia/Shanghai、Asia/Singapore。",
            helpKey:
              "insight-domain:留空按 UTC；Cron 按所选时区执行。可填写 IANA 时区，例如 Asia/Shanghai、Asia/Singapore。",
          },
          {
            key: "enabled",
            label: "持续采集",
            labelKey: "insight-domain:持续采集",
            type: "switch",
          },
          {
            key: "full_history",
            label: "初始化完整历史",
            labelKey: "insight-domain:初始化完整历史",
            type: "switch",
          },
          {
            key: "initial_days",
            label: "初始化最近天数",
            labelKey: "insight-domain:初始化最近天数",
            type: "number",
            required: true,
            min: 1,
            when: (row) => !row.full_history,
          },
          {
            key: "reconcile_minutes",
            label: "分支头核对间隔（分钟）",
            labelKey: "insight-domain:分支头核对间隔（分钟）",
            type: "number",
            required: true,
            min: 1,
            when: (row) => row.update_mode === "event",
            help: "事件更新的兜底核对，默认每天一次。",
            helpKey: "insight-domain:事件更新的兜底核对，默认每天一次。",
          },
        ],
      },

    ],
  };
}
