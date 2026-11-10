import { deleteResource, runItemAction } from "../../shared/api/resource";
import { commonText } from "../../shared/i18n/i18n";
import type { ResourceConfigContext } from "../../shared/resources/config";
import {
  type AnyRow,
  cronDate,
  form,
  list,
  namedOptions,
} from "../../shared/resources/config";
import type { FormConfig, ListConfig } from "../../shared/resources/types";
import { TARGETS } from "../hosts/resource-config";

export interface CronJob {
  id: string;
  name: string;
  description: string;
  scope: string;
  schedule: string;
  timezone: string;
  enabled: boolean;
  action: string;
  target_kind: string;
  target_ref: string;
  environment_id: string;
  command: string;
  timeout_seconds: number;
  max_retries: number;
  retry_delay_seconds: number;
  next_run_at: number;
  run_as_name: string;
  version: number;
}

export interface CronRun {
  id: string;
  job_id: string;
  job_name: string;
  scope: string;
  state: string;
  scheduled_at: number;
  started_at: number;
  finished_at: number;
  attempt: number;
  output: string;
  config: string;
  downstream_id: string;
  downstream_kind: string;
  version: number;
  trigger_kind: string;
}

export const cronState = (state: string) =>
  ({
    queued: commonText("common-domain:dashboard.queued"),
    running: commonText("resource-domain:cron.running"),
    succeeded: commonText("common-domain:dashboard.succeeded"),
    failed: commonText("common-domain:dashboard.failed"),
    skipped: commonText("common-domain:dashboard.skipped"),
    "needs-review": commonText("resource-domain:cron.needsReview"),
  })[state] ?? state;

export const cronReview = (workspace: string) => async (row: CronRun) => {
  const note = prompt(commonText("resource-domain:cron.unblockHelp"));
  if (note === null) return;
  await runItemAction(workspace, "cron-runs", row.id, "acknowledge", {
    version: row.version,
    note,
  });
};

export const cronLinks = (workspace: string) => [
  {
    label: "返回任务",
    labelKey: "resource-domain:cron.backTask",
    to: (r: CronRun) => ({
      group: r.scope,
      item: "cron-jobs",
      id: r.job_id,
      mode: "view" as const,
    }),
  },
  {
    label: "查看下游执行",
    labelKey: "resource-domain:cron.downstream",
    when: (r: CronRun) => r.downstream_id !== "0",
    to: (r: CronRun) => ({
      group: r.downstream_kind === "pipeline-runs" ? "build" : "deploy",
      item: r.downstream_kind === "pipeline-runs" ? "runs" : "revisions",
      id: r.downstream_id,
      mode: "view" as const,
    }),
  },
  {
    label: "核实并解除阻塞",
    labelKey: "resource-domain:cron.verifyUnblock",
    permission: "cron-jobs.run",
    when: (r: CronRun) => r.state === "needs-review",
    action: cronReview(workspace),
  },
];

