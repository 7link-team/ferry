import { deleteResource } from "../../shared/api/resource";
import { commonText } from "../../shared/i18n/i18n";
import type { ResourceConfigContext } from "../../shared/resources/config";
import {
  type AnyRow,
  cronDate,
  form,
  label,
  list,
  namedOptions,
} from "../../shared/resources/config";
import type { FormConfig, ListConfig } from "../../shared/resources/types";

export interface AlertRule {
  id: string;
  name: string;
  description: string;
  target_kind: string;
  target_ref: string;
  metric: string;
  threshold: number;
  enabled: boolean;
  has_webhook: boolean;
  webhook?: string;
  clear_webhook?: boolean;
  last_error: string;
  version: number;
}

export interface AlertEvent {
  id: string;
  rule_id: string;
  rule_name: string;
  state: string;
  message: string;
  triggered_at: number;
  recovered_at: number;
  trigger_delivery: string;
  recovery_delivery: string;
  version: number;
}

export const alertTargets = [
  { value: "host", label: "主机", labelKey: "common-domain:nav.hosts" },
  {
    value: "pipeline",
    label: "流水线",
    labelKey: "common-domain:shell.pipeline",
  },
  {
    value: "deployment",
    label: "应用",
    labelKey: "common-domain:nav.applications",
  },
  {
    value: "cron-job",
    label: "定时任务",
    labelKey: "common-domain:nav.cronJobs",
  },
];

export const alertMetrics = [
  {
    value: "offline",
    label: "离线",
    labelKey: "common-domain:dashboard.offline",
  },
  {
    value: "load1",
    label: "一分钟负载",
    labelKey: "resource-domain:alert.load",
  },
  {
    value: "cpu-percent",
    label: "CPU 使用率（%）",
    labelKey: "resource-domain:alert.cpu",
  },
  {
    value: "memory-percent",
    label: "内存使用率（%）",
    labelKey: "resource-domain:alert.memory",
  },
  {
    value: "disk-percent",
    label: "磁盘使用率（%）",
    labelKey: "resource-domain:alert.disk",
  },
  {
    value: "execution-failed",
    label: "最近执行失败",
    labelKey: "resource-domain:alert.recentFailure",
  },
];

export const alertEventLink = {
  label: "告警事件",
  labelKey: "common-domain:nav.alertEvents",
  to: () => ({ group: "deploy", item: "alert-events", mode: "list" as const }),
};

