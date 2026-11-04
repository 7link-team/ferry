import { TagBadge } from "../tags/TagBadge";
import { HostAgentVersion } from "./RunnerConnection";
import { defaultExecutionPolicy, ExecutionPolicyFields, type ExecutionPolicy, type ExecutionStatus } from "./ExecutionPolicy";
import { FolderOpen, SquareTerminal } from "lucide-react";
import {
  createResource,
  deleteResource,
  listResource,
  runAction,
  runItemAction,
} from "../../shared/api/resource";
import { commonText, executionText } from "../../shared/i18n/i18n";
import { currentLocale, localizedPath } from "../../shared/i18n/locale";
import type { ResourceConfigContext } from "../../shared/resources/config";
import {
  ago,
  type AnyRow,
  form,
  label,
  list,
  namedOptions,
} from "../../shared/resources/config";
import type {
  FormConfig,
  ListConfig,
  Option,
} from "../../shared/resources/types";
import { Badge } from "../../shared/ui/badge";
import {
  CREDENTIAL_KINDS,
  sshCredentialOptions,
} from "../credentials/resource-config";
import { tagOptions } from "../tags/resource-config";

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
  runner_auto_update: "inherit" | "on" | "off";
  state: string;
  group_id: string;
  tag_ids: string[];
  tags: { id: string; name: string; color: string }[];
  credential_id: string;
  os?: string;
  agent_version?: string;
  agent_update_available?: boolean;
  agent_update_state?: string;
  jump_host_id: string;
  description: string;
  seen_at: number;
  version: number;
  execution?: ExecutionPolicy;
  execution_status?: ExecutionStatus;
}

export const CONNECTIONS: Option[] = [
  {
    value: "runner",
    label: "主机代理",
    labelKey: "resource-domain:host.runner",
  },
  {
    value: "ssh",
    label: "SSH 直连",
    labelKey: "resource-domain:host.directSsh",
  },
];

export const STATES: Option[] = [
  {
    value: "online",
    label: "在线",
    labelKey: "common-domain:dashboard.online",
  },
  {
    value: "offline",
    label: "离线",
    labelKey: "common-domain:dashboard.offline",
  },
  {
    value: "unknown",
    label: "未知",
    labelKey: "common-domain:dashboard.unknown",
  },
];

export const sshJumpOptions =
  (workspace: string) => async (): Promise<Option[]> => {
    const choices: Option[] = [];
    for (let page = 1; ; page++) {
      const result = await listResource<Host>(workspace, "hosts", {
        page,
        page_size: 200,
        connection: "ssh",
      });
      choices.push(
        ...result.items
          .filter((host) => !host.jump_host_id || host.jump_host_id === "0")
          .map((host) => ({ value: host.id, label: host.name })),
      );
      if (page * result.page_size >= result.total) break;
    }
    return choices;
  };

export const groupOptions = (workspace: string) =>
  namedOptions(workspace, "host-groups");

export const TARGETS = [
  { value: "host", label: "主机", labelKey: "common-domain:nav.hosts" },
  {
    value: "host-group",
    label: "主机组",
    labelKey: "common-domain:nav.hostGroups",
  },
];

