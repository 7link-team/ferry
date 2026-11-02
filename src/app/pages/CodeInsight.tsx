import { Tabs } from "radix-ui";
import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import { codeInsightForm,codeInsightList } from "../../features/code-insights/config";
import { insightText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { ResourceForm } from "../resources/ResourceForm";
import { ResourceTable } from "../resources/ResourceTable";
import { href,useQueryValue } from "../router";
import { RepositoryAnalysis } from "./RepositoryAnalysis";
export * from "../../features/code-insights/config";

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
