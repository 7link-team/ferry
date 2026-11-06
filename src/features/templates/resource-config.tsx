import { lazy, Suspense } from "react";
import { stringify } from "yaml";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { TemplateParametersEditor, type TemplateParameter } from "./TemplateParametersEditor";
import { api } from "../../shared/api/api";
import { deleteResource } from "../../shared/api/resource";
import { commonText, workflowText } from "../../shared/i18n/i18n";
import { emptyGraph, parseGraph } from "../workflows/graph";
import { WorkflowEditor } from "../workflows/WorkflowEditor";
import { WorkflowDraftAssetsContext } from "../workflows/WorkflowAssets";
import type { ResourceConfigContext } from "../../shared/resources/config";
import {
  type AnyRow,
  form,
  list,
  namedOptions,
} from "../../shared/resources/config";
import type { FormConfig, ListConfig } from "../../shared/resources/types";
import { DEFAULT_ACTIONS, parseActions } from "../pipelines/actions-workflow";
import { pipelineExecutorOptions } from "../pipelines/executor-options";

const TemplateCodeEditor = lazy(() => import("../../shared/editor/CodeEditor").then(module => ({ default: module.CodeEditor })));

export interface Template {
  id: string;
  name: string;
  description: string;
  content: string;
  parameters: string;
  version: number;
}

export type { TemplateParameter } from "./TemplateParametersEditor";

export const templateList = (
  workspace: string,
  resource: string,
  target: string,
  group: string,
  item: string,
) =>
  list<Template>({
    resource,
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
        key: "description",
        label: "说明",
        labelKey: "common-domain:common.description",
      },
      {
        key: "version",
        label: "版本",
        labelKey: "common-domain:common.version",
      },
    ],
    actions: [
      {
        label: "查看",
        labelKey: "common-domain:common.view",
        to: (r) => ({ id: r.id, mode: "view" }),
      },
      {
        label: "使用模板",
        labelKey: "resource-domain:template.use",
        permission: `${target}.create`,
        to: () => ({ group, item, mode: "new" }),
        query: (r) => ({ template: r.id }),
      },
      {
        label: "复制",
        labelKey: "resource-domain:resource.copy",
        permission: `${resource}.create`,
        to: () => ({ mode: "new" }),
        query: (r) => ({ copy: r.id }),
      },
      {
        label: "编辑",
        labelKey: "common-domain:common.edit",
        permission: `${resource}.update`,
        to: (r) => ({ id: r.id, mode: "edit" }),
      },
      {
        label: "删除模板",
        labelKey: "resource-domain:template.delete",
        permission: `${resource}.delete`,
        destructive: true,
        action: (r) => deleteResource(workspace, resource, r.id),
      },
    ],
    toolbar: { create: true, import: false, export: false },
  });

