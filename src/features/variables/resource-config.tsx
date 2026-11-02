import { deleteResource, runItemAction } from "../../shared/api/resource";
import { commonText } from "../../shared/i18n/i18n";
import type { ResourceConfigContext } from "../../shared/resources/config";
import {
  type AnyRow,
  form,
  label,
  list,
  namedOptions,
} from "../../shared/resources/config";
import type { FormConfig, ListConfig } from "../../shared/resources/types";

export interface Variable {
  id: string;
  name: string;
  scope: string;
  scope_ref: string;
  kind: string;
  description: string;
  has_value: boolean;
  value?: string;
  version: number;
}

export const VARIABLE_SCOPES = [
  {
    value: "global",
    label: "全局",
    labelKey: "resource-domain:variable.global",
  },
  {
    value: "deploy",
    label: "部署",
    labelKey: "resource-domain:variable.deploy",
  },
  { value: "build", label: "构建", labelKey: "common-domain:nav.build" },
  {
    value: "environment",
    label: "执行环境",
    labelKey: "resource-domain:pipeline.environment",
  },
];

export const VARIABLE_KINDS = [
  {
    value: "secret",
    label: "密钥",
    labelKey: "resource-domain:variable.secret",
  },
  { value: "plain", label: "普通", labelKey: "resource-domain:variable.plain" },
];

export const variableSource = (row: Variable) =>
  row.scope === "global"
    ? commonText("resource-domain:variable.global")
    : `${label(VARIABLE_SCOPES, row.scope)} · ${row.scope_ref === "0" ? commonText("resource-domain:variable.moduleDefault") : commonText("resource-domain:variable.object", { id: row.scope_ref })}`;

export function variablesLists({
  workspace,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    variables: list({
      resource: "variables",
      rowKey: (row: Variable) => row.id,
      filters: [
        {
          key: "q",
          label: "名称 / 备注",
          labelKey: "resource-domain:resource.nameNotes",
          type: "text",
        },
        {
          key: "kind",
          label: "类型",
          labelKey: "resource-domain:resource.type",
          type: "select",
          options: VARIABLE_KINDS,
        },
        {
          key: "scope_ref",
          label: "对象 ID",
          labelKey: "resource-domain:variable.objectId",
          type: "text",
          placeholder: "0 表示模块默认",
          placeholderKey: "resource-domain:variable.zeroDefault",
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
          key: "kind",
          label: "类型",
          labelKey: "resource-domain:resource.type",
          render: (row) => label(VARIABLE_KINDS, row.kind),
        },
        {
          key: "scope",
          label: "来源层",
          labelKey: "resource-domain:variable.sourceLevel",
          render: variableSource,
        },
        {
          key: "value",
          label: "值",
          labelKey: "resource-domain:variable.value",
          render: (row) =>
            row.kind === "secret" ? (
              row.has_value ? (
                commonText("common-domain:common.set")
              ) : (
                commonText("common-domain:common.notSet")
              )
            ) : (
              <span className="whitespace-pre-wrap break-all">
                {row.value === ""
                  ? commonText("common-domain:form.emptyString")
                  : row.value}
              </span>
            ),
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
          permission: "variables.read",
          to: (row) => ({ id: row.id, mode: "view" }),
        },
        {
          label: "编辑",
          labelKey: "common-domain:common.edit",
          permission: "variables.update",
          to: (row) => ({ id: row.id, mode: "edit" }),
        },
        {
          label: "删除",
          labelKey: "common-domain:common.delete",
          permission: "variables.delete",
          destructive: true,
          action: (row) => deleteResource(workspace, "variables", row.id),
        },
      ],
      toolbar: { import: false, export: false },
      empty: "当前作用域没有变量。全局、模块和对象同名时，具体对象优先。",
      emptyKey: "resource-domain:variable.empty",
    }),
  };
}

export function variablesForms({
  workspace,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  return {
    variables: form({
      resource: "variables",
      heading: (row: Variable) => row.name,
      blank: {
        name: "",
        scope: "global",
        scope_ref: "0",
        kind: "secret",
        value: "",
        description: "",
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
              immutable: true,
              help: "96个以内字母、数字、下划线或连字符；引用写作 variable/名称。",
              helpKey: "resource-domain:variable.nameHelp",
            },
            {
              key: "kind",
              label: "类型",
              labelKey: "resource-domain:resource.type",
              type: "select",
              options: VARIABLE_KINDS,
              immutable: true,
            },
            {
              key: "scope",
              label: "作用域",
              labelKey: "resource-domain:variable.scope",
              type: "select",
              options: VARIABLE_SCOPES,
              immutable: true,
              resetOnChange: ["scope_ref"],
            },
            {
              key: "scope_ref",
              label: "执行环境",
              labelKey: "resource-domain:pipeline.environment",
              type: "select",
              options: namedOptions(workspace, "pipeline-environments"),
              required: true,
              immutable: true,
              when: (row) => row.scope === "environment",
              help: "只用于选中该环境的流水线执行；同名值覆盖全局、构建和流水线变量。",
              helpKey: "resource-domain:variable.environmentHelp",
            },
            {
              key: "scope_ref",
              label: "对象 ID",
              labelKey: "resource-domain:variable.objectId",
              type: "text",
              immutable: true,
              when: (row) =>
                row.scope !== "global" && row.scope !== "environment",
              help: "0 或留空为模块默认；部署范围填应用 ID，构建范围填流水线 ID。具体对象覆盖模块，模块覆盖全局。",
              helpKey: "resource-domain:variable.objectHelp",
            },
            {
              key: "value",
              label: "变量值",
              labelKey: "resource-domain:variable.plainValue",
              type: "textarea",
              span: 2,
              when: (row) => row.kind === "plain",
              help: "空白和换行会原样保存；留空表示空字符串。",
              helpKey: "resource-domain:variable.whitespaceHelp",
            },
            {
              key: "value",
              label: "密钥值",
              labelKey: "resource-domain:variable.secretValue",
              type: "secret",
              stored: "has_value",
              replaceOnEdit: true,
              span: 2,
              when: (row) => row.kind === "secret",
              help: "加密保存。编辑时勾选替换后留空，表示设置为空字符串；查看明文会记录审计。",
              helpKey: "resource-domain:variable.secretEditHelp",
              reveal: {
                permission: "variables.reveal",
                action: async (row) =>
                  (
                    await runItemAction<{ value: string }>(
                      workspace,
                      "variables",
                      String(row.id),
                      "reveal",
                      {},
                    )
                  ).value,
              },
            },
            {
              key: "description",
              label: "备注",
              labelKey: "resource-domain:resource.notes",
              type: "textarea",
              span: 2,
            },
          ],
        },
      ],
    }),
  };
}
