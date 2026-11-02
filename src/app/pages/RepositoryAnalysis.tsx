import { useEffect,useMemo,useState } from "react";
import { useTranslation } from "react-i18next";
import { analysisConfig,analysisDatePresets,analysisReport,analysisReports,dayMs,overviewPeriods,overviewRanking } from "../../features/code-insights/analysis-config";
import { insightText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { ResourceForm } from "../resources/ResourceForm";
import { ResourceTable } from "../resources/ResourceTable";
import { useQueryValue } from "../router";
export * from "../../features/code-insights/analysis-config";

export function RepositoryAnalysis({ workspace, connectionId, permissions, scope = "connection" }: { workspace: string; connectionId: string; permissions: Set<string>; scope?: "connection" | "insight" }) {
  useTranslation("insight-domain");
  const [report, setReport] = useQueryValue("analysis_report", "overview");
  const kind = analysisReports.some(([key]) => key === report) ? report : "overview";
  const [utcDay, setUtcDay] = useState(() => new Date().toISOString().slice(0, 10));
  useEffect(() => {
    const timer = window.setTimeout(() => setUtcDay(new Date().toISOString().slice(0, 10)), Date.parse(`${utcDay}T00:00:00Z`) + dayMs - Date.now() + 100);
    return () => clearTimeout(timer);
  }, [utcDay]);
  const form = useMemo(() => analysisConfig(workspace, connectionId, permissions), [workspace, connectionId, permissions]);
  const list = useMemo(() => analysisReport(workspace, connectionId, kind, scope), [workspace, connectionId, kind, scope]);
  const rankings = useMemo(() => overviewPeriods.map(period => ({ period, config: overviewRanking(workspace, connectionId, period, new Date(`${utcDay}T00:00:00Z`), scope) })), [workspace, connectionId, utcDay, scope]);
  return <div className={scope === "insight" ? "flex min-w-0 shrink-0 flex-col gap-5" : "flex min-h-0 flex-1 flex-col gap-5 overflow-auto"}>
    {scope === "connection" && <ResourceForm config={form} workspace={workspace} permissions={permissions} showViewNavigation={false} />}
    <section className="flex min-h-[48rem] flex-1 flex-col gap-3" aria-label={insightText("贡献分析报告")}>
      <h3 className="font-semibold">{insightText("贡献分析报告")}</h3>
      <p className="text-sm leading-6 text-muted-foreground">{insightText("按仓库与 SHA 去重，仅统计已完整采集的提交。日期筛选使用提交时间，日历与周期分组使用作者时间（UTC）。默认排除合并提交；已观察历史包含删除分支和强推前采集的事实。每次提交的有效变更为新增行减去删除行，最低为零。")}</p>
      <div className="flex flex-wrap gap-2" aria-label={insightText("报告维度")}>{analysisReports.map(([key, label]) => <Button key={key} size="sm" variant={kind === key ? "secondary" : "outline"} aria-pressed={kind === key} onClick={() => setReport(key)}>{insightText(label)}</Button>)}</div>
      <ResourceTable key={`${connectionId}/${kind}`} config={list} workspace={workspace} permissions={permissions} />
    </section>
    {kind === "overview" && <section aria-label={insightText("六个周期贡献排名")} className="space-y-4">
      <h3 className="font-semibold">{insightText("周期贡献排名（UTC）")}</h3>
      <p className="text-sm text-muted-foreground">{insightText("今天、昨天、本周、上周、本月与上月分别统计。每个榜单独立筛选与分页，按有效变更排名；周从周一开始。")}</p>
      <div className="grid min-w-0 gap-6 xl:grid-cols-2">{rankings.map(({ period, config }) => <section key={`${connectionId}/${period}/${utcDay}`} aria-label={insightText("periodRanking", { period: insightText(analysisDatePresets.find(([key]) => key === period)![1]) })} className="flex min-h-[40rem] min-w-0 flex-col gap-3 rounded-md border p-3">
        <h4 className="font-medium">{insightText(analysisDatePresets.find(([key]) => key === period)![1])} · {config.fixed!.since} {insightText("至")}{config.fixed!.until}</h4>
        <ResourceTable config={config} workspace={workspace} permissions={permissions} />
      </section>)}</div>
    </section>}
  </div>;
}
