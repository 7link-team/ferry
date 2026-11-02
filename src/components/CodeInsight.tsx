import { useTranslation } from "react-i18next";
import { insightText } from "../i18n";
import { currentLocale } from "../locale";
import { useMemo } from "react";
import { Tabs } from "radix-ui";
import type { FormConfig, ListConfig } from "./dsl";
import { ResourceForm } from "./ResourceForm";
import { ResourceTable } from "./ResourceTable";
import { RepositoryAnalysis, analysisState } from "./RepositoryAnalysis";
import { Button } from "./ui/button";
import { createResource, deleteResource, base } from "../resource";
import { href, useQueryValue } from "../router";

type Row = Record<string, unknown>;
const time = (value: unknown) => typeof value === "number" && value > 0 ? new Date(value).toLocaleString(currentLocale()) : "—";
const coverage = (value: unknown) => value === 0 ? insightText("完整历史") : time(value);
const updateMode = (value: unknown) => value === "event" ? insightText("事件更新") : insightText("定时更新");

export function codeInsightList(workspace: string, connectionId?: string): ListConfig<Row> {
  return {
    resource: "code-insights", rowKey: row => String(row.id),
    ...(connectionId ? { fixed: { connection_id: connectionId } } : {}),
    filters: [{ key: "q", label: "名称", labelKey: "insight-domain:名称", type: "text" }],
    columns: [{ key: "name", label: "洞察名称", labelKey: "insight-domain:洞察名称" }, { key: "description", label: "说明", labelKey: "insight-domain:说明" },
      { key: "repositories", label: "仓库数", labelKey: "insight-domain:仓库数", render: row => Array.isArray(row.repositories) ? row.repositories.length : String(row.repository_count ?? "—") },
      { key: "update_mode", label: "更新方式", labelKey: "insight-domain:更新方式", render: row => updateMode(row.update_mode) },
      { key: "enabled", label: "持续采集", labelKey: "insight-domain:持续采集", render: row => row.enabled ? insightText("已启用") : insightText("已停用") }],
    actions: [
      { label: "查看", labelKey: "insight-domain:查看", permission: "code-insights.read", to: row => ({ group: "build", item: "code-insights", mode: "view", id: String(row.id) }), query: () => ({}) },
      { label: "编辑", labelKey: "insight-domain:编辑", permission: "code-insights.update", to: row => ({ group: "build", item: "code-insights", mode: "edit", id: String(row.id) }), query: () => ({}) },
      { label: "删除", labelKey: "insight-domain:删除", permission: "code-insights.delete", destructive: true, action: async row => { await deleteResource(workspace, "code-insights", String(row.id)); } },
    ],
    toolbar: { create: connectionId ? false : { label: "创建洞察", labelKey: "insight-domain:创建洞察" }, import: false, export: false },
    empty: "尚无代码洞察。创建洞察并选择仓库后，即可独立管理作者映射和分析看板。", emptyKey: "insight-domain:尚无代码洞察。创建洞察并选择仓库后，即可独立管理作者映射和分析看板。",
  };
}

