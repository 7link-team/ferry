import { api } from "../../shared/api/api";
import { deleteResource } from "../../shared/api/resource";
import { commonText } from "../../shared/i18n/i18n";
import type { ResourceConfigContext } from "../../shared/resources/config";
import { type AnyRow, form, list } from "../../shared/resources/config";
import type { FormConfig, ListConfig } from "../../shared/resources/types";

export interface Role {
  id: string;
  name: string;
  builtin: boolean;
  permissions: string[];
  version: number;
}

export interface Member {
  id: string;
  name: string;
  email: string;
  role_id: string;
  role_name: string;
  owner: boolean;
  active: boolean;
  state: string;
  version: number;
}

export function workspacesLists({
  workspace,
  permissions,
  onChanged,
}: ResourceConfigContext): Record<string, ListConfig<AnyRow>> {
  return {
    members: list({
      resource: "members",
      rowKey: (r: Member) => r.id,
      filters: [
        {
          key: "q",
          label: "姓名或邮箱",
          labelKey: "resource-domain:platform.nameEmail",
          type: "text",
        },
      ],
      columns: [
        { key: "name", label: "姓名", labelKey: "common-domain:auth.name" },
        { key: "email", label: "邮箱", labelKey: "common-domain:auth.email" },
        {
          key: "role_name",
          label: "角色",
          labelKey: "common-domain:dashboard.roles",
        },
        {
          key: "state",
          label: "状态",
          labelKey: "resource-domain:status.title",
          render: (r) =>
            r.owner
              ? commonText("resource-domain:member.owner")
              : !r.active
                ? commonText("resource-domain:member.disabled")
                : r.state === "active"
                  ? commonText("resource-domain:member.active")
                  : r.state,
        },
      ],
      actions: [
        {
          label: "查看",
          labelKey: "common-domain:common.view",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
        {
          label: "调整角色",
          labelKey: "resource-domain:member.changeRole",
          permission: "members.update",
          when: (r) => !r.owner,
          to: (r) => ({ id: r.id, mode: "edit" }),
        },
        {
          label: "移除",
          labelKey: "resource-domain:member.remove",
          permission: "members.delete",
          when: (r) => !r.owner,
          destructive: true,
          action: async (r) => {
            await deleteResource(workspace, "members", r.id);
            onChanged?.();
          },
        },
      ],
      toolbar: {
        create: false,
        import: false,
        export: false,
      },
    }),
    roles: list({
      resource: "roles",
      rowKey: (r: Role) => r.id,
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
          sortable: true,
        },
        {
          key: "builtin",
          label: "类型",
          labelKey: "resource-domain:resource.type",
          render: (r) =>
            r.builtin
              ? commonText("resource-domain:role.builtin")
              : commonText("resource-domain:role.custom"),
        },
        {
          key: "permissions",
          label: "权限数",
          labelKey: "resource-domain:role.permissionCount",
          render: (r) => r.permissions.length,
        },
      ],
      actions: [
        {
          label: "查看",
          labelKey: "common-domain:common.view",
          to: (r) => ({ id: r.id, mode: "view" }),
        },
        {
          label: "复制",
          labelKey: "resource-domain:resource.copy",
          permission: "roles.create",
          when: (r) => r.permissions.every((p) => permissions?.has(p) ?? true),
          to: () => ({ mode: "new" }),
          query: (r) => ({ copy: r.id }),
        },
        {
          label: "编辑",
          labelKey: "common-domain:common.edit",
          permission: "roles.update",
          when: (r) => !r.builtin,
          to: (r) => ({ id: r.id, mode: "edit" }),
        },
        {
          label: "删除",
          labelKey: "common-domain:common.delete",
          permission: "roles.delete",
          when: (r) => !r.builtin,
          destructive: true,
          action: async (r) => {
            await deleteResource(workspace, "roles", r.id);
            onChanged?.();
          },
        },
      ],
      toolbar: { create: true, import: false, export: false },
    }),
  };
}

export function workspacesForms({
  workspace,
  permissions,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  return {
    members: form({
      resource: "members",
      heading: (r: Member) => r.name,
      blank: {},
      readOnly: (r) => !!r.owner,
      sections: [
        {
          fields: [
            {
              key: "name",
              label: "姓名",
              labelKey: "common-domain:auth.name",
              type: "text",
              immutable: true,
            },
            {
              key: "email",
              label: "邮箱",
              labelKey: "common-domain:auth.email",
              type: "text",
              immutable: true,
            },
            {
              key: "role_name",
              label: "当前角色",
              labelKey: "resource-domain:member.currentRole",
              type: "text",
              immutable: true,
            },
            {
              key: "role_id",
              label: "角色",
              labelKey: "common-domain:dashboard.roles",
              type: "select",
              required: true,
              when: (r) => !r.owner,
              options: async () => {
                if (permissions && !permissions.has("members.update"))
                  return [];
                const result = await api<{ items: Role[] }>(
                  `/workspaces/${workspace}/members/role-options`,
                );
                return result.items.map((r) => ({
                  value: r.id,
                  label: r.name,
                }));
              },
            },
          ],
        },
      ],
    }),
    roles: form({
      resource: "roles",
      heading: (r: Role) => r.name,
      blank: { name: "", permissions: [] },
      readOnly: (r) => !!r.builtin,
      copy: (r) => ({
        name: commonText("resource-domain:resource.copyName", { name: r.name }),
        permissions: [...r.permissions],
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
              key: "permissions",
              label: "权限",
              labelKey: "resource-domain:role.permissions",
              type: "multiselect",
              span: 2,
              help: "勾选操作会自动包含依赖权限；取消依赖权限会同时取消依赖它的操作。",
              helpKey: "resource-domain:role.permissionsHelp",
              options: async () => {
                const result = await api<{
                  items: {
                    key: string;
                    group: string;
                    action: string;
                    requires: string[];
                  }[];
                }>("/permissions");
                return result.items.map((p) => ({
                  value: p.key,
                  label: p.action,
                  group: p.group,
                  requires: p.requires,
                  disabled: permissions ? !permissions.has(p.key) : false,
                }));
              },
            },
          ],
        },
      ],
    }),
  };
}
