import { useEffect, useState } from "react";
import { AlertCircle, CheckCircle2, Clock3, Loader2, RefreshCw } from "lucide-react";
import { api, errorText } from "../../shared/api/api";
import { base, createResource } from "../../shared/api/resource";
import { insightText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { analysisState } from "./analysis-config";
import { time, type Row } from "./config";

export function sourceTaskState(source: Row) {
  const job = source.job as Row | undefined;
  if (job?.effective_state) return String(job.effective_state);
  return String(job?.state ?? source.state ?? "pending");
}

export function taskStateLabel(state: string) {
  if (state === "blocked") return insightText("采集暂不可用");
  if (state === "retry_wait") return insightText("等待重试");
  return analysisState(state);
}

function collectorPhaseLabel(phase: unknown) {
  if (phase === "api") return insightText("平台接口采集");
  if (phase === "git_fetch") return insightText("获取仓库变更");
  if (phase === "git_analyze") return insightText("分析提交信息");
  return "";
}

/** Status polling reads collection state only; it never schedules another scan. */
export function CodeInsightStatus({ workspace, insightId, permissions, onConfigure, onPublished }: {
  workspace: string; insightId: string; permissions: Set<string>; onConfigure: () => void; onPublished: () => void;
}) {
  const [row, setRow] = useState<Row>();
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  const [checkedAt, setCheckedAt] = useState(0);
  useEffect(() => {
    let live = true;
    let timer = 0;
    const controller = new AbortController();
    let previousSuccess = "";
    const read = async () => {
      try {
        const next = await api<Row>(`${base(workspace, "code-insights")}/${encodeURIComponent(insightId)}`, { signal: controller.signal });
        if (!live) return;
        const success = JSON.stringify((next.sources as Row[] | undefined ?? []).map(source => [source.repository, source.synced_at]));
        if (previousSuccess && previousSuccess !== success) onPublished();
        previousSuccess = success;
        setRow(next); setCheckedAt(Date.now()); setError("");
      } catch (cause) { if (live && !controller.signal.aborted) setError(errorText(cause)); }
      if (live) timer = window.setTimeout(() => { if (document.hidden) timer = window.setTimeout(read, 15000); else void read(); }, 15000);
    };
    void read();
    return () => { live = false; controller.abort(); clearTimeout(timer); };
  }, [workspace, insightId, revision, onPublished]);
  const sources = (row?.sources ?? []) as Row[];
  const active = sources.filter(source => ["running", "queued", "retry_wait"].includes(sourceTaskState(source)));
  const running = active.some(source => sourceTaskState(source) === "running");
  const queued = active.some(source => sourceTaskState(source) === "queued");
  const blocked = sources.filter(source => sourceTaskState(source) === "blocked");
  const failed = sources.filter(source => ["failed", "partial", "unsupported", "unavailable"].includes(sourceTaskState(source)));
  const successes = sources.map(source => Number(source.synced_at ?? 0)).filter(value => value > 0);
  const status = !row ? "正在加载分析状态" : row.enabled === false ? "分析已暂停" : blocked.length ? "分析尚未完成" : running ? "同步中" : queued ? "已排队" : active.length ? "等待重试" : failed.length || row.last_error ? "分析尚未完成" : sources.length && sources.every(source => sourceTaskState(source) === "complete") ? "同步完成" : "等待初始化";
  const Icon = running ? Loader2 : blocked.length || failed.length || row?.last_error ? AlertCircle : status === "同步完成" ? CheckCircle2 : Clock3;
  const retry = async () => {
    setBusy(true); setError("");
    try { await createResource(workspace, "code-insights", {}, `${base(workspace, "code-insights")}/${encodeURIComponent(insightId)}/sync`); setRevision(value => value + 1); }
    catch (cause) { setError(errorText(cause)); }
    finally { setBusy(false); }
  };
  return <section aria-label={insightText("分析任务状态")} className="space-y-4 rounded-xl border bg-card p-4">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div className="min-w-0"><h2 className="truncate text-lg font-semibold">{String(row?.name ?? insightText("代码洞察"))}</h2><p role="status" className="mt-1 flex items-center gap-2 text-sm"><Icon className={`size-4 shrink-0 ${running ? "animate-spin" : ""}`} />{insightText(status)}{row && <span className="text-muted-foreground">· {sources.length} {insightText("仓库")}</span>}</p></div>
      <div className="flex flex-wrap gap-2">{permissions.has("code-insights.sync") && <Button size="sm" variant="outline" disabled={busy} onClick={() => void retry()}><RefreshCw className={busy ? "animate-spin" : ""} />{insightText("重新核对 / 重试同步")}</Button>}<Button size="sm" variant="ghost" onClick={onConfigure}>{insightText("配置与同步")}</Button></div>
    </div>
    {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
    {row?.last_error ? <p role="alert" className="text-sm text-destructive">{String(row.last_error)}</p> : null}
    <dl className="grid gap-3 text-sm sm:grid-cols-3">
      <div><dt className="text-xs text-muted-foreground">{insightText("最近成功分析")}</dt><dd className="mt-1 tabular-nums">{successes.length ? time(Math.max(...successes)) : insightText("尚未成功")}</dd></div>
      <div><dt className="text-xs text-muted-foreground">{insightText("下次计划更新")}</dt><dd className="mt-1 tabular-nums">{!row ? "—" : row.enabled === false ? insightText("分析已暂停") : row.update_mode === "event" ? insightText("仓库事件触发") : time(row.next_run_at)}</dd></div>
      <div><dt className="text-xs text-muted-foreground">{insightText("状态检查时间")}</dt><dd className="mt-1 tabular-nums">{time(checkedAt)}</dd></div>
    </dl>
    {(active.length > 0 || blocked.length > 0) && <p className="text-xs text-muted-foreground">{insightText(blocked.length ? "采集暂未完成，请查看仓库错误详情并重试同步。" : "任务状态每 15 秒更新；下次计划时间表示触发采集，不是完成时间。当前无法准确预估完成时间。")}</p>}
    {sources.length > 0 && <details open={active.length > 0 || blocked.length > 0 || failed.length > 0} className="border-t pt-3"><summary className="cursor-pointer text-sm font-medium">{insightText("各仓库覆盖与任务状态")}</summary><ul className="mt-3 divide-y">{sources.map((source, index) => {
      const job = source.job as Row | undefined;
      const state = sourceTaskState(source);
      const phase = state === "running" ? collectorPhaseLabel(job?.collector_phase) : "";
      const lastError = String(job?.last_error || source.last_error || "");
      return <li key={String(source.repository ?? index)} className="space-y-1 py-3 first:pt-0 last:pb-0">
        <div className="flex flex-wrap items-center justify-between gap-2"><span className="break-all text-sm font-medium">{String(source.repository ?? "—")}</span><span className={`text-xs ${state === "blocked" || state === "failed" ? "text-destructive" : "text-muted-foreground"}`}>{taskStateLabel(state)}</span></div>
        <p className="flex flex-wrap gap-x-5 gap-y-1 text-xs text-muted-foreground"><span>{insightText("最近成功分析")}：{Number(source.synced_at) > 0 ? time(source.synced_at) : insightText("尚未成功")}</span>{job?.updated_at ? <span>{insightText("任务状态更新")}：{time(job.updated_at)}</span> : null}{state === "retry_wait" && job?.retry_at ? <span>{insightText("下次重试")}：{time(job.retry_at)}</span> : null}</p>
        {phase && <p className="text-xs text-muted-foreground">{phase}</p>}
        {lastError && <p className="break-words text-xs text-destructive">{lastError}</p>}
      </li>;
    })}</ul></details>}
  </section>;
}
