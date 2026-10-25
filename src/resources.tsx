/**
 * Every resource page in one place: a `ListConfig` and a `FormConfig` per
 * resource. This is the whole of a page — there is no per-resource component.
 *
 * A page absent from here renders the "not built yet" placeholder, so adding a
 * module means adding entries here plus the server handlers.
 */
import { duration, stateBadge, type PipelineRun } from "./pipeline";
import { deploymentState, type Deployment, type Revision } from "./deployment";
import { nav } from "./nav";
import { navigate } from "./router";
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
  credential_id: string;
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
export interface Credential {
  id: string;
  name: string;
  kind: string;
  username: string;
  description: string;
  /** The server reports only whether a secret is set, never its value. */
  has_secret: boolean;
  /** Write-only: sent when creating or rotating, never returned by any read. */
  secret?: string;
  version: number;
}
export interface Tag {
  id: string;
  name: string;
  color: string;
  version: number;
}

const CREDENTIAL_KINDS: Option[] = [
  { value: "password", label: "密码" },
  { value: "ssh-key", label: "SSH 私钥" },
  { value: "token", label: "访问令牌" },
];
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
/** SSH login needs a password or a key; a token is for Git, not for sshd. */
const sshCredentialOptions = (workspace: string) => async (): Promise<Option[]> => {
  const page = await listResource<Credential>(workspace, "credentials", { page_size: 200 });
  return page.items
    .filter((c) => c.kind === "password" || c.kind === "ssh-key")
    .map((c) => ({ value: c.id, label: `${c.name}（${label(CREDENTIAL_KINDS, c.kind)}）` }));
};

const groupOptions = (workspace: string) => async (): Promise<Option[]> => {
  const page = await listResource<HostGroup>(workspace, "host-groups", { page_size: 200 });
  return page.items.map((group) => ({ value: group.id, label: group.name }));
};

interface Repository {
  id: string; name: string; url: string; default_branch: string;
  credential_id: string; description: string;
}
interface Pipeline {
  id: string; name: string; repository_id: string; target_kind: string;
  target_ref: string; definition: string; description: string;
}
const namedOptions = (workspace: string, resource: string) => async (): Promise<Option[]> => {
  const items: Option[] = [];
  for (let page = 1; ; page += 1) {
    const result = await listResource<{ id: string; name: string }>(workspace, resource, { page, page_size: 200 });
    items.push(...result.items.map((row) => ({ value: row.id, label: row.name })));
    if (!result.items.length || page * result.page_size >= result.total) return items;
  }
};
const TARGETS = [{ value: "host", label: "主机" }, { value: "host-group", label: "主机组" }];