export const templateForm = (resource: string, application: boolean, workspace = "", permissions?: Set<string>) =>
  form<Template>({
    resource,
    width: "full",
    heading: (r) => r.name,
    blank: {
      name: "",
      description: "",
      content:
        resource === "config-templates"
          ? "port={{ .Values.port }}\n"
          : application
            ? stringify(
                {
                  apiVersion: "ferry/v1",
                  kind: "Application",
                  metadata: { name: "web" },
                  services: [{ name: "web", image: "{{ .Values.image }}" }],
                },
              )
            : JSON.stringify(
                {
                  engine: "github-actions",
                  workflow: DEFAULT_ACTIONS,
                  executor_bindings: [],
                  checkout_bindings: [],
                  graph_layout: {},
                },
                null,
                2,
              ),
      parameters:
        resource === "pipeline-templates"
          ? "[]"
          : resource === "config-templates"
          ? JSON.stringify(
              [
                {
                  name: "port",
                  type: "integer",
                  default: 8080,
                  min: 1,
                  max: 65535,
                },
              ],
              null,
              2,
            )
          : JSON.stringify(
              [
                {
                  name: application ? "image" : "command",
                  label: application
                    ? commonText("resource-domain:template.image")
                    : commonText("resource-domain:template.buildCommand"),
                  type: "string",
                  default: application ? "nginx:alpine" : "echo build",
                  min_length: 1,
                },
              ],
              null,
              2,
            ),
    },
    copy: (r) => ({
      name: commonText("resource-domain:resource.copyName", { name: r.name }),
      description: r.description,
      content: r.content,
      parameters: r.parameters,
    }),
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
          {
            key: "parameters",
            when: () => resource !== "pipeline-templates",
            label: "模板参数",
            labelKey: "resource-domain:template.parameters",
            type: "custom",
            render: ({ draft, readOnly, onPatch, onValidityChange }) => <TemplateParametersEditor value={String(draft.parameters ?? "[]")} readOnly={readOnly} onChange={parameters => onPatch({ parameters })} onValidityChange={onValidityChange} />,
            span: 2,
            required: true,
          },
          {
            key: "content",
            label:
              resource === "config-templates"
                ? "应用配置文件内容"
                : application
                  ? "应用模板（YAML / JSON）"
                  : "流水线模板（JSON / YAML）",
            labelKey:
              resource === "config-templates"
                ? "resource-domain:template.fileContent"
                : application
                  ? "resource-domain:template.appContent"
                  : "resource-domain:template.pipelineContent",
            type: resource === "pipeline-templates" ? "actions-template-editor" : "custom",
            ...(resource === "pipeline-templates" ? {} : { render: ({ draft, readOnly, onPatch }: { draft: Partial<Template>; readOnly: boolean; onPatch: (patch: Partial<Template>) => void; }) => <div className="h-[28rem] min-w-0 overflow-hidden rounded-md border"><Suspense fallback={<LoadingSkeleton variant="editor" />}><TemplateCodeEditor value={String(draft.content ?? "")} language={application ? "yaml" : "plaintext"} path={`${workspace}/${resource}/${draft.id ?? "new"}/content.${application ? "yaml" : "txt"}`} label={commonText("resource-domain:deployment.config")} readOnly={readOnly} onChange={content => onPatch({ content })} /></Suspense></div> }),
            ...(resource === "pipeline-templates" ? { options: pipelineExecutorOptions(workspace, permissions) } : {}),
            span: 2,
            required: true,
            help:
              resource === "config-templates"
                ? "例如 nginx.conf、application.yml 或 .toml 的内容，使用 {{ .Values.参数名 }} 替换参数。应用服务的 files 中填写 path（目标文件路径）、fromTemplate（此配置名称）和 values（参数）。秘密参数只填写变量名；回滚恢复当时的文件内容。"
                : application
                  ? '使用 "{{ .Values.image }}" 等占位符；secret: true 参数填写变量名，只能作为 valueFrom 的完整值。保存校验模板语法，生成时校验完整配置。'
                  : '使用 "{{ .Values.command }}" 等创建参数；保留 GitHub Actions ${{ ... }} 运行时表达式。secret 参数填写变量名，作为 Actions env 的完整值生成 secrets 引用。',
            helpKey:
              resource === "config-templates"
                ? "resource-domain:template.fileHelp"
                : application
                  ? "resource-domain:template.appHelp"
                  : "resource-domain:template.pipelineHelp",
          },
        ],
      },
    ],
  });