export function alertsLists({
  workspace,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    "alert-rules": list<AlertRule>({
      resource: "alert-rules",
      permissionResource: "alerts",
      rowKey: (r) => r.id,
      filters: [
        {
          key: "q",
          label: "名称",
          labelKey: "common-domain:common.name",
          type: "text",
        },
        {
          key: "target_kind",
          label: "目标类型",
          labelKey: "resource-domain:resource.targetType",
          type: "select",
          options: alertTargets,
        },
      ],
      columns: [
        {
          key: "name",
          label: "名称",
          labelKey: "common-domain:common.name",
          sortable: true,
        },
        {
          key: "metric",
          label: "指标",
          labelKey: "resource-domain:alert.metric",
          render: (r) => label(alertMetrics, r.metric),
        },
        {
          key: "threshold",
          label: "阈值",
          labelKey: "resource-domain:alert.threshold",
        },
        {
          key: "enabled",
          label: "启用",
          labelKey: "resource-domain:status.enabled",
          render: (r) =>
            r.enabled
              ? commonText("common-domain:common.yes")
              : commonText("common-domain:common.no"),
        },
        {
          key: "has_webhook",
          label: "Webhook",
          render: (r) =>
            r.has_webhook
              ? commonText("resource-domain:alert.configured")
              : commonText("resource-domain:alert.notConfigured"),
        },
        {
          key: "last_error",
          label: "检查异常",
          labelKey: "resource-domain:alert.checkError",
        },
      ],
      actions: [
        {
          label: "查看",
          labelKey: "common-domain:common.view",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
        {
          label: "编辑",
          labelKey: "common-domain:common.edit",
          permission: "alerts.update",
          to: (r) => ({ id: r.id, mode: "edit" }),
        },
        { ...alertEventLink, query: (r: AlertRule) => ({ rule_id: r.id }) },
        {
          label: "删除",
          labelKey: "common-domain:common.delete",
          permission: "alerts.delete",
          destructive: true,
          action: (r) => deleteResource(workspace, "alert-rules", r.id),
        },
      ],
      toolbar: { create: true, import: false, export: false },
    }),
    "alert-events": list<AlertEvent>({
      resource: "alert-events",
      rowKey: (r) => r.id,
      filters: [
        {
          key: "rule_id",
          label: "规则",
          labelKey: "resource-domain:alert.rule",
          type: "select",
          options: namedOptions(workspace, "alert-rules"),
        },
        {
          key: "state",
          label: "状态",
          labelKey: "resource-domain:status.title",
          type: "select",
          options: [
            {
              value: "firing",
              label: "未恢复",
              labelKey: "common-domain:dashboard.firing",
            },
            {
              value: "recovered",
              label: "已恢复",
              labelKey: "common-domain:dashboard.recovered",
            },
          ],
        },
      ],
      columns: [
        {
          key: "rule_name",
          label: "规则",
          labelKey: "resource-domain:alert.rule",
        },
        {
          key: "state",
          label: "状态",
          labelKey: "resource-domain:status.title",
          render: (r) =>
            r.state === "firing"
              ? commonText("common-domain:dashboard.firing")
              : commonText("common-domain:dashboard.recovered"),
        },
        {
          key: "triggered_at",
          label: "触发时间",
          labelKey: "resource-domain:alert.triggeredAt",
          sortable: true,
          render: (r) => cronDate(r.triggered_at),
        },
        {
          key: "recovered_at",
          label: "恢复时间",
          labelKey: "resource-domain:alert.recoveredAt",
          render: (r) => cronDate(r.recovered_at),
        },
        {
          key: "trigger_delivery",
          label: "触发通知",
          labelKey: "resource-domain:alert.triggerDelivery",
        },
        {
          key: "recovery_delivery",
          label: "恢复通知",
          labelKey: "resource-domain:alert.recoveryDelivery",
        },
      ],
      actions: [
        {
          label: "查看",
          labelKey: "common-domain:common.view",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
      ],
      toolbar: { create: false, import: false, export: false },
    }),
  };
}

export function alertsForms({
  workspace,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  return {
    "alert-rules": form<AlertRule>({
      resource: "alert-rules",
      permissionResource: "alerts",
      heading: (r) => r.name,
      blank: {
        name: "",
        description: "",
        target_kind: "host",
        target_ref: "",
        metric: "offline",
        threshold: 0,
        enabled: false,
        webhook: "",
        clear_webhook: false,
      },
      viewActions: [alertEventLink],
      sections: [
        {
          fields: [
            {
              key: "name",
              label: "名称",
              labelKey: "common-domain:common.name",
              type: "text",
              required: true,
            },
            {
              key: "enabled",
              label: "启用",
              labelKey: "resource-domain:status.enabled",
              type: "switch",
            },
            {
              key: "target_kind",
              label: "目标类型",
              labelKey: "resource-domain:resource.targetType",
              type: "select",
              required: true,
              immutable: true,
              options: alertTargets,
              resetOnChange: ["target_ref", "metric"],
            },
            ...(
              [
                ["host", "hosts", "主机"],
                ["pipeline", "pipelines", "流水线"],
                ["deployment", "deployments", "应用"],
                ["cron-job", "cron-jobs", "定时任务"],
              ] as const
            ).map(([kind, resource, label]) => ({
              key: "target_ref",
              label: `目标${label}`,
              labelKey: `resource-domain:target.${kind}`,
              type: "select" as const,
              required: true,
              immutable: true,
              when: (d: Partial<AlertRule>) => d.target_kind === kind,
              options: namedOptions(workspace, resource),
            })),
            {
              key: "metric",
              label: "指标",
              labelKey: "resource-domain:alert.metric",
              type: "select",
              required: true,
              immutable: true,
              when: (d) => d.target_kind === "host",
              options: alertMetrics.filter(
                (m) => m.value !== "execution-failed",
              ),
            },
            {
              key: "metric",
              label: "指标",
              labelKey: "resource-domain:alert.metric",
              type: "select",
              required: true,
              immutable: true,
              when: (d) => d.target_kind !== "host",
              options: alertMetrics.filter(
                (m) => m.value === "execution-failed",
              ),
            },
            {
              key: "threshold",
              label: "阈值",
              labelKey: "resource-domain:alert.threshold",
              type: "number",
              required: true,
              when: (d) =>
                !!d.metric &&
                !["offline", "execution-failed"].includes(d.metric),
              help: "指标严格大于阈值时触发；内存和磁盘范围0–100，负载为非负整数。",
              helpKey: "resource-domain:alert.thresholdHelp",
            },
            {
              key: "webhook",
              label: "Webhook 地址",
              labelKey: "resource-domain:alert.webhook",
              type: "password",
              stored: "has_webhook",
              help: "可选HTTP(S)地址；加密保存、不回显。编辑留空保留原地址。触发和恢复各尝试一次，失败不自动重试。",
              helpKey: "resource-domain:alert.webhookHelp",
            },
            {
              key: "clear_webhook",
              label: "清除已有 Webhook",
              labelKey: "resource-domain:alert.clearWebhook",
              type: "switch",
              when: (d) => !!d.has_webhook,
            },
            {
              key: "description",
              label: "说明",
              labelKey: "common-domain:common.description",
              type: "textarea",
              span: 2,
            },
            {
              key: "last_error",
              label: "检查异常",
              labelKey: "resource-domain:alert.checkError",
              type: "text",
              immutable: true,
              when: (d) => !!d.last_error,
              help: "检查执行身份的账号权限和目标是否仍然有效。",
              helpKey: "resource-domain:alert.identityHelp",
            },
          ],
        },
      ],
    }),
    "alert-events": form<AlertEvent>({
      resource: "alert-events",
      heading: (r) => r.rule_name,
      blank: {},
      readOnly: () => true,
      pollMs: (r) =>
        r.state === "firing" ||
        r.trigger_delivery === "投递中" ||
        r.recovery_delivery === "投递中"
          ? 2000
          : 0,
      viewActions: [
        {
          label: "返回规则",
          labelKey: "resource-domain:alert.backRule",
          to: () => ({ group: "deploy", item: "alerts", mode: "list" }),
        },
      ],
      sections: [
        {
          fields: [
            {
              key: "rule_name",
              label: "规则",
              labelKey: "resource-domain:alert.rule",
              type: "text",
            },
            {
              key: "state",
              label: "状态",
              labelKey: "resource-domain:status.title",
              type: "select",
              options: [
                {
                  value: "firing",
                  label: "未恢复",
                  labelKey: "common-domain:dashboard.firing",
                },
                {
                  value: "recovered",
                  label: "已恢复",
                  labelKey: "common-domain:dashboard.recovered",
                },
              ],
            },
            {
              key: "message",
              label: "触发信息",
              labelKey: "resource-domain:alert.message",
              type: "textarea",
              span: 2,
            },
            {
              key: "trigger_delivery",
              label: "触发通知",
              labelKey: "resource-domain:alert.triggerDelivery",
              type: "text",
            },
            {
              key: "recovery_delivery",
              label: "恢复通知",
              labelKey: "resource-domain:alert.recoveryDelivery",
              type: "text",
            },
          ],
        },
      ],
    }),
  };
}
