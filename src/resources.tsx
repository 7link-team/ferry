/**
 * Every resource page in one place: a `ListConfig` and a `FormConfig` per
 * resource. This is the whole of a page — there is no per-resource component.
 *
 * A page absent from here renders the "not built yet" placeholder, so adding a
 * module means adding entries here plus the server handlers.
 */
import { Badge } from "@/components/ui/badge";
import { deleteResource, listResource, runAction, runItemAction } from "./resource";
import type { FormConfig, ListConfig, Option } from "./components/dsl";

/* The registries are keyed by resource name, so their values have to share one
   type while each config stays checked against its own row type. These two
   helpers do exactly that: the argument is checked, the return is erased. */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type AnyRow = any;
const list = <T,>(config: ListConfig<T>): ListConfig<AnyRow> => config;
const form = <T,>(config: FormConfig<T>): FormConfig<AnyRow> => config;

export interface HostGroup {
  id: string;
  name: string;
  description: string;
  parent_id: string;
  version: number;
}
export interface Host {
  id: string;
  name: string;
  address: string;
  port: number;
  username: string;
  connection: string;
  state: string;
  group_id: string;
  description: string;
  seen_at: number;
  version: number;
}
export interface Activity {
  id: string;
  actor_name: string;
  action: string;
  resource: string;
  resource_id: string;
  details: unknown;
  created_at: number;
}
export interface Tag {
  id: string;
  name: string;
  color: string;
  version: number;
}

const CONNECTIONS: Option[] = [
  { value: "runner", label: "代理（runner）" },
  { value: "ssh", label: "SSH 直连" },
];
const STATES: Option[] = [
  { value: "online", label: "在线" },
  { value: "offline", label: "离线" },
  { value: "unknown", label: "未知" },
];

const label = (options: Option[], value: string) =>
  options.find((option) => option.value === value)?.label ?? value;

/** Relative time; absolute dates make an operator do arithmetic. */
function ago(at: number) {
  if (!at) return "—";
  const seconds = Math.max(0, Math.round((Date.now() - at) / 1000));
  if (seconds < 60) return `${seconds} 秒前`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟前`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} 小时前`;
  return `${Math.floor(seconds / 86400)} 天前`;
}

/** Group options for the filter and the form, loaded once per render pass. */
const groupOptions = (workspace: string) => async (): Promise<Option[]> => {
  const page = await listResource<HostGroup>(workspace, "host-groups", { page_size: 200 });
  return page.items.map((group) => ({ value: group.id, label: group.name }));
};