export const templateGenerator = (
  workspace: string,
  resource: string,
  field: string,
  workflowKind: "actions" | "nodes" = "actions",
): NonNullable<FormConfig<AnyRow>["generator"]> => ({
  options: namedOptions(workspace, resource, undefined, resource === "pipeline-templates" ? {workflow_kind:workflowKind} : {}),
  load: async (id) => {
    const row = await api<Template>(
      `/workspaces/${workspace}/${resource}/${id}`,
    );
    const parameters = JSON.parse(row.parameters) as TemplateParameter[];
    return {
      name: row.name,
      version: row.version,
      values: Object.fromEntries(
        parameters
          .filter((p) => p.default !== undefined)
          .map((p) => [
            p.name,
            p.type === "boolean" ? String(p.default) : p.default,
          ]),
      ),
      fields: parameters.map((p) => ({
        key: p.name,
        get label() {
          return `${p.label || p.name}${p.secret ? commonText("resource-domain:template.secretNameSuffix") : ""}`;
        },
        type:
          p.enum || p.type === "boolean"
            ? ("select" as const)
            : p.type === "integer"
              ? ("number" as const)
              : ("text" as const),
        required: p.default === undefined,
        ...(p.enum
          ? { options: p.enum.map((value) => ({ value, label: value })) }
          : p.type === "boolean"
            ? {
                options: [
                  {
                    value: "true",
                    label: "是",
                    labelKey: "common-domain:common.yes",
                  },
                  {
                    value: "false",
                    label: "否",
                    labelKey: "common-domain:common.no",
                  },
                ],
              }
            : {}),
        get help() {
          return p.secret
            ? commonText("resource-domain:template.secretHelp")
            : `${p.name}${p.min !== undefined ? commonText("resource-domain:parameter.minimum", { value: p.min }) : ""}${p.max !== undefined ? commonText("resource-domain:parameter.maximum", { value: p.max }) : ""}${p.min_length !== undefined ? commonText("resource-domain:parameter.minLength", { value: p.min_length }) : ""}${p.max_length !== undefined ? commonText("resource-domain:parameter.maxLength", { value: p.max_length }) : ""}`;
        },
      })),
    };
  },
  generate: async (id, version, values) => {
    // The displayed template version is checked by the server; metadata fetch
    // here is only for typed controls, never an implicit version upgrade.
    const row = await api<Template>(
      `/workspaces/${workspace}/${resource}/${id}`,
    );
    const parameters = JSON.parse(row.parameters) as TemplateParameter[];
    const typed = Object.fromEntries(
      Object.entries(values)
        .filter(
          ([key, value]) =>
            !(
              parameters.find((p) => p.name === key)?.type === "integer" &&
              value === ""
            ),
        )
        .map(([key, value]) => [
          key,
          parameters.find((p) => p.name === key)?.type === "boolean"
            ? value === "true"
            : value,
        ]),
    );
    const result = await api<Record<string, string>>(
      `/workspaces/${workspace}/${resource}/${id}/renderings`,
      { method: "POST", body: JSON.stringify({ version, values: typed }) },
    );
    if (resource === "pipeline-templates") {
      const rendered = JSON.parse(result[field] ?? "{}") as Record<
        string,
        unknown
      >;
      if (workflowKind === "nodes") {
        const definition = JSON.stringify(rendered);
        parseGraph(definition);
        return {definition};
      }
      if (
        (rendered.engine === "github-actions" ||
          rendered.engine === "actions") &&
        typeof rendered.workflow === "string"
      ) {
        parseActions(rendered.workflow);
        return {
          definition: rendered.workflow,
          definition_format: "github-actions",
          executor_bindings: rendered.executor_bindings ?? [],
          checkout_bindings: rendered.checkout_bindings ?? [],
          graph_layout: rendered.graph_layout ?? {},
        };
      }
      throw new Error(commonText("resource-domain:template.invalidActions"));
    }
    return { [field]: result[field] };
  },
});

export function templatesLists({
  workspace,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    "config-templates": {
      ...templateList(
        workspace,
        "config-templates",
        "deployments",
        "deploy",
        "apps",
      ),
      actions:
        templateList(
          workspace,
          "config-templates",
          "deployments",
          "deploy",
          "apps",
        ).actions?.filter((a) => a.label !== "使用模板") ?? [],
    },
    "app-templates": templateList(
      workspace,
      "app-templates",
      "deployments",
      "deploy",
      "apps",
    ),
    "pipeline-templates": templateList(
      workspace,
      "pipeline-templates",
      "pipelines",
      "build",
      "pipelines",
    ),
    "workflow-templates": {...templateList(workspace,"pipeline-templates","workflows","workflow","workflows"),fixed:{workflow_kind:"nodes"}},
  };
}

export function templatesForms({ workspace, permissions }: ResourceConfigContext): Record<
  string,
  FormConfig<AnyRow>
> {
  return {
    "config-templates": templateForm("config-templates", false),
    "app-templates": templateForm("app-templates", true),
    "pipeline-templates": templateForm("pipeline-templates", false, workspace, permissions),
    "workflow-templates": nodeTemplateForm(workspace, permissions),
  };
}

function nodeTemplateForm(workspace: string, permissions = new Set<string>()): FormConfig<AnyRow> {
  const original = templateForm("pipeline-templates",false,workspace,permissions);
  return {...original,editor:{field:"content",get nameLabel(){return workflowText("name");}},blank:{...original.blank,content:JSON.stringify(emptyGraph())},sections:original.sections.map(section=>({...section,fields:section.fields.map(field=>field.key!=="content"?field:{...field,type:"custom",render:({draft,readOnly,onPatch,onValidityChange})=><WorkflowDraftAssetsContext.Provider value={null}><WorkflowEditor workspace={workspace} templateMode draft={{name:String(draft.name??""),definition:String(draft.content??"")}} readOnly={readOnly} permissions={permissions} onPatch={patch=>{if(patch.definition!==undefined)onPatch({content:patch.definition});}} onValidityChange={onValidityChange}/></WorkflowDraftAssetsContext.Provider>})}))};
}
