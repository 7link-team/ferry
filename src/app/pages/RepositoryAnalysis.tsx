import { useEffect, useMemo, useState } from "react";
import { RefreshCw, Search, SlidersHorizontal, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import { analysisConfig, analysisFilters, analysisReport, analysisReports, dayMs } from "../../features/code-insights/analysis-config";
import { AnalysisOverview } from "../../features/code-insights/AnalysisOverview";
import { AutoForm } from "../../shared/forms/auto-form";
import { commonText, configText, insightText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { ResourceForm } from "../resources/ResourceForm";
import { FilterField, readFilters, ResourceTable, writeFilters } from "../resources/ResourceTable";
import { useQueryValue } from "../router";
export * from "../../features/code-insights/analysis-config";

const filterKeys = [...analysisFilters.map(filter => filter.key), "period"];
const primary = new Set(["repos", "authors", "branches", "since", "until", "period"]);

export function RepositoryAnalysis({ workspace, connectionId, permissions, scope = "connection", refreshVersion = 0 }: { workspace: string; connectionId: string; permissions: Set<string>; scope?: "connection" | "insight"; refreshVersion?: number }) {
  useTranslation("insight-domain");
  const [report, setReport] = useQueryValue("analysis_report", "overview");
  const kind = analysisReports.some(([key]) => key === report) ? report : "overview";
  const [conditions, setConditions] = useState(() => readFilters(filterKeys));
  const [draft, setDraft] = useState(conditions);
  const [advanced, setAdvanced] = useState(() => analysisFilters.some(filter => !primary.has(filter.key) && !!conditions[filter.key]));
  const [refreshKey, setRefreshKey] = useState(0);
  useEffect(() => { if (refreshVersion) setRefreshKey(value => value + 1); }, [refreshVersion]);
  const [utcDay, setUtcDay] = useState(() => new Date().toISOString().slice(0, 10));
  useEffect(() => {
    const timer = window.setTimeout(() => setUtcDay(new Date().toISOString().slice(0, 10)), Date.parse(`${utcDay}T00:00:00Z`) + dayMs - Date.now() + 100);
    return () => clearTimeout(timer);
  }, [utcDay]);
  useEffect(() => {
    const sync = () => { const next = readFilters(filterKeys); setConditions(next); setDraft(next); };
    window.addEventListener("popstate", sync);
    return () => window.removeEventListener("popstate", sync);
  }, []);
  const form = useMemo(() => analysisConfig(workspace, connectionId, permissions), [workspace, connectionId, permissions]);
  const reportConfig = useMemo(() => analysisReport(workspace, connectionId, kind, scope), [workspace, connectionId, kind, scope]);
  const list = useMemo(() => {
    const { summary, ...config } = reportConfig;
    return { ...config, queryPrefix: `analysis.${kind}`, filters: [], filterPresets: [], defaultRefresh: 0,
      ...(kind === "heatmap" || kind === "period" ? { summary: summary! } : {}),
    };
  }, [reportConfig, kind]);
  const commit = (next: Record<string, string>) => {
    setConditions(next); setDraft(next); writeFilters(next);
    const params = new URLSearchParams(location.search);
    for (const key of [...params.keys()]) if (key.startsWith("analysis.") && key.endsWith(".page")) params.delete(key);
    const query = params.toString();
    history.replaceState(history.state, "", `${location.pathname}${query ? `?${query}` : ""}`);
    setRefreshKey(value => value + 1);
  };
  return <div className={scope === "insight" ? "flex min-w-0 shrink-0 flex-col gap-5" : "flex min-h-0 flex-1 flex-col gap-5 overflow-auto"}>
    {scope === "connection" && <ResourceForm config={form} workspace={workspace} permissions={permissions} showViewNavigation={false} />}
    <section className="flex min-w-0 flex-col gap-5" aria-label={insightText("贡献分析报告")}>
      <div className="flex flex-wrap gap-1 border-b pb-2" aria-label={insightText("报告维度")}>{analysisReports.map(([key, label]) => <Button key={key} size="sm" variant={kind === key ? "secondary" : "ghost"} aria-pressed={kind === key} onClick={() => setReport(key)}>{insightText(label)}</Button>)}</div>
      <div className="space-y-3 rounded-xl border bg-card p-4">
        {reportConfig.filterPresets?.map(group => <div key={group.label} role="group" aria-label={configText(group.label, group.labelKey)} className="flex flex-wrap gap-1">{group.presets.map(preset => <Button key={preset.label} size="xs" variant="ghost" onClick={() => commit({ ...conditions, ...(typeof preset.values === "function" ? preset.values() : preset.values) })}>{configText(preset.label, preset.labelKey)}</Button>)}</div>)}
        <AutoForm aria-label={insightText("统一分析筛选")} className="flex flex-wrap items-end gap-3" onSubmit={event => { event.preventDefault(); commit(draft); }}>
          {reportConfig.filters?.filter(filter => primary.has(filter.key)).map(filter => <FilterField key={filter.key} filter={filter} value={draft[filter.key] ?? ""} onChange={value => setDraft(previous => ({ ...previous, [filter.key]: value }))} />)}
          <div className="flex flex-wrap items-center gap-2">
            <Button type="submit" size="sm"><Search />{commonText("common.search")}</Button>
            <Button type="button" size="sm" variant="ghost" onClick={() => commit(Object.fromEntries(filterKeys.map(key => [key, ""])))}><X />{commonText("common.clear")}</Button>
            <Button type="button" size="sm" variant="ghost" aria-expanded={advanced} onClick={() => setAdvanced(value => !value)}><SlidersHorizontal />{insightText("高级筛选")}</Button>
            <Button type="button" size="sm" variant="outline" onClick={() => setRefreshKey(value => value + 1)}><RefreshCw />{commonText("common.refresh")}</Button>
          </div>
          {advanced && <div className="flex w-full flex-wrap items-end gap-3 border-t pt-3">{reportConfig.filters?.filter(filter => !primary.has(filter.key)).map(filter => <FilterField key={filter.key} filter={filter} value={draft[filter.key] ?? ""} onChange={value => setDraft(previous => ({ ...previous, [filter.key]: value }))} />)}</div>}
        </AutoForm>
      </div>
      {kind === "overview" ? <AnalysisOverview workspace={workspace} connectionId={connectionId} scope={scope} day={utcDay} conditions={conditions} refreshKey={refreshKey} onOpen={(next, dates) => { commit({ ...conditions, since: dates.since!, until: dates.until! }); setReport(next); }} /> : <div className="flex min-h-[28rem] flex-col"><ResourceTable key={`${connectionId}/${kind}/${refreshKey}`} config={list} workspace={workspace} permissions={permissions} conditions={conditions} refreshKey={refreshKey} hideToolbar /></div>}
    </section>
  </div>;
}
