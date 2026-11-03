import { deleteResource, named } from "../../shared/api/resource";
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
import type { FormConfig, ListConfig } from "../../shared/resources/types";
import { TARGETS } from "../hosts/resource-config";
import { templateGenerator } from "../templates/resource-config";
import { deploymentState, type Deployment, type Revision } from "./deployment";

export function deploymentsLists({
  workspace,
  dictionaries,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    deployments: list({
      resource: "deployments",
      rowKey: (r: Deployment) => r.id,
      filters: [
        { key: "category", label: "分类", labelKey: "typed-domain:category", type: "select", options: dictionaries?.["app-category"] ?? dictionaryOptions(workspace, "app-category") },
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
          sortable: true,
        },
        { key: "category", label: "分类", labelKey: "typed-domain:category", render: (row: Deployment) => row.category ? label(dictionaries?.["app-category"] ?? [], row.category) : "—" },
        {
          key: "state",
          label: "状态",
          labelKey: "resource-domain:status.title",
          render: (r) => deploymentState(r.state),
        },
        {
          key: "target_name",
          label: "目标",
          labelKey: "resource-domain:resource.target",
          render: (r) =>
            `${label(TARGETS, r.target_kind)} · ${named((r as AnyRow).target_name as string, r.target_ref)}`,
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
          permission: "deployments.read",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
        {
          label: "编辑",
          labelKey: "common-domain:common.edit",
          permission: "deployments.update",
          to: (r) => ({ id: r.id, mode: "edit" }),
        },
        {
          label: "删除记录（保留容器和卷）",
          labelKey: "resource-domain:deployment.deleteRecord",
          permission: "deployments.delete",
          destructive: true,
          action: (r) => deleteResource(workspace, "deployments", r.id),
        },
      ],
    }),
    "deployment-revisions": list({
      resource: "deployment-revisions",
      rowKey: (r: Revision) => r.id,
      filters: [
        {
          key: "deployment_id",
          label: "应用",
          labelKey: "common-domain:nav.applications",
          type: "select",
          options: namedOptions(workspace, "deployments"),
        },
      ],
      columns: [
        { key: "number", label: "Revision", sortable: true },
        {
          key: "deployment_name",
          label: "应用",
          labelKey: "common-domain:nav.applications",
          render: (r) =>
            named((r as AnyRow).deployment_name as string, r.deployment_id),
        },
        {
          key: "action",
          label: "操作",
          labelKey: "common-domain:table.actions",
          render: (r) =>
            r.action === "rollback"
              ? commonText("resource-domain:deployment.rollback")
              : commonText("resource-domain:variable.deploy"),
        },
        {
          key: "state",
          label: "状态",
          labelKey: "resource-domain:status.title",
          render: (r) => deploymentState(r.state),
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
          permission: "deployments.read",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
      ],
      toolbar: { create: false, import: false, export: false },
    }),
  };
}

export function deploymentsForms({
  workspace,
  permissions,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  return {
    deployments: form({
      ...(permissions?.has("app-templates.read")
        ? {
            generator: templateGenerator(
              workspace,
              "app-templates",
              "manifest",
            ),
          }
        : {}),
      resource: "deployments",
      heading: (r: Deployment) => r.name,
      blank: {
        name: "",
        category: "",
        mode: "compose",
        target_kind: "host",
        target_ref: "",
        workdir: "/opt/deployments",
        description: "",
        manifest:
          "apiVersion: ferry/v1\nkind: Application\nmetadata:\n  name: web\nservices:\n  - name: web\n    image: nginx:alpine\n    ports:\n      - container: 80\n        published: 8080\n",
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
            { key: "category", label: "分类", labelKey: "typed-domain:category", type: "select", options: dictionaryOptions(workspace, "app-category") },
            {
              key: "mode",
              label: "部署模式",
              labelKey: "resource-domain:deployment.mode",
              type: "select",
              options: [
                { value: "native", label: "Native（systemd）" },
                { value: "docker", label: "Docker" },
                { value: "compose", label: "Docker Compose" },
                { value: "stack", label: "Docker Swarm" },
              ],
              immutable: true,
            },
            {
              key: "target_kind",
              label: "目标类型",
              labelKey: "resource-domain:resource.targetType",
              type: "select",
              options: [
                {
                  value: "host",
                  label: "单主机",
                  labelKey: "resource-domain:deployment.singleHost",
                },
                {
                  value: "host-group",
                  label: "主机组（逐台执行）",
                  labelKey: "resource-domain:deployment.hostGroup",
                },
              ],
              immutable: true,
            },
            {
              key: "target_ref",
              label: "目标主机",
              labelKey: "resource-domain:deployment.targetHost",
              type: "select",
              options: namedOptions(workspace, "hosts"),
              required: true,
              immutable: true,
              when: (d) => d.target_kind === "host",
            },
            {
              key: "target_ref",
              label: "目标主机组",
              labelKey: "resource-domain:deployment.targetGroup",
              type: "select",
              options: namedOptions(workspace, "host-groups"),
              required: true,
              immutable: true,
              when: (d) => d.target_kind === "host-group",
              help: "部署全部直接成员，按主机 ID 排序；失败停止，已成功主机不自动回滚。",
              helpKey: "resource-domain:deployment.groupHelp",
            },
            {
              key: "workdir",
              label: "部署工作目录",
              labelKey: "resource-domain:deployment.workdir",
              type: "text",
              required: true,
              immutable: true,
              help: "目标 Linux 主机上的可写绝对路径；自动按工作空间和应用隔离子目录。容器模式需要 Docker，Compose 另需 Compose v2+；Native 需要 systemd 及管理服务的权限。所有模式需要 flock、sha256sum。",
              helpKey: "resource-domain:deployment.workdirHelp",
            },
            {
              key: "manifest",
              label: "应用配置",
              labelKey: "resource-domain:deployment.config",
              type: "manifest",
              required: true,
              span: 2,
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
