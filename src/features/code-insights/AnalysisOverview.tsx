import { useEffect, useMemo, useState } from "react";
import { errorText } from "../../shared/api/api";
import { listResource, type Envelope } from "../../shared/api/resource";
import { insightText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { metricClass } from "./AnalysisTooltip";
import { AnalysisSummary, analysisDatePresets, analysisReport, overviewPeriods, overviewRanking, type Row } from "./analysis-config";

function useReport(workspace: string, config: ReturnType<typeof analysisReport>, conditions: Record<string, string>, refreshKey: number) {
  const [state, setState] = useState<{ data?: Envelope<Row>; error?: string; loading: boolean }>({ loading: true });
  useEffect(() => {
    const controller = new AbortController();
    setState({ loading: true });
    void listResource<Row>(workspace, config.resource, { ...conditions, ...config.fixed, page: 1, page_size: 10 }, controller.signal, config.endpoint)
      .then(data => { if (!controller.signal.aborted) setState({ data, loading: false }); })
      .catch(error => { if (!controller.signal.aborted) setState({ error: errorText(error), loading: false }); });
    return () => controller.abort();
  }, [workspace, config, conditions, refreshKey]);
  return state;
}

const metrics = ["commits", "additions", "deletions", "effectives"] as const;
const metricLabels = ["提交数", "新增行", "删除行", "有效变更"] as const;

function Bars({ state, metric, authors }: { state: ReturnType<typeof useReport>; metric: string; authors: boolean }) {
  if (state.loading) return <div role="status" className="h-28 animate-pulse rounded-md bg-muted/50" aria-label={insightText("正在加载分析数据")} />;
  if (state.error) return <p role="alert" className="py-6 text-sm text-destructive">{state.error}</p>;
  const rows = [...(state.data?.items ?? [])].sort((a, b) => Number(b[metric] ?? 0) - Number(a[metric] ?? 0)).slice(0, 10);
  if (!rows.length) return <p className="flex h-28 items-center justify-center text-sm text-muted-foreground">{insightText("暂无已采集数据")}</p>;
  const max = Math.max(1, ...rows.map(row => Number(row[metric] ?? 0)));
  return <ol className="space-y-2.5">
    {rows.map((row, index) => {
      const name = authors ? String(row.nickname ?? row.name ?? row.email ?? "—") : String(row.repository ?? "—");
      const value = Number(row[metric] ?? 0);
      return <li key={String(row.id ?? index)} className="space-y-1">
        <div className="flex min-w-0 items-center gap-3 text-xs"><span className="min-w-0 flex-1 truncate" title={name}>{name}</span><span className={`shrink-0 font-medium tabular-nums ${metricClass(metric)}`}>{value.toLocaleString()}</span></div>
        <div className="h-1.5 overflow-hidden rounded-full bg-muted" aria-hidden="true"><div className="h-full rounded-full bg-primary/75" style={{ width: `${value / max * 100}%` }} /></div>
      </li>;
    })}
  </ol>;
}

function PeriodCard({ workspace, connectionId, scope, period, day, conditions, refreshKey, metric, onOpen }: {
  workspace: string; connectionId: string; scope: "connection" | "insight"; period: typeof overviewPeriods[number]; day: string;
  conditions: Record<string, string>; refreshKey: number; metric: string; onOpen: (kind: string, dates: Record<string, string>) => void;
}) {
  const ranking = useMemo(() => overviewRanking(workspace, connectionId, period, new Date(`${day}T00:00:00Z`), scope), [workspace, connectionId, period, day, scope]);
  const repositories = useMemo(() => ({ ...analysisReport(workspace, connectionId, "repositories", scope), fixed: ranking.fixed! }), [workspace, connectionId, scope, ranking]);
  const authors = useReport(workspace, ranking, conditions, refreshKey);
  const repos = useReport(workspace, repositories, conditions, refreshKey);
  return <section className="min-w-0 rounded-xl border bg-card p-4" aria-label={insightText("periodRanking", { period: insightText(analysisDatePresets.find(([key]) => key === period)![1]) })}>
    <header className="mb-5 flex flex-wrap items-baseline justify-between gap-2"><h4 className="font-semibold">{insightText(analysisDatePresets.find(([key]) => key === period)![1])}</h4><span className="text-xs tabular-nums text-muted-foreground">{ranking.fixed!.since} – {ranking.fixed!.until}</span></header>
    <div className="grid min-w-0 gap-5 sm:grid-cols-2">
      <div className="min-w-0"><Button size="sm" variant="link" className="mb-3 h-auto p-0" onClick={() => onOpen("repositories", ranking.fixed!)}>{insightText("仓库")}</Button><Bars state={repos} metric={metric} authors={false} /></div>
      <div className="min-w-0"><Button size="sm" variant="link" className="mb-3 h-auto p-0" onClick={() => onOpen("ranking", ranking.fixed!)}>{insightText("贡献排名")}</Button><Bars state={authors} metric={metric} authors /></div>
    </div>
  </section>;
}

export function AnalysisOverview({ workspace, connectionId, scope, day, conditions, refreshKey, onOpen }: {
  workspace: string; connectionId: string; scope: "connection" | "insight"; day: string; conditions: Record<string, string>; refreshKey: number;
  onOpen: (kind: string, dates: Record<string, string>) => void;
}) {
  const metric = "effectives";
  const config = useMemo(() => analysisReport(workspace, connectionId, "overview", scope), [workspace, connectionId, scope]);
  const state = useReport(workspace, config, conditions, refreshKey);
  const totals = state.data?.items[0];
  return <div className="space-y-6">
    {state.error && <p role="alert" className="text-sm text-destructive">{state.error}</p>}
    <dl className="grid grid-cols-2 gap-3 lg:grid-cols-5">{["projects", ...metrics].map((key, index) => <div key={key} className="rounded-xl border bg-card px-4 py-4"><dt className="text-xs text-muted-foreground">{insightText(index ? metricLabels[index - 1]! : "参与仓库")}</dt><dd className={`mt-2 text-2xl font-semibold tabular-nums ${metricClass(key)}`}>{state.loading || state.error ? "—" : Number(totals?.[key] ?? 0).toLocaleString()}</dd></div>)}</dl>
    {state.data?.coverage !== undefined && <AnalysisSummary data={state.data} kind="overview" />}
    <section aria-label={insightText("六个周期贡献排名")} className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-3"><h3 className="font-semibold">{insightText("周期贡献排名（UTC）")}</h3><span className="text-xs text-muted-foreground">{insightText("有效变更")} · Top 10</span></div>
      <div className="grid min-w-0 gap-4 xl:grid-cols-2">{overviewPeriods.map(period => <PeriodCard key={period} {...{ workspace, connectionId, scope, period, day, conditions, refreshKey, metric, onOpen }} />)}</div>
    </section>
  </div>;
}
