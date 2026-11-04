import { pipelineExecutorOptions } from "./executor-options";
import { TagBadge } from "../tags/TagBadge";
import {
  createResource,
  deleteResource,
  named,
  runItemAction,
} from "../../shared/api/resource";
import { commonText } from "../../shared/i18n/i18n";
import { currentLocale } from "../../shared/i18n/locale";
import type { ResourceConfigContext } from "../../shared/resources/config";
import {
  form,
  label,
  list,
  namedOptions,
  dictionaryOptions,
  type AnyRow,
} from "../../shared/resources/config";
import type {
  FormConfig,
  ListConfig,
  RowAction,
} from "../../shared/resources/types";
import type { Navigation } from "../../shared/routing/types";
import { Badge } from "../../shared/ui/badge";
import { TARGETS } from "../hosts/resource-config";
import { tagOptions } from "../tags/resource-config";
import { templateGenerator } from "../templates/resource-config";
import { DEFAULT_ACTIONS, dispatchInputs } from "./actions-workflow";
import { duration, stateBadge, type PipelineRun } from "./pipeline";

export interface Pipeline {
  id: string;
  name: string;
  repository_id: string;
  target_kind: string;
  group_id: string;
  group_name: string;
  application_id: string;
  application_name: string;
  tag_ids: string[];
  environment_ids: string[];
  tags: { id: string; name: string; color?: string }[];
  environments: { id: string; name: string }[];
  target_ref: string;
  definition: string;
  description: string;
  definition_format: string;
  executor_bindings: unknown[];
  checkout_bindings: unknown[];
  graph_layout: Record<string, unknown>;
}

export interface PipelineContext {
  id: string;
  name: string;
  description: string;
  target_kind: string;
  target_ref: string;
  requires_approval?: boolean;
  version: number;
}

export const inputsOf = (row: Pipeline) =>
  row.definition_format === "github-actions"
    ? dispatchInputs(row.definition)
    : [];

export const pipelineRunAction = (
  workspace: string,
  navigate: Navigation["navigate"],
): RowAction<Pipeline> => ({
  label: "触发",
  labelKey: "resource-domain:pipeline.trigger",
  permission: "pipelines.run",
  parameters: (row) => [
    ...(row.environments?.length
      ? [
          {
            key: "environment_id",
            label: "执行环境",
            labelKey: "resource-domain:pipeline.environment",
            type: "select" as const,
            required: true,
            options: row.environments.map((environment) => ({
              value: environment.id,
              label: environment.name,
            })),
            help: "环境默认执行器替代流水线默认值；任务单独指定的执行器优先。环境变量覆盖流水线默认变量。",
            helpKey: "resource-domain:pipeline.environmentHelp",
          },
        ]
      : []),
    ...inputsOf(row).map((input) => ({
      key: `input:${input.name}`,
      label: input.name,
      required: input.required && input.type !== "boolean",
      ...(input.description ? { help: input.description } : {}),
      ...(input.type === "boolean"
        ? { type: "switch" as const }
        : input.type === "choice"
          ? {
              type: "select" as const,
              options: input.options.map((option) => ({
                value: option,
                label: option,
              })),
            }
          : input.type === "number"
            ? { type: "number" as const }
            : { type: "text" as const }),
    })),
  ],
  initialValues: (row) =>
    Object.fromEntries(
      inputsOf(row)
        .filter((input) => input.default !== undefined)
        .map((input) => [
          `input:${input.name}`,
          input.type === "boolean"
            ? input.default === true || input.default === "true"
            : input.default,
        ]),
    ),
  action: async (row, values) => {
    const body: Record<string, unknown> = {},
      inputs: Record<string, unknown> = {};
    for (const [key, value] of Object.entries(values ?? {})) {
      if (key.startsWith("input:")) {
        if (value !== "" && value !== undefined) inputs[key.slice(6)] = value;
      } else body[key] = value;
    }
    if (Object.keys(inputs).length) body.inputs = inputs;
    const run = await runItemAction<{ id: string }>(
      workspace,
      "pipelines",
      row.id,
      "run",
      body,
    );
    navigate(
      { workspace, group: "build", item: "runs", id: run.id, mode: "view" },
      {},
    );
  },
});