export function hostsLists({
  workspace,
  hostStates,
  permissions,
}: ResourceConfigContext & { hostStates: Option[] }): Record<string, ListConfig<AnyRow>> {
  const stateOptions = hostStates.map(option => option.color ? {
    ...option, content: <TagBadge color={option.color}>{label(hostStates, option.value)}</TagBadge>,
  } : option);
  return {
    "host-groups": list({
      resource: "host-groups",
      rowKey: (row: HostGroup) => row.id,
      filters: [
        {
          key: "q",
          label: "名称/说明",
          labelKey: "resource-domain:resource.searchNameDescription",
          type: "text",
          placeholder: "输入关键字",
          placeholderKey: "resource-domain:resource.keyword",
        },
      ],
      columns: [
        { key: "name", label: "名称", labelKey: "common-domain:common.name" },
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
          permission: "host-groups.read",
          to: (row: HostGroup) => ({ id: row.id, mode: "view" }),
        },
        {
          label: "编辑",
          labelKey: "common-domain:common.edit",
          permission: "host-groups.update",
          to: (row: HostGroup) => ({ id: row.id, mode: "edit" }),
        },
        {
          label: "删除",
          labelKey: "common-domain:common.delete",
          permission: "host-groups.delete",
          destructive: true,
          action: (row: HostGroup) =>
            deleteResource(workspace, "host-groups", row.id),
        },
      ],
      empty: "还没有主机组。先建一个，再把主机归进去。",
      emptyKey: "resource-domain:hostGroup.empty",
    }),
    hosts: list({
      resource: "hosts",
      defaultRefresh: 10,
      toolbar: {
        create: { label: "添加主机", labelKey: "common-domain:form.addHost" },
      },
      rowKey: (row: Host) => row.id,
      filters: [
        {
          key: "tag_id",
          label: "标签",
          labelKey: "common-domain:nav.tags",
          type: "select",
          options: tagOptions(workspace),
        },
        {
          key: "q",
          label: "名称/地址/账号",
          labelKey: "resource-domain:host.search",
          type: "text",
          placeholder: "输入关键字",
          placeholderKey: "resource-domain:resource.keyword",
        },
        {
          key: "group_id",
          label: "主机组",
          labelKey: "common-domain:nav.hostGroups",
          type: "select",
          options: groupOptions(workspace),
        },
        {
          key: "connection",
          label: "接入方式",
          labelKey: "common-domain:form.connection",
          type: "select",
          options: CONNECTIONS,
        },
        {
          key: "state",
          label: "状态",
          labelKey: "resource-domain:status.title",
          type: "select",
          options: stateOptions,
        },
      ],
      columns: [
        { key: "name", label: "名称", labelKey: "common-domain:common.name" },
        {
          key: "tags",
          label: "标签",
          labelKey: "common-domain:nav.tags",
          render: (row: Host) => (
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
          key: "address",
          label: "地址",
          labelKey: "resource-domain:host.address",
          width: 180,
        },
        {
          key: "connection",
          label: "接入方式",
          labelKey: "common-domain:form.connection",
          width: 200,
          render: (row: Host) => <div className="space-y-1"><div>{label(CONNECTIONS, row.connection)}</div>{row.connection === "runner" && <HostAgentVersion workspace={workspace} hostId={row.id} version={row.agent_version} available={row.agent_update_available} updateState={row.agent_update_state} online={row.state === "online"} canUpdate={permissions?.has("hosts.update") ?? false} />}</div>,
        },
        {
          key: "state",
          label: "状态",
          labelKey: "resource-domain:status.title",
          width: 90,
          render: (row: Host) => {
            const text = row.connection === "runner" && row.state === "offline"
              ? commonText("resource-domain:runner.disconnected")
              : label(hostStates, row.state);
            const color = hostStates.find(option => option.value === row.state)?.color;
            return color ? <TagBadge color={color}>{text}</TagBadge> : (
            <Badge
              variant={
                row.state === "online"
                  ? "default"
                  : row.state === "offline"
                    ? "destructive"
                    : "secondary"
              }
            >
              {text}
            </Badge>
            );
          },
        },
        {
          key: "execution_status",
          label: "执行容量",
          labelKey: "execution-domain:hostExecution.title",
          render: (row: Host) => row.execution_status ? <div className="space-y-1">{row.execution?.shared && <Badge variant="outline">{executionText("hostExecution.sharedBadge")}</Badge>}<Badge variant="secondary">{executionText(`hostExecution.${row.state === "online" ? row.execution_status.state : "unavailable"}`)}</Badge><div className="text-xs tabular-nums text-muted-foreground">{row.execution_status.active} / {row.execution_status.capacity}</div>{permissions?.has("pipelines.read") && row.execution_status.running_jobs.map((job, index) => <a key={`${job.run_id}:${job.job_key}:${index}`} className="block max-w-48 truncate text-xs text-primary underline underline-offset-2" href={localizedPath(`/workspaces/${workspace}/pipeline-runs/${job.run_id}`, currentLocale())}>{job.job_key} · {job.run_id}</a>)}</div> : "—",
        },
        {
          key: "seen_at",
          label: "最近在线",
          labelKey: "resource-domain:host.recentlyOnline",
          width: 150,
          sortable: true,
          render: (row: Host) =>
            row.seen_at
              ? ago(row.seen_at)
              : commonText("resource-domain:host.neverConnected"),
        },
      ],
      actions: [
        {
          label: "查看",
          labelKey: "common-domain:common.view",
          permission: "hosts.read",
          to: (row: Host) => ({ id: row.id, mode: "view" }),
        },
        {
          label: "打开终端",
          labelKey: "common-domain:shell.openTerminal",
          icon: <SquareTerminal className="size-4" />,
          newTab: true,
          permission: "hosts.terminal",
          to: (row: Host) => ({ id: row.id, mode: "workspace" }),
          query: () => ({ tool: "terminal" }),
        },
        {
          label: "打开文件",
          labelKey: "common-domain:shell.openFiles",
          icon: <FolderOpen className="size-4" />,
          newTab: true,
          permission: "hosts.files",
          to: (row: Host) => ({ id: row.id, mode: "workspace" }),
          query: () => ({ tool: "files" }),
        },
        {
          label: "测试连接",
          labelKey: "resource-domain:host.testConnection",
          permission: "hosts.read",
          action: async (row: Host) => {
            const result = await runItemAction<{ ok: boolean; stderr: string }>(
              workspace,
              "hosts",
              row.id,
              "test-connection",
              {},
            );
            if (!result.ok)
              throw new Error(
                result.stderr || commonText("resource-domain:host.testFailed"),
              );
          },
        },
        {
          label: "编辑",
          labelKey: "common-domain:common.edit",
          permission: "hosts.update",
          to: (row: Host) => ({ id: row.id, mode: "edit" }),
        },
        {
          label: "删除",
          labelKey: "common-domain:common.delete",
          permission: "hosts.delete",
          destructive: true,
          action: (row: Host) => deleteResource(workspace, "hosts", row.id),
        },
      ],
      bulk: [
        {
          label: "批量删除",
          labelKey: "resource-domain:resource.bulkDelete",
          permission: "hosts.delete",
          destructive: true,
          action: async (ids) => {
            await runAction(workspace, "hosts", "bulk-delete", { ids });
          },
        },
      ],
      empty: "还没有主机。用代理一条命令接入，或填 SSH 账号直连。",
      emptyKey: "resource-domain:host.empty",
    }),
  };
}