export function listConfigs(workspace: string): Record<string, ListConfig<AnyRow>> {
  return {
    "host-groups": list({
      resource: "host-groups",
      rowKey: (row: HostGroup) => row.id,
      filters: [{ key: "q", label: "名称/说明", type: "text", placeholder: "输入关键字" }],
      columns: [
        { key: "name", label: "名称" },
        { key: "description", label: "说明" },
      ],
      actions: [
        { label: "查看", permission: "host-groups.read", to: (row: HostGroup) => ({ id: row.id, mode: "view" }) },
        { label: "编辑", permission: "host-groups.update", to: (row: HostGroup) => ({ id: row.id, mode: "edit" }) },
        {
          label: "删除",
          permission: "host-groups.delete",
          destructive: true,
          action: (row: HostGroup) => deleteResource(workspace, "host-groups", row.id),
        },
      ],
      empty: "还没有主机组。先建一个，再把主机归进去。",
    }),

    hosts: list({
      resource: "hosts",
      rowKey: (row: Host) => row.id,
      filters: [
        { key: "q", label: "名称/地址/账号", type: "text", placeholder: "输入关键字" },
        { key: "group_id", label: "主机组", type: "select", options: groupOptions(workspace) },
        { key: "connection", label: "接入方式", type: "select", options: CONNECTIONS },
        { key: "state", label: "状态", type: "select", options: STATES },
      ],
      columns: [
        { key: "name", label: "名称" },
        { key: "address", label: "地址", width: 180 },
        {
          key: "connection",
          label: "接入方式",
          width: 130,
          render: (row: Host) => label(CONNECTIONS, row.connection),
        },
        {
          key: "state",
          label: "状态",
          width: 90,
          render: (row: Host) => (
            <Badge
              variant={
                row.state === "online"
                  ? "default"
                  : row.state === "offline"
                    ? "destructive"
                    : "secondary"
              }
            >
              {label(STATES, row.state)}
            </Badge>
          ),
        },
        { key: "seen_at", label: "最近心跳", width: 130, sortable: true, render: (row: Host) => ago(row.seen_at) },
      ],
      actions: [
        { label: "查看", permission: "hosts.read", to: (row: Host) => ({ id: row.id, mode: "view" }) },
        {
          label: "测试连接",
          permission: "hosts.read",
          action: async (row: Host) => {
            const result = await runItemAction<{ ok: boolean; stderr: string }>(
              workspace, "hosts", row.id, "test-connection", {},
            );
            if (!result.ok) throw new Error(result.stderr || "连接失败。");
          },
        },
        { label: "编辑", permission: "hosts.update", to: (row: Host) => ({ id: row.id, mode: "edit" }) },
        {
          label: "删除",
          permission: "hosts.delete",
          destructive: true,
          action: (row: Host) => deleteResource(workspace, "hosts", row.id),
        },
      ],
      bulk: [
        {
          label: "批量删除",
          permission: "hosts.delete",
          destructive: true,
          action: async (ids) => {
            await runAction(workspace, "hosts", "bulk-delete", { ids });
          },
        },
      ],
      empty: "还没有主机。用代理一条命令接入，或填 SSH 账号直连。",
    }),

    tags: list({
      resource: "tags",
      rowKey: (row: Tag) => row.id,
      filters: [{ key: "q", label: "名称", type: "text", placeholder: "输入关键字" }],
      columns: [
        { key: "name", label: "名称" },
        {
          key: "color",
          label: "颜色",
          width: 120,
          render: (row: Tag) =>
            row.color ? (
              <span className="inline-flex items-center gap-2">
                <span
                  className="inline-block size-3 rounded-full border border-border"
                  style={{ background: row.color }}
                />
                {row.color}
              </span>
            ) : (
              "—"
            ),
        },
      ],
      actions: [
        { label: "编辑", permission: "tags.update", to: (row: Tag) => ({ id: row.id, mode: "edit" }) },
        {
          label: "删除",
          permission: "tags.delete",
          destructive: true,
          action: (row: Tag) => deleteResource(workspace, "tags", row.id),
        },
      ],
      empty: "还没有标签。",
    }),

    activity: list({
      resource: "activity",
      rowKey: (row: Activity) => row.id,
      filters: [
        { key: "q", label: "操作人/资源", type: "text", placeholder: "输入关键字" },
        {
          key: "action",
          label: "动作",
          type: "select",
          options: [
            { value: "create", label: "新增" },
            { value: "update", label: "修改" },
            { value: "delete", label: "删除" },
            { value: "bulk-delete", label: "批量删除" },
          ],
        },
      ],
      columns: [
        { key: "created_at", label: "时间", width: 180, sortable: true, render: (row: Activity) => new Date(row.created_at).toLocaleString("zh-CN") },
        { key: "actor_name", label: "操作人", width: 120 },
        { key: "action", label: "动作", width: 100 },
        { key: "resource", label: "资源", width: 140 },
        {
          key: "details",
          label: "详情",
          render: (row: Activity) => (
            <code className="text-xs text-muted-foreground">{JSON.stringify(row.details)}</code>
          ),
        },
      ],
      toolbar: { create: false, import: false },
      empty: "还没有操作记录。",
    }),
  };
}

export function formConfigs(workspace: string): Record<string, FormConfig<AnyRow>> {
  return {
    "host-groups": form({
      resource: "host-groups",
      heading: (row: HostGroup) => row.name,
      blank: { name: "", description: "", parent_id: "" },
      sections: [
        {
          fields: [
            { key: "name", label: "名称", type: "text", required: true, placeholder: "例如 生产-华东" },
            { key: "parent_id", label: "上级主机组", type: "select", options: groupOptions(workspace), help: "留空表示顶级" },
            { key: "description", label: "说明", type: "textarea", span: 2 },
          ],
        },
      ],
    }),

    hosts: form({
      resource: "hosts",
      heading: (row: Host) => row.name,
      blank: { name: "", connection: "runner", address: "", port: 22, username: "", group_id: "", description: "" },
      sections: [
        {
          title: "基本信息",
          fields: [
            { key: "name", label: "名称", type: "text", required: true, placeholder: "例如 web-01" },
            { key: "group_id", label: "主机组", type: "select", options: groupOptions(workspace) },
            {
              key: "connection",
              label: "接入方式",
              type: "select",
              required: true,
              options: CONNECTIONS,
              immutable: true,
              help: "创建后不可更改：改了会让已注册的代理密钥或 SSH 凭据失效。",
            },
          ],
        },
        {
          title: "SSH 连接",
          fields: [
            {
              key: "address",
              label: "主机地址",
              type: "text",
              required: true,
              placeholder: "10.0.0.11 或 host.example.com",
              when: (draft: Partial<Host>) => draft.connection === "ssh",
            },
            { key: "port", label: "端口", type: "number", required: true, when: (draft: Partial<Host>) => draft.connection === "ssh" },
            { key: "username", label: "登录账号", type: "text", required: true, placeholder: "root", when: (draft: Partial<Host>) => draft.connection === "ssh" },
          ],
        },
        {
          fields: [{ key: "description", label: "备注", type: "textarea", span: 2 }],
        },
      ],
    }),

    tags: form({
      resource: "tags",
      heading: (row: Tag) => row.name,
      blank: { name: "", color: "" },
      sections: [
        {
          fields: [
            { key: "name", label: "名称", type: "text", required: true },
            { key: "color", label: "颜色", type: "text", placeholder: "#4f9cf9" },
          ],
        },
      ],
    }),
  };
}