export function schedulingLists({
  workspace,
  navigate,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    "cron-jobs": list<CronJob>({
      resource: "cron-jobs",
      rowKey: (r) => r.id,
      filters: [
        {
          key: "q",
          label: "名称 / 说明",
          labelKey: "resource-domain:resource.nameDescription",
          type: "text",
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
          key: "schedule",
          label: "Cron 表达式",
          labelKey: "resource-domain:cron.expression",
        },
        {
          key: "timezone",
          label: "时区",
          labelKey: "resource-domain:cron.timezone",
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
          key: "next_run_at",
          label: "下次执行",
          labelKey: "resource-domain:cron.nextRun",
          sortable: true,
          render: (r) => cronDate(r.next_run_at),
        },
        {
          key: "run_as_name",
          label: "执行身份",
          labelKey: "resource-domain:cron.runAs",
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
          permission: "cron-jobs.update",
          to: (r) => ({ id: r.id, mode: "edit" }),
        },
        {
          label: "立即执行",
          labelKey: "resource-domain:cron.runNow",
          permission: "cron-jobs.run",
          action: async (r) => {
            const result = await runItemAction<{ id: string }>(
              workspace,
              "cron-jobs",
              r.id,
              "run-now",
              {},
            );
            navigate(
              {
                group: r.scope,
                item: "cron-runs",
                id: result.id,
                mode: "view",
              },
              {},
            );
          },
        },
        {
          label: "执行历史",
          labelKey: "common-domain:nav.runHistory",
          to: (r) => ({ group: r.scope, item: "cron-runs", mode: "list" }),
          query: (r) => ({ job_id: r.id }),
        },
        {
          label: "删除",
          labelKey: "common-domain:common.delete",
          permission: "cron-jobs.delete",
          destructive: true,
          action: (r) => deleteResource(workspace, "cron-jobs", r.id),
        },
      ],
      toolbar: { import: false, export: false },
    }),
    "cron-runs": list<CronRun>({
      resource: "cron-runs",
      rowKey: (r) => r.id,
      filters: [
        {
          key: "job_id",
          label: "任务",
          labelKey: "resource-domain:cron.task",
          type: "select",
          options: namedOptions(workspace, "cron-jobs"),
        },
        {
          key: "state",
          label: "状态",
          labelKey: "resource-domain:status.title",
          type: "select",
          options: [
            "queued",
            "running",
            "succeeded",
            "failed",
            "skipped",
            "needs-review",
          ].map((value) => ({
            value,
            get label() {
              return cronState(value);
            },
          })),
        },
      ],
      columns: [
        {
          key: "job_name",
          label: "任务",
          labelKey: "resource-domain:cron.task",
        },
        {
          key: "state",
          label: "状态",
          labelKey: "resource-domain:status.title",
          render: (r) => cronState(r.state),
        },
        {
          key: "scheduled_at",
          label: "计划时间",
          labelKey: "resource-domain:cron.scheduledAt",
          sortable: true,
          render: (r) => cronDate(r.scheduled_at),
        },
        {
          key: "attempt",
          label: "尝试次数",
          labelKey: "resource-domain:cron.attempt",
        },
        {
          key: "trigger_kind",
          label: "触发方式",
          labelKey: "resource-domain:cron.triggerType",
          render: (r) =>
            r.trigger_kind === "manual"
              ? commonText("resource-domain:cron.manual")
              : commonText("resource-domain:cron.scheduled"),
        },
      ],
      actions: [
        {
          label: "查看",
          labelKey: "common-domain:common.view",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
        ...cronLinks(workspace),
      ],
      toolbar: { create: false, import: false, export: false },
    }),
  };
}

export function schedulingForms({
  workspace,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  return {
    "cron-jobs": form<CronJob>({
      resource: "cron-jobs",
      heading: (r) => r.name,
      blank: {
        name: "",
        description: "",
        scope: "host",
        schedule: "0 * * * *",
        timezone: "",
        enabled: false,
        action: "",
        target_kind: "",
        target_ref: "",
        command: "",
        timeout_seconds: 1800,
        max_retries: 0,
        retry_delay_seconds: 30,
      },
      viewActions: [
        {
          label: "执行历史",
          labelKey: "common-domain:nav.runHistory",
          to: (r) => ({ group: r.scope, item: "cron-runs", mode: "list" }),
          query: (r) => ({ job_id: r.id }),
        },
      ],
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
              key: "scope",
              label: "所属模块",
              labelKey: "resource-domain:cron.module",
              type: "select",
              resetOnChange: ["action", "target_kind", "target_ref", "command"],
              options: [
                {
                  value: "host",
                  label: "主机",
                  labelKey: "common-domain:nav.hosts",
                },
                {
                  value: "build",
                  label: "构建",
                  labelKey: "common-domain:nav.build",
                },
                {
                  value: "deploy",
                  label: "部署",
                  labelKey: "resource-domain:variable.deploy",
                },
              ],
              immutable: true,
            },
            {
              key: "enabled",
              label: "启用定时执行",
              labelKey: "resource-domain:cron.enable",
              type: "switch",
            },
            {
              key: "timezone",
              label: "时区",
              labelKey: "resource-domain:cron.timezone",
              type: "text",
              required: true,
              placeholder: "Asia/Shanghai",
              help: "必须明确填写 IANA 时区，例如 Asia/Shanghai、Asia/Singapore、UTC。",
              helpKey: "resource-domain:cron.timezoneHelp",
            },
            {
              key: "schedule",
              label: "执行周期",
              labelKey: "resource-domain:cron.schedule",
              type: "cron",
              required: true,
              span: 2,
              help: "停机错过的触发不补跑，同任务重叠时跳过并记录。",
              helpKey: "resource-domain:cron.scheduleHelp",
            },
            {
              key: "action",
              label: "动作",
              labelKey: "resource-domain:resource.action",
              type: "select",
              required: true,
              when: (d) => d.scope === "host",
              options: [
                {
                  value: "command",
                  label: "执行命令",
                  labelKey: "resource-domain:cron.commandAction",
                },
              ],
            },
            {
              key: "action",
              label: "动作",
              labelKey: "resource-domain:resource.action",
              type: "select",
              required: true,
              when: (d) => d.scope === "build",
              options: [
                {
                  value: "pipeline",
                  label: "触发流水线",
                  labelKey: "resource-domain:cron.pipelineAction",
                },
              ],
            },
            {
              key: "action",
              label: "动作",
              labelKey: "resource-domain:resource.action",
              type: "select",
              required: true,
              when: (d) => d.scope === "deploy",
              options: [
                {
                  value: "redeploy",
                  label: "重新部署当前配置",
                  labelKey: "resource-domain:cron.redeployAction",
                },
                {
                  value: "restart",
                  label: "重启当前成功版本",
                  labelKey: "resource-domain:cron.restartAction",
                },
                {
                  value: "sync-marketplace",
                  label: "同步市场目录",
                  labelKey: "resource-domain:cron.syncAction",
                },
              ],
            },
            {
              key: "target_kind",
              label: "目标类型",
              labelKey: "resource-domain:resource.targetType",
              type: "select",
              required: true,
              resetOnChange: ["target_ref"],
              when: (d) => d.scope === "host",
              options: TARGETS,
            },
            {
              key: "target_kind",
              label: "目标类型",
              labelKey: "resource-domain:resource.targetType",
              type: "select",
              required: true,
              when: (d) => d.scope === "build",
              options: [
                {
                  value: "pipeline",
                  label: "流水线",
                  labelKey: "common-domain:shell.pipeline",
                },
              ],
            },
            {
              key: "target_kind",
              label: "目标类型",
              labelKey: "resource-domain:resource.targetType",
              type: "select",
              required: true,
              when: (d) =>
                d.scope === "deploy" && d.action !== "sync-marketplace",
              options: [
                {
                  value: "deployment",
                  label: "应用",
                  labelKey: "common-domain:nav.applications",
                },
              ],
            },
            {
              key: "target_kind",
              label: "目标类型",
              labelKey: "resource-domain:resource.targetType",
              type: "select",
              required: true,
              when: (d) => d.action === "sync-marketplace",
              options: [
                {
                  value: "marketplace-source",
                  label: "市场来源",
                  labelKey: "common-domain:nav.marketplaceSources",
                },
              ],
            },
            {
              key: "target_ref",
              label: "市场来源",
              labelKey: "common-domain:nav.marketplaceSources",
              type: "select",
              required: true,
              when: (d) => d.target_kind === "marketplace-source",
              options: namedOptions(workspace, "marketplace-sources"),
            },
            ...(
              [
                ["host", "hosts", "主机"],
                ["host-group", "host-groups", "主机组"],
                ["pipeline", "pipelines", "流水线"],
                ["deployment", "deployments", "应用"],
              ] as const
            ).map(([kind, resource, label]) => ({
              key: "target_ref",
              label: `目标${label}`,
              labelKey: `resource-domain:target.${kind}`,
              type: "select" as const,
              required: true,
              when: (d: Partial<CronJob>) => d.target_kind === kind,
              options: namedOptions(workspace, resource),
            })),
            {
              key: "environment_id",
              label: "执行环境",
              labelKey: "resource-domain:pipeline.environment",
              type: "select",
              emptyValues: ["0", 0],
              emptyLabelKey: "resource-domain:reference.noEnvironment",
              options: namedOptions(workspace, "pipeline-environments"),
              when: (d) => d.scope === "build",
              help: "流水线绑定环境时必须选择其中一个；不绑定环境时留空。",
              helpKey: "resource-domain:cron.environmentHelp",
            },
            {
              key: "command",
              label: "命令",
              labelKey: "resource-domain:cron.command",
              type: "code",
              span: 2,
              required: true,
              when: (d) => d.scope === "host",
              help: "通过所选主机通道执行；主机组逐台执行，失败即停。不要把密钥写进脚本。",
              helpKey: "resource-domain:cron.commandHelp",
            },
            {
              key: "timeout_seconds",
              label: "超时（秒）",
              labelKey: "resource-domain:cron.timeout",
              type: "number",
              required: true,
              help: "1–86400；下游执行等待超时不会强制停止远端，会标记需核实。",
              helpKey: "resource-domain:cron.timeoutHelp",
            },
            {
              key: "max_retries",
              label: "失败重试次数",
              labelKey: "resource-domain:cron.retries",
              type: "number",
              required: true,
              help: "默认0；仅对确定失败重试，最多10次。命令必须允许重复执行。",
              helpKey: "resource-domain:cron.retriesHelp",
            },
            {
              key: "retry_delay_seconds",
              label: "重试间隔（秒）",
              labelKey: "resource-domain:cron.retryDelay",
              type: "number",
              required: true,
            },
            {
              key: "run_as_name",
              label: "执行身份",
              labelKey: "resource-domain:cron.runAs",
              type: "text",
              immutable: true,
              when: (d) => !!d.run_as_name,
              help: "保存后使用保存者身份；每次执行重新核实账号和权限。",
              helpKey: "resource-domain:cron.runAsHelp",
            },
            {
              key: "description",
              label: "说明",
              labelKey: "common-domain:common.description",
              type: "textarea",
              span: 2,
            },
          ],
        },
      ],
    }),
    "cron-runs": form<CronRun>({
      resource: "cron-runs",
      heading: (r) => r.job_name,
      blank: {},
      readOnly: () => true,
      pollMs: (r) => (r.state === "queued" || r.state === "running" ? 1000 : 0),
      viewActions: cronLinks(workspace),
      sections: [
        {
          fields: [
            {
              key: "job_name",
              label: "任务",
              labelKey: "resource-domain:cron.task",
              type: "text",
            },
            {
              key: "state",
              label: "状态",
              labelKey: "resource-domain:status.title",
              type: "select",
              options: [
                "queued",
                "running",
                "succeeded",
                "failed",
                "skipped",
                "needs-review",
              ].map((value) => ({
                value,
                get label() {
                  return cronState(value);
                },
              })),
            },
            {
              key: "attempt",
              label: "尝试次数",
              labelKey: "resource-domain:cron.attempt",
              type: "number",
            },
            {
              key: "downstream_id",
              label: "下游执行 ID",
              labelKey: "resource-domain:cron.downstreamId",
              type: "text",
            },
            {
              key: "output",
              label: "执行输出",
              labelKey: "resource-domain:cron.output",
              type: "code",
              span: 2,
            },
            {
              key: "config",
              label: "本次配置快照",
              labelKey: "resource-domain:cron.snapshot",
              type: "code",
              span: 2,
            },
          ],
        },
      ],
    }),
  };
}