export function pipelinesLists({
  workspace,
  permissions,
  navigate,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    ...Object.fromEntries(
      (["pipeline-groups", "pipeline-environments"] as const).map(
        (resource) => [
          resource,
          list<PipelineContext>({
            resource,
            rowKey: (row) => row.id,
            filters: [
              {
                key: "q",
                label: "名称",
                labelKey: "common-domain:common.name",
                type: "text",
              },
            ],
            columns: [
              {
                key: "name",
                label: "名称",
                labelKey: "common-domain:common.name",
              },
              {
                key: "description",
                label: "说明",
                labelKey: "common-domain:common.description",
              },
            ],
            actions: [
              {
                label: "查看",
                labelKey: "common-domain:common.view",
                permission: resource + ".read",
                to: (row) => ({ id: row.id, mode: "view" }),
              },
              {
                label: "编辑",
                labelKey: "common-domain:common.edit",
                permission: resource + ".update",
                to: (row) => ({ id: row.id, mode: "edit" }),
              },
              {
                label: "删除",
                labelKey: "common-domain:common.delete",
                permission: resource + ".delete",
                destructive: true,
                action: (row) => deleteResource(workspace, resource, row.id),
              },
            ],
          }),
        ],
      ),
    ),
    pipelines: list({
      resource: "pipelines",
      rowKey: (r: Pipeline) => r.id,
      toolbar: {
        create: {
          label: "创建流水线",
          labelKey: "resource-domain:pipeline.create",
        },
      },
      filters: [
        {
          key: "q",
          label: "名称",
          labelKey: "common-domain:common.name",
          type: "text",
        },
        ...(!permissions || permissions.has("pipeline-groups.read")
          ? [
              {
                key: "group_id",
                label: "分组",
                labelKey: "common-domain:table.groups",
                type: "select" as const,
                options: namedOptions(workspace, "pipeline-groups"),
              },
            ]
          : []),
        ...(!permissions || permissions.has("deployments.read")
          ? [
              {
                key: "application_id",
                label: "应用",
                labelKey: "common-domain:nav.applications",
                type: "select" as const,
                options: namedOptions(workspace, "deployments"),
              },
            ]
          : []),
        ...(!permissions || permissions.has("tags.read")
          ? [
              {
                key: "tag_id",
                label: "标签",
                labelKey: "common-domain:nav.tags",
                type: "select" as const,
                options: tagOptions(workspace),
              },
            ]
          : []),
      ],
      columns: [
        { key: "name", label: "名称", labelKey: "common-domain:common.name" },
        {
          key: "group_name",
          label: "分组",
          labelKey: "common-domain:table.groups",
        },
        {
          key: "application_name",
          label: "应用",
          labelKey: "common-domain:nav.applications",
        },
        {
          key: "tags",
          label: "标签",
          labelKey: "common-domain:nav.tags",
          render: (row) => (
            <span className="flex flex-wrap gap-1">
              {row.tags?.map((tag) => (
                <TagBadge color={tag.color} key={tag.id}>
                  {tag.name}
                </TagBadge>
              ))}
            </span>
          ),
        },
        {
          key: "repository_name",
          label: "仓库",
          labelKey: "common-domain:dashboard.repositories",
          render: (r: AnyRow) =>
            named(r.repository_name as string, r.repository_id as string),
        },
        {
          key: "target_kind",
          label: "目标类型",
          labelKey: "resource-domain:resource.targetType",
          render: (r) => label(TARGETS, r.target_kind),
        },
        {
          key: "target_name",
          label: "目标",
          labelKey: "resource-domain:resource.target",
          render: (r: AnyRow) =>
            named(r.target_name as string, r.target_ref as string),
        },
        {
          key: "description",
          label: "备注",
          labelKey: "resource-domain:resource.notes",
        },
      ],
      actions: [
        {
          label: "查看",
          labelKey: "common-domain:common.view",
          permission: "pipelines.read",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
        pipelineRunAction(workspace, navigate),
        {
          label: "编辑",
          labelKey: "common-domain:common.edit",
          permission: "pipelines.update",
          to: (r) => ({ id: r.id, mode: "edit" }),
        },
        {
          label: "删除",
          labelKey: "common-domain:common.delete",
          permission: "pipelines.delete",
          destructive: true,
          action: (r) => deleteResource(workspace, "pipelines", r.id),
        },
      ],
    }),
    "pipeline-artifacts": list({
      resource: "pipeline-artifacts",
      permissionResource: "pipelines",
      rowKey: (r: { id: string }) => r.id,
      filters: [
        {
          key: "q",
          label: "文件路径",
          labelKey: "resource-domain:artifact.path",
          type: "text",
        },
        {
          key: "run_id",
          label: "执行 ID",
          labelKey: "resource-domain:execution.id",
          type: "text",
        },
        {
          key: "pipeline_id",
          label: "流水线",
          labelKey: "common-domain:shell.pipeline",
          type: "select",
          options: namedOptions(workspace, "pipelines"),
        },
      ],
      columns: [
        {
          key: "path",
          label: "文件路径",
          labelKey: "resource-domain:artifact.path",
        },
        {
          key: "run_id",
          label: "执行",
          labelKey: "resource-domain:execution.title",
          render: (r: AnyRow) =>
            r.run_number
              ? `${named(r.pipeline_name as string, r.pipeline_id as string)} #${r.run_number}`
              : named("", r.run_id as string),
        },
        {
          key: "size",
          label: "大小（字节）",
          labelKey: "resource-domain:artifact.bytes",
          sortable: true,
        },
        { key: "sha256", label: "SHA-256" },
        {
          key: "download",
          label: "下载",
          labelKey: "resource-domain:artifact.download",
          render: (r: { id: string; path: string }) => (
            <a
              className="text-primary underline"
              href={`/api/workspaces/${encodeURIComponent(workspace)}/pipeline-artifacts/${r.id}/content`}
              download
            >
              {commonText("resource-domain:artifact.download")}
              {r.path}
            </a>
          ),
        },
      ],
      toolbar: { create: false, import: false, export: false },
      empty:
        "还没有构建产物。在流水线定义的 artifacts 中声明相对文件路径，构建成功后会自动归档。",
      emptyKey: "resource-domain:artifact.empty",
    }),
    "pipeline-runs": list({
      resource: "pipeline-runs",
      rowKey: (r: PipelineRun) => r.id,
      filters: [
        {
          key: "state",
          label: "状态",
          labelKey: "resource-domain:status.title",
          type: "select",
          options: [
            "queued",
            "running",
            "cancelling",
            "succeeded",
            "failed",
            "cancelled",
          ].map((value) => ({
            value,
            get label() {
              return stateBadge(value).label;
            },
          })),
        },
      ],
      columns: [
        {
          key: "number",
          label: "执行序号",
          labelKey: "resource-domain:execution.number",
          sortable: true,
          render: (r) => `#${r.number}`,
        },
        {
          key: "pipeline_name",
          label: "流水线",
          labelKey: "common-domain:shell.pipeline",
          render: (r) => named(r.pipeline_name, r.pipeline_id),
        },
        {
          key: "state",
          label: "状态",
          labelKey: "resource-domain:status.title",
          render: (r) => {
            const { label, ...props } = stateBadge(r.state);
            return <Badge {...props}>{label}</Badge>;
          },
        },
        {
          key: "triggered_by_name",
          label: "触发人",
          labelKey: "resource-domain:execution.triggeredBy",
          render: (r) => named(r.triggered_by_name, r.triggered_by),
        },
        {
          key: "duration",
          label: "耗时",
          labelKey: "resource-domain:execution.duration",
          render: (r) => duration(r.started_at, r.finished_at),
        },
        {
          key: "started_at",
          label: "开始时间",
          labelKey: "resource-domain:execution.startedAt",
          render: (r) =>
            r.started_at
              ? new Date(r.started_at).toLocaleString(currentLocale())
              : "—",
        },
      ],
      actions: [
        {
          label: "查看",
          labelKey: "common-domain:common.view",
          permission: "pipelines.read",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
      ],
      toolbar: { create: false, import: false, export: false },
    }),
  };
}

export function pipelinesForms({
  workspace,
  permissions,
  navigate,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  return {
    "pipeline-groups": form<PipelineContext>({
      resource: "pipeline-groups",
      heading: (row) => row.name,
      blank: { name: "", description: "" },
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
              key: "description",
              label: "说明",
              labelKey: "common-domain:common.description",
              type: "textarea",
            },
          ],
        },
      ],
    }),
    "pipeline-environments": form<PipelineContext>({
      resource: "pipeline-environments",
      heading: (row) => row.name,
      blank: { name: "", description: "", target_kind: "host", target_ref: "" },
      viewActions: [
        {
          label: "管理环境变量",
          labelKey: "resource-domain:pipeline.manageVariables",
          permission: "variables.read",
          to: () => ({
            group: "build",
            item: "environment-variables",
            mode: "list",
            id: "",
          }),
          query: (row) => ({ scope_ref: row.id }),
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
              options: dictionaryOptions(workspace, "environment"),
              required: true,
            },
            {
              key: "description",
              label: "说明",
              labelKey: "common-domain:common.description",
              type: "textarea",
            },
            {
              key: "target_kind",
              label: "默认执行器类型",
              labelKey: "resource-domain:pipeline.defaultExecutor",
              type: "select",
              options: TARGETS,
              resetOnChange: ["target_ref"],
            },
            {
              key: "target_ref",
              label: "默认主机",
              labelKey: "resource-domain:pipeline.defaultHost",
              type: "select",
              required: true,
              options: namedOptions(workspace, "hosts"),
              when: (row) => row.target_kind === "host",
            },
            {
              key: "target_ref",
              label: "默认主机组",
              labelKey: "resource-domain:pipeline.defaultGroup",
              type: "select",
              required: true,
              options: namedOptions(workspace, "host-groups"),
              when: (row) => row.target_kind === "host-group",
            },
            {
              key: "requires_approval",
              label: "需要审批",
              labelKey: "resource-domain:pipeline.approval",
              type: "switch",
              help: "Actions 任务声明 environment 为此环境名称时，开始执行前需要有执行权限的成员批准（相当于 GitHub 环境的必需审阅者）。",
              helpKey: "resource-domain:pipeline.approvalHelp",
            },
          ],
        },
      ],
    }),
    pipelines: form({
      ...(permissions?.has("pipeline-templates.read")
        ? {
            generator: templateGenerator(
              workspace,
              "pipeline-templates",
              "definition",
            ),
          }
        : {}),
      resource: "pipelines",
      width: "full",
      heading: (r: Pipeline) => r.name,
      viewActions: [pipelineRunAction(workspace, navigate)],
      blank: {
        group_id: "",
        application_id: "",
        tag_ids: [],
        environment_ids: [],
        name: "",
        repository_id: "",
        target_kind: "host",
        target_ref: "",
        description: "",
        definition_format: "github-actions",
        executor_bindings: [],
        checkout_bindings: [],
        graph_layout: {},
        definition: DEFAULT_ACTIONS,
      },
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
              key: "group_id",
              when: () =>
                !permissions || permissions.has("pipeline-groups.read"),
              label: "分组",
              labelKey: "common-domain:table.groups",
              type: "select",
              emptyValues: ["0", 0],
              emptyLabelKey: "resource-domain:reference.ungrouped",
              options: namedOptions(workspace, "pipeline-groups"),
            },
            {
              key: "tag_ids",
              when: () => !permissions || permissions.has("tags.read"),
              label: "标签",
              labelKey: "common-domain:nav.tags",
              type: "multiselect",
              options: tagOptions(workspace),
              span: 2,
              createOption: {
                label: "新建标签",
                labelKey: "resource-domain:tag.create",
                permission: "tags.create",
                fields: [
                  {
                    key: "name",
                    label: "名称",
                    labelKey: "common-domain:common.name",
                    type: "text",
                    required: true,
                  },
                ],
                create: async (values) => {
                  const row = await createResource<{ id: string }>(
                    workspace,
                    "tags",
                    values,
                  );
                  return { value: row.id, label: String(values.name) };
                },
              },
            },
            {
              key: "definition",
              label: "Actions 工作流",
              labelKey: "resource-domain:pipeline.workflow",
              type: "actions-editor",
              required: true,
              span: 2,
              options: pipelineExecutorOptions(workspace, permissions),
            },
          ],
        },
      ],
    }),
  };
}