export function hostsForms({
  workspace,
}: ResourceConfigContext): Record<string, FormConfig<AnyRow>> {
  return {
    "host-groups": form({
      resource: "host-groups",
      heading: (row: HostGroup) => row.name,
      blank: { name: "", description: "", parent_id: "" },
      sections: [
        {
          fields: [
            {
              key: "name",
              label: "名称",
              labelKey: "common-domain:common.name",
              type: "text",
              required: true,
              placeholder: "例如 生产-华东",
              placeholderKey: "resource-domain:hostGroup.example",
            },
            {
              key: "parent_id",
              label: "上级主机组",
              labelKey: "resource-domain:hostGroup.parent",
              type: "select",
              emptyValues: ["0", 0],
              emptyLabelKey: "resource-domain:reference.topLevel",
              options: groupOptions(workspace),
              help: "留空表示顶级",
              helpKey: "resource-domain:hostGroup.parentHelp",
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
    hosts: form({
      resource: "hosts",
      heading: (row: Host) => row.name,
      blank: {
        name: "",
        connection: "runner",
        runner_auto_update: "inherit",
        address: "",
        port: 22,
        username: "",
        group_id: "",
        tag_ids: [],
        execution: { ...defaultExecutionPolicy },
        credential_id: "",
        jump_host_id: "",
        description: "",
      },
      sections: [
        {
          title: "基本信息",
          titleKey: "common-domain:common.basic",
          fields: [
            {
              key: "name",
              label: "名称",
              labelKey: "common-domain:common.name",
              type: "text",
              required: true,
              placeholder: "例如 web-01",
              placeholderKey: "resource-domain:host.nameExample",
            },
            {
              key: "runner_auto_update",
              label: "Runner 自动升级",
              labelKey: "resource-domain:runner.autoUpdate",
              type: "select",
              options: [
                {
                  value: "inherit",
                  label: "跟随平台默认",
                  labelKey: "resource-domain:runner.followDefault",
                },
                {
                  value: "on",
                  label: "启用",
                  labelKey: "resource-domain:status.enabled",
                },
                {
                  value: "off",
                  label: "关闭",
                  labelKey: "resource-domain:status.off",
                },
              ],
              when: (row: Partial<Host>) => row.connection === "runner",
              help: "等待完整任务结束后升级到当前服务版本，期间暂停新任务。",
              helpKey: "resource-domain:runner.updateWaitHelp",
            },
            {
              key: "group_id",
              label: "主机组",
              labelKey: "common-domain:nav.hostGroups",
              type: "select",
              emptyValues: ["0", 0],
              emptyLabelKey: "resource-domain:reference.ungrouped",
              options: groupOptions(workspace),
            },
            {
              key: "tag_ids",
              label: "标签",
              labelKey: "common-domain:nav.tags",
              type: "multiselect",
              options: tagOptions(workspace),
              span: 2,
              help: "可选择多个标签；取消全部选择会清空关联。",
              helpKey: "resource-domain:host.tagsHelp",
            },
            {
              key: "connection",
              label: "接入方式",
              labelKey: "common-domain:form.connection",
              type: "select",
              required: true,
              options: CONNECTIONS,
              immutable: true,
              help: "创建后不可更改：改了会让已注册的代理密钥或 SSH 凭据失效。",
              helpKey: "resource-domain:host.connectionImmutable",
            },
          ],
        },
        {
          title: "流水线执行资源",
          titleKey: "execution-domain:hostExecution.section",
          fields: [{
            key: "execution", label: "执行容量", labelKey: "execution-domain:hostExecution.title", type: "custom", span: 2,
            render: ({ draft, readOnly, onPatch, onValidityChange }) => <ExecutionPolicyFields value={draft.execution} readOnly={readOnly} onChange={execution => onPatch({ execution })} onValidityChange={onValidityChange} />,
          }],
        },
        {
          title: "SSH 连接",
          titleKey: "resource-domain:host.sshSection",
          fields: [
            {
              key: "address",
              label: "主机地址",
              labelKey: "resource-domain:host.hostAddress",
              type: "text",
              required: true,
              placeholder: "10.0.0.11 或 host.example.com",
              placeholderKey: "resource-domain:host.addressExample",
              when: (draft: Partial<Host>) => draft.connection === "ssh",
            },
            {
              key: "port",
              label: "端口",
              labelKey: "resource-domain:host.port",
              type: "number",
              required: true,
              when: (draft: Partial<Host>) => draft.connection === "ssh",
            },
            {
              key: "username",
              label: "登录账号",
              labelKey: "resource-domain:host.loginAccount",
              type: "text",
              required: true,
              placeholder: "root",
              when: (draft: Partial<Host>) => draft.connection === "ssh",
            },
            {
              key: "jump_host_id",
              label: "跳板主机",
              labelKey: "resource-domain:host.jumpHost",
              type: "select",
              emptyValues: ["0", 0],
              emptyLabelKey: "resource-domain:reference.noJumpHost",
              options: sshJumpOptions(workspace),
              optionFilter: (option, draft: Partial<Host>) => option.value !== draft.id,
              help: "可选。先添加一台可直连的 SSH 主机，多个目标主机可共用它。目标主机仍使用自己的登录凭据。",
              helpKey: "resource-domain:host.jumpHelp",
              when: (draft: Partial<Host>) => draft.connection === "ssh",
            },
            {
              key: "credential_id",
              label: "登录凭据",
              labelKey: "resource-domain:host.loginCredential",
              type: "select",
              required: true,
              options: sshCredentialOptions(workspace),
              span: 2,
              help: "密码与私钥由服务端加密保存，连接时才解密。",
              helpKey: "resource-domain:host.credentialHelp",
              createOption: {
                label: "添加登录凭据",
                labelKey: "resource-domain:credential.addLogin",
                permission: "credentials.create",
                fields: [
                  {
                    key: "name",
                    label: "凭据名称",
                    labelKey: "resource-domain:credential.name",
                    type: "text",
                    required: true,
                  },
                  {
                    key: "kind",
                    label: "类型",
                    labelKey: "resource-domain:resource.type",
                    type: "select",
                    required: true,
                    options: CREDENTIAL_KINDS.filter(
                      (option) =>
                        option.value === "password" ||
                        option.value === "ssh-key",
                    ),
                  },
                  {
                    key: "secret",
                    label: "密码",
                    labelKey: "common-domain:auth.password",
                    type: "password",
                    required: true,
                    span: 2,
                    when: (values) => values.kind === "password",
                  },
                  {
                    key: "secret",
                    label: "SSH 私钥",
                    labelKey: "resource-domain:credential.sshKey",
                    type: "secret",
                    required: true,
                    span: 2,
                    when: (values) => values.kind === "ssh-key",
                  },
                ],
                create: async (values) => {
                  const row = await createResource<{ id: string }>(
                    workspace,
                    "credentials",
                    { ...values, scope: "host", username: "", description: "" },
                  );
                  return { value: row.id, label: String(values.name) };
                },
              },
              when: (draft: Partial<Host>) => draft.connection === "ssh",
            },
          ],
        },
        {
          fields: [
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