export function listConfigs(workspace: string): Record<string, ListConfig<AnyRow>> {
  return {
    deployments: list({
      resource: "deployments", rowKey: (r: Deployment) => r.id,
      filters: [{ key: "q", label: "名称", type: "text" }],
      columns: [{ key: "name", label: "名称", sortable: true }, { key: "state", label: "状态", render: (r) => deploymentState(r.state) }, { key: "target_ref", label: "目标主机 ID" }, { key: "description", label: "备注" }],
      actions: [
        { label: "查看", permission: "deployments.read", to: (r) => ({ id: r.id, mode: "view" }) },
        { label: "编辑", permission: "deployments.update", to: (r) => ({ id: r.id, mode: "edit" }) },
        { label: "删除记录（保留容器和卷）", permission: "deployments.delete", destructive: true, action: (r) => deleteResource(workspace, "deployments", r.id) },
      ],
    }),
    "deployment-revisions": list({
      resource: "deployment-revisions", rowKey: (r: Revision) => r.id,
      filters: [{ key: "deployment_id", label: "应用 ID", type: "text" }],
      columns: [{ key: "number", label: "Revision", sortable: true }, { key: "deployment_id", label: "应用 ID" }, { key: "action", label: "操作", render: (r) => r.action === "rollback" ? "回滚" : "部署" }, { key: "state", label: "状态", render: (r) => deploymentState(r.state) }, { key: "started_at", label: "开始时间", render: (r) => r.started_at ? new Date(r.started_at).toLocaleString() : "—" }],
      actions: [{ label: "查看", permission: "deployments.read", to: (r) => ({ id: r.id, mode: "view" }) }],
      toolbar: { create: false, import: false, export: false },
    }),
    repositories: list({
      resource: "repositories", rowKey: (r: Repository) => r.id,
      filters: [{ key: "q", label: "名称 / URL", type: "text" }],
      columns: [
        { key: "name", label: "名称" }, { key: "url", label: "URL" },
        { key: "default_branch", label: "默认分支" }, { key: "description", label: "备注" },
      ],
      actions: [
        { label: "查看", permission: "repositories.read", to: (r) => ({ id: r.id, mode: "view" }) },
        { label: "编辑", permission: "repositories.update", to: (r) => ({ id: r.id, mode: "edit" }) },
        { label: "删除", permission: "repositories.delete", destructive: true,
          action: (r) => deleteResource(workspace, "repositories", r.id) },
      ],
    }),
    pipelines: list({
      resource: "pipelines", rowKey: (r: Pipeline) => r.id,
      filters: [{ key: "q", label: "名称", type: "text" }],
      columns: [
        { key: "name", label: "名称" }, { key: "repository_id", label: "仓库 ID" },
        { key: "target_kind", label: "目标类型", render: (r) => label(TARGETS, r.target_kind) },
        { key: "target_ref", label: "目标 ID" }, { key: "description", label: "备注" },
      ],
      actions: [
        { label: "查看", permission: "pipelines.read", to: (r) => ({ id: r.id, mode: "view" }) },
        { label: "触发", permission: "pipelines.run", action: async (r) => {
          const run = await runItemAction<{ id: string }>(workspace, "pipelines", r.id, "run", {});
          const group = nav.find((g) => g.items.some((i) => i.resource === "pipeline-runs"))!;
          const item = group.items.find((i) => i.resource === "pipeline-runs")!;
          navigate({ workspace, group: group.key, item: item.key, id: run.id, mode: "view" }, {});
        } },
        { label: "编辑", permission: "pipelines.update", to: (r) => ({ id: r.id, mode: "edit" }) },
        { label: "删除", permission: "pipelines.delete", destructive: true,
          action: (r) => deleteResource(workspace, "pipelines", r.id) },
      ],
    }),
    "pipeline-runs": list({
      resource: "pipeline-runs", rowKey: (r: PipelineRun) => r.id,
      filters: [{ key: "state", label: "状态", type: "select", options:
        ["queued", "running", "succeeded", "failed", "cancelled"].map((value) => ({ value, label: stateBadge(value).label })) }],
      columns: [
        { key: "number", label: "执行序号", sortable: true, render: (r) => `#${r.number}` },
        { key: "pipeline_id", label: "流水线 ID" },
        { key: "state", label: "状态", render: (r) => {
          const { label, ...props } = stateBadge(r.state);
          return <Badge {...props}>{label}</Badge>;
        } },
        { key: "triggered_by", label: "触发人 ID" },
        { key: "duration", label: "耗时", render: (r) => duration(r.started_at, r.finished_at) },
        { key: "started_at", label: "开始时间", render: (r) => r.started_at ? new Date(r.started_at).toLocaleString("zh-CN") : "—" },
      ],
      actions: [{ label: "查看", permission: "pipelines.read", to: (r) => ({ id: r.id, mode: "view" }) }],
      toolbar: { create: false, import: false, export: false },
    }),
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

    credentials: list({
      resource: "credentials",
      rowKey: (row: Credential) => row.id,
      filters: [
        { key: "q", label: "名称/账号", type: "text", placeholder: "输入关键字" },
        { key: "kind", label: "类型", type: "select", options: CREDENTIAL_KINDS },
      ],
      columns: [
        { key: "name", label: "名称" },
        { key: "kind", label: "类型", width: 120, render: (r: Credential) => label(CREDENTIAL_KINDS, r.kind) },
        { key: "username", label: "账号", width: 160 },
        {
          key: "has_secret",
          label: "内容",
          width: 100,
          render: (r: Credential) =>
            r.has_secret ? (
              <Badge variant="secondary">已设置</Badge>
            ) : (
              <Badge variant="destructive">未设置</Badge>
            ),
        },
      ],
      actions: [
        { label: "编辑", permission: "credentials.update", to: (r: Credential) => ({ id: r.id, mode: "edit" }) },
        {
          label: "删除",
          permission: "credentials.delete",
          destructive: true,
          action: (r: Credential) => deleteResource(workspace, "credentials", r.id),
        },
      ],
      toolbar: { import: false, export: false },
      empty: "还没有凭据。SSH 主机和 Git 仓库都从这里取登录信息。",
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
    deployments: form({
      resource: "deployments", heading: (r: Deployment) => r.name,
      blank: { name: "", mode: "compose", target_kind: "host", target_ref: "", workdir: "/opt/deployments", description: "", manifest: "apiVersion: ferry/v1\nkind: Application\nmetadata:\n  name: web\nservices:\n  - name: web\n    image: nginx:alpine\n    ports:\n      - container: 80\n        published: 8080\n" },
      sections: [{ fields: [
        { key: "name", label: "名称", type: "text", required: true },
        { key: "mode", label: "部署模式", type: "select", options: [{ value: "compose", label: "Docker Compose" }], immutable: true },
        { key: "target_kind", label: "目标类型", type: "select", options: [{ value: "host", label: "单主机" }], immutable: true },
        { key: "target_ref", label: "目标主机", type: "select", options: namedOptions(workspace, "hosts"), required: true, immutable: true },
        { key: "workdir", label: "部署工作目录", type: "text", required: true, immutable: true, help: "目标 Linux 主机上的可写绝对路径；自动按工作空间和应用隔离子目录。需要 Docker Compose v2+ 和 flock。" },
        { key: "manifest", label: "应用 manifest（YAML / JSON）", type: "code", required: true, span: 2, help: "首批支持镜像、多服务、端口、普通环境变量、命名卷和依赖；不支持密钥变量、配置模板、artifact 或滚动更新。不要填写密钥明文。" },
        { key: "description", label: "备注", type: "textarea", span: 2 },
      ] }],
    }),
    repositories: form({
      resource: "repositories", heading: (r: Repository) => r.name,
      blank: { name: "", url: "", default_branch: "main", credential_id: "0", description: "" },
      sections: [{ fields: [
        { key: "name", label: "名称", type: "text", required: true },
        { key: "url", label: "URL", type: "text", required: true },
        { key: "default_branch", label: "默认分支", type: "text", required: true },
        { key: "credential_id", label: "凭据", type: "select", options: async () => [
          { value: "0", label: "无凭据" }, ...await namedOptions(workspace, "credentials")(),
        ] },
        { key: "description", label: "备注", type: "textarea", span: 2 },
      ] }],
    }),
    pipelines: form({
      resource: "pipelines", heading: (r: Pipeline) => r.name,
      blank: { name: "", repository_id: "", target_kind: "host", target_ref: "", description: "",
        definition: JSON.stringify({ workdir: "/tmp/build-{{run}}", steps: [{ name: "构建", run: "echo hello", timeout_seconds: 1800 }] }, null, 2) },
      sections: [{ fields: [
        { key: "name", label: "名称", type: "text", required: true },
        { key: "repository_id", label: "仓库", type: "select", options: namedOptions(workspace, "repositories") },
        { key: "target_kind", label: "目标类型", type: "select", options: TARGETS, resetOnChange: ["target_ref"] },
        { key: "target_ref", label: "目标主机", type: "select", required: true,
          options: namedOptions(workspace, "hosts"), when: (d) => d.target_kind === "host" },
        { key: "target_ref", label: "目标主机组", type: "select", required: true,
          options: namedOptions(workspace, "host-groups"), when: (d) => d.target_kind === "host-group" },
        { key: "definition", label: "定义（JSON）", type: "code", required: true, span: 2,
          help: "逐步执行；服务端会检查定义并返回具体错误。" },
        { key: "description", label: "备注", type: "textarea", span: 2 },
      ] }],
    }),
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
      blank: { name: "", connection: "runner", address: "", port: 22, username: "", group_id: "", credential_id: "", description: "" },
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
            {
              key: "credential_id",
              label: "登录凭据",
              type: "select",
              required: true,
              options: sshCredentialOptions(workspace),
              span: 2,
              help: "在「构建 → 授权认证」里维护。密钥与密码只存在服务端，连接时才解密。",
              when: (draft: Partial<Host>) => draft.connection === "ssh",
            },
          ],
        },
        {
          fields: [{ key: "description", label: "备注", type: "textarea", span: 2 }],
        },
      ],
    }),

    credentials: form({
      resource: "credentials",
      heading: (r: Credential) => r.name,
      blank: { name: "", kind: "password", username: "", description: "", secret: "" },
      sections: [
        {
          fields: [
            { key: "name", label: "名称", type: "text", required: true, placeholder: "例如 生产机 root" },
            {
              key: "kind",
              label: "类型",
              type: "select",
              required: true,
              options: CREDENTIAL_KINDS,
              immutable: true,
              help: "创建后不可更改：改类型等于换一条凭据。",
            },
            { key: "username", label: "账号", type: "text", placeholder: "root" },
            {
              key: "secret",
              label: "内容",
              type: "secret",
              span: 2,
              help: "密码、私钥全文或令牌。保存后服务端加密存放，任何接口都不会再返回它；编辑时留空表示不修改。",
            },
            { key: "description", label: "备注", type: "textarea", span: 2 },
          ],
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
