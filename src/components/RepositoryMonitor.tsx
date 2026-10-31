import { useMemo } from "react";
import { Bell, Settings2 } from "lucide-react";
import type { FormConfig, ListConfig } from "./dsl";
import { ResourceForm } from "./ResourceForm";
import { ResourceTable } from "./ResourceTable";
import { patchResource, base } from "../resource";

export const repositoryEventOptions = [
  ["repository.created", "新增仓库"], ["repository.deleted", "删除仓库"],
  ["branch.created", "新增分支"], ["branch.deleted", "删除分支"],
  ["push", "代码推送"], ["pull_request", "PR / 合并请求变更"], ["merge", "合并完成"],
  ["tag.created", "新增标签"], ["tag.deleted", "删除标签"],
].map(([value, label]) => ({ value: value!, label: label! }));

export function RepositoryMonitor({ workspace, connectionId, permissions }: { workspace: string; connectionId: string; permissions: Set<string> }) {
  const form = useMemo<FormConfig<Record<string, unknown>>>(() => ({
    resource: "code-connection-monitors", permissionResource: "repositories", width: "full", blank: {}, heading: () => "事件订阅", readOnly: () => true,
    sections: [{ title: "订阅配置", fields: [
      { key: "enabled", label: "启用监控", type: "switch" },
      { key: "all_repositories", label: "全部仓库", type: "switch" },
      { key: "repositories", label: "指定仓库", type: "textarea", when: row => !row.all_repositories },
      { key: "events", label: "订阅事件", type: "multiselect", options: repositoryEventOptions, span: 2 },
      { key: "capability_note", label: "平台支持范围", type: "text", span: 2 },
      { key: "callback_path", label: "平台事件接收地址", type: "text", span: 2, help: `在代码平台填写可公开访问的服务地址加此路径。当前地址：${location.origin}。127.0.0.1 无法被云平台访问。` },
      { key: "secret", label: "事件密钥", type: "secret", stored: "has_secret" },
      { key: "webhook", label: "通知 Webhook", type: "secret", stored: "has_webhook" },
      { key: "provider_changed", label: "连接平台已变更，需要重新保存监控配置", type: "switch", when: row => !!row.provider_changed },
    ] }],
    viewActions: [{
      label: "配置监控", icon: <Settings2 className="size-4" />, permission: "repositories.update",
      when: row => row.provider !== "git",
      description: "保存后生效。事件密钥和通知地址加密保存，留空保留原值。",
      initialValues: row => ({ enabled: row.enabled, all_repositories: row.all_repositories, repositories: row.repositories, events: Array.isArray(row.events) ? row.events.filter(event => row.provider !== "codeup" || !String(event).startsWith("repository.")) : [] }),
      parameters: row => [
        { key: "enabled", label: "启用监控", type: "switch" },
        { key: "all_repositories", label: "全部仓库", type: "switch" },
        { key: "repositories", label: "指定仓库路径", type: "textarea", when: value => !value.all_repositories, required: true, help: "从仓库列表复制完整路径，每行一个，例如 team/project。精确匹配，支持多级命名空间。" },
        { key: "events", label: "订阅事件", type: "multiselect", options: repositoryEventOptions.filter(option => row.provider !== "codeup" || !option.value.startsWith("repository.")), required: true },
        { key: "secret", label: "事件密钥", type: "password", required: !row.has_secret, help: "16–256 位 ASCII 字符。在代码平台 Webhook 中填写同一密钥；更换后旧密钥立即失效。" },
        { key: "webhook", label: "通知 Webhook 地址", type: "password", help: "可选。HTTP(S) JSON 接收地址；每个事件投递一次，失败记录原因且不重试。留空时仍记录事件。" },
        { key: "clear_webhook", label: "清除已保存的通知地址", type: "switch", when: () => !!row.has_webhook },
      ],
      action: async (row, values) => { await patchResource(workspace, "code-connection-monitors", connectionId, { ...values, version: row.version }); },
    }],
  }), [workspace, connectionId]);
  const events = useMemo<ListConfig<Record<string, unknown>>>(() => ({
    resource: "repository-events", permissionResource: "repositories", endpoint: `${base(workspace, "code-connections")}/${encodeURIComponent(connectionId)}/events`, rowKey: row => String(row.id), defaultRefresh: 15,
    filters: [{ key: "q", label: "仓库 / 分支 / 操作人", type: "text" }, { key: "event", label: "事件", type: "select", options: repositoryEventOptions }],
    columns: [
      { key: "created_at", label: "接收时间", sortable: true, render: row => new Date(Number(row.created_at)).toLocaleString() },
      { key: "repository", label: "仓库", sortable: true },
      { key: "event", label: "事件", render: row => repositoryEventOptions.find(option => option.value === row.event)?.label ?? String(row.event) },
      { key: "reference", label: "分支 / 标签" }, { key: "action", label: "动作" }, { key: "actor", label: "操作人" }, { key: "delivery", label: "通知结果" },
    ], toolbar: { create: false, import: false, export: false }, empty: "尚未收到匹配事件。请先保存配置，并在代码平台启用 Webhook。",
  }), [workspace, connectionId]);
  return <div className="flex min-h-0 flex-1 flex-col gap-5 overflow-auto">
    <div className="rounded-lg border bg-muted/30 p-4 text-sm leading-6 text-muted-foreground">
      <p className="flex items-center gap-2 font-medium text-foreground"><Bell className="size-4" />接收代码平台事件，通知到你的 Webhook</p>
      <p>在代码平台创建 JSON Webhook，填写本页接收地址和事件密钥，并勾选对应事件。新增、删除仓库需要组织级或系统级 Webhook；仅配置单仓库 Webhook 无法覆盖新仓库。</p>
      <p>GitHub 使用 SHA256 签名；Gitea 使用 X-Gitea-Signature；GitLab 使用 Secret Token。Codeup 使用 X-Codeup-Token，Gitee 使用密码模式的 X-Gitee-Token。平台未提供的事件不会由本服务推断或补造。</p>
    </div>
    <ResourceForm config={form} workspace={workspace} permissions={permissions} showViewNavigation={false} />
    <section className="flex min-h-80 flex-1 flex-col gap-3" aria-label="仓库事件记录"><h3 className="text-base font-semibold">事件记录</h3><ResourceTable config={events} workspace={workspace} permissions={permissions} /></section>
  </div>;
}