export function codeInsightForm(workspace: string): FormConfig<Row> {
  return {
    resource: "code-insights", width: "full", heading: row => String(row.name ?? insightText("代码洞察")),
    blank: { name: "", description: "", enabled: true, update_mode: "scheduled", schedule: "0 */4 * * *", timezone: "", full_history: false, initial_days: 90, reconcile_minutes: 1440, aliases: [], repositories: [] },
    pollMs: () => 15000,
    sections: [
      { title: "基本信息", titleKey: "insight-domain:基本信息", fields: [{ key: "name", label: "洞察名称", labelKey: "insight-domain:洞察名称", type: "text", required: true }, { key: "description", label: "说明", labelKey: "insight-domain:说明", type: "textarea", span: 2 }] },
      { title: "仓库范围", titleKey: "insight-domain:仓库范围", fullWidth: true, fields: [{ key: "repositories", label: "选择仓库", labelKey: "insight-domain:选择仓库", type: "connection-repositories", required: true, span: 2 }] },
      { title: "采集策略", titleKey: "insight-domain:采集策略", fields: [
        { key: "update_mode", label: "更新方式", labelKey: "insight-domain:更新方式", type: "select", required: true, options: [{ value: "scheduled", label: "定时更新", labelKey: "insight-domain:定时更新" }, { value: "event", label: "事件更新", labelKey: "insight-domain:事件更新" }], help: "定时更新按计划采集；事件更新在仓库发生变化时采集。", helpKey: "insight-domain:定时更新按计划采集；事件更新在仓库发生变化时采集。" },
        { key: "schedule", label: "执行周期", labelKey: "insight-domain:执行周期", type: "cron", required: true, span: 2, when: row => row.update_mode === "scheduled", help: "默认每 4 小时更新一次。", helpKey: "insight-domain:默认每 4 小时更新一次。" },
        { key: "timezone", label: "时区", labelKey: "insight-domain:时区", type: "text", placeholder: "UTC", when: row => row.update_mode === "scheduled", help: "留空按 UTC；Cron 按所选时区执行。可填写 IANA 时区，例如 Asia/Shanghai、Asia/Singapore。", helpKey: "insight-domain:留空按 UTC；Cron 按所选时区执行。可填写 IANA 时区，例如 Asia/Shanghai、Asia/Singapore。" },
        { key: "enabled", label: "持续采集", labelKey: "insight-domain:持续采集", type: "switch" }, { key: "full_history", label: "初始化完整历史", labelKey: "insight-domain:初始化完整历史", type: "switch" },
        { key: "initial_days", label: "初始化最近天数", labelKey: "insight-domain:初始化最近天数", type: "number", required: true, min: 1, when: row => !row.full_history },
        { key: "reconcile_minutes", label: "分支头核对间隔（分钟）", labelKey: "insight-domain:分支头核对间隔（分钟）", type: "number", required: true, min: 1, when: row => row.update_mode === "event", help: "事件更新的兜底核对，默认每天一次。", helpKey: "insight-domain:事件更新的兜底核对，默认每天一次。" },
      ] },
      { title: "作者映射", titleKey: "insight-domain:作者映射", fullWidth: true, fields: [{ key: "aliases", label: "作者昵称映射", labelKey: "insight-domain:作者昵称映射", type: "author-aliases", span: 2 }] },
      { title: "同步覆盖", titleKey: "insight-domain:同步覆盖", fullWidth: true, fields: [
        { key: "update_mode", label: "更新方式", labelKey: "insight-domain:更新方式", type: "text", immutable: true, when: row => !!row.id, display: row => updateMode(row.update_mode) },
        { key: "next_run_at", label: "下次计划更新", labelKey: "insight-domain:下次计划更新", type: "text", immutable: true, when: row => !!row.id && row.update_mode === "scheduled", display: row => time(row.next_run_at) },
        { key: "coverage_start", label: "已请求覆盖起点", labelKey: "insight-domain:已请求覆盖起点", type: "text", immutable: true, when: row => !!row.id, display: row => coverage(row.coverage_start) },
        { key: "last_error", label: "发现 / 核对错误", labelKey: "insight-domain:发现 / 核对错误", type: "textarea", immutable: true, when: row => !!row.id && !!row.last_error, span: 2 },
        { key: "sources", label: "各仓库覆盖与任务状态", labelKey: "insight-domain:各仓库覆盖与任务状态", type: "records", span: 2, when: row => !!row.id, record: {
          title: row => String(row.repository ?? ""),
          meta: row => insightText("sourceMeta", { state: analysisState(row.state), start: coverage(row.coverage_start), time: time(row.synced_at) }),
          body: row => { const job = row.job as Row | undefined; return [row.last_error, job && insightText("jobProgress", { state: analysisState(job.state), completed: job.completed_generation ?? 0, generation: job.generation ?? 0 }), job?.last_error].filter(Boolean).join("\n"); },
        } },
      ] },
    ],
    viewActions: [{ label: "重新核对 / 重试同步", labelKey: "insight-domain:重新核对 / 重试同步", permission: "code-insights.sync", action: async row => { await createResource(workspace, "code-insights", {}, `${base(workspace, "code-insights")}/${encodeURIComponent(String(row.id))}/sync`); } }],
  };
}

export function CodeInsightDetail({ workspace, insightId, permissions }: { workspace: string; insightId: string; permissions: Set<string> }) {
  useTranslation("insight-domain");
  const [tab, setTab] = useQueryValue("tab", "dashboard");
  const form = useMemo(() => codeInsightForm(workspace), [workspace]);
  const summary = useMemo(() => ({ ...form, sections: [form.sections[0]!, form.sections[4]!], readOnly: () => true }), [form]);
  return <Tabs.Root value={tab === "settings" ? "settings" : "dashboard"} onValueChange={setTab} className="flex min-h-0 flex-1 flex-col gap-4">
    <Tabs.List aria-label={insightText("代码洞察详情")} className="flex gap-2 border-b">
      <Tabs.Trigger value="dashboard" asChild><Button variant="ghost">{insightText("分析看板")}</Button></Tabs.Trigger>
      <Tabs.Trigger value="settings" asChild><Button variant="ghost">{insightText("配置与同步")}</Button></Tabs.Trigger>
    </Tabs.List>
    <Tabs.Content value="dashboard" className="flex min-h-0 flex-1 flex-col gap-4 overflow-auto"><div className="shrink-0"><ResourceForm config={summary} workspace={workspace} permissions={permissions} showViewNavigation={false} /></div><RepositoryAnalysis workspace={workspace} connectionId={insightId} scope="insight" permissions={permissions} /></Tabs.Content>
    <Tabs.Content value="settings" className="min-h-0 flex-1 overflow-auto"><ResourceForm config={form} workspace={workspace} permissions={permissions} /></Tabs.Content>
  </Tabs.Root>;
}

export function ConnectionInsights({ workspace, connectionId, permissions }: { workspace: string; connectionId: string; permissions: Set<string> }) {
  useTranslation("insight-domain");
  const list = useMemo(() => codeInsightList(workspace, connectionId), [workspace, connectionId]);
  return <section aria-label={insightText("相关代码洞察")} className="flex min-h-0 flex-1 flex-col gap-3">
    <p className="text-sm text-muted-foreground">{insightText("在代码洞察中选择本连接的仓库，独立配置作者映射并查看分析看板。")}</p>
    {permissions.has("code-insights.read") ? <>
      <a className="text-sm text-primary underline underline-offset-4" href={href({ group: "build", item: "code-insights", mode: "list", id: "" }, {})}>{insightText("管理代码洞察")}</a>
      <ResourceTable config={list} workspace={workspace} permissions={permissions} />
    </> : <p className="text-sm text-muted-foreground">{insightText("查看相关洞察需要代码洞察读取权限。")}</p>}
  </section>;
}
