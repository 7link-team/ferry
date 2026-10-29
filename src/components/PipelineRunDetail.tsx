import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
import { ChevronDown, ChevronRight } from "lucide-react";
import { Badge } from "./ui/badge";
import { Button } from "./ui/button";
import { api, errorText } from "../api";
import { base } from "../resource";
import { href } from "../router";
import { duration, isActive, mergeLogs, stateBadge, type LogPage, type PipelineRun, type RunLog } from "../pipeline";

const CANCEL_NOTICE = "停止等待并标记为已取消；目标主机上的命令可能继续执行完毕。";

export function PipelineRunDetail({ workspace, runId, permissions }: {
  workspace: string; runId: string; permissions: Set<string>;
}) {
  const [run, setRun] = useState<PipelineRun>();
  const [logs, setLogs] = useState<RunLog[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [complete, setComplete] = useState(false);
  const [selectedJob, setSelectedJob] = useState("");
  const [expandedSteps, setExpandedSteps] = useState<Record<number, boolean>>({});
  const scroll = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const cancelRequest = useRef<AbortController | null>(null);
  const reload = useRef<() => void>(() => {});
  const endpoint = `${base(workspace, "pipeline-runs")}/${encodeURIComponent(runId)}`;

  useEffect(() => {
    let live = true;
    let finished = false;
    let after = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let request: AbortController | undefined;
    setRun(undefined);
    setLogs([]);
    setComplete(false);
    follow.current = true;
    const load = async () => {
      if (!live || document.hidden || finished || request) return;
      clearTimeout(timer);
      const controller = new AbortController();
      request = controller;
      try {
        // Read state first: once terminal, all persisted log pages can be
        // drained before stopping, even if there are more than 200 chunks.
        const next = await api<PipelineRun>(endpoint, { signal: controller.signal });
        if (!live || controller.signal.aborted) return;
        setRun(next);
        let page: LogPage;
        do {
          page = await api<LogPage>(`${endpoint}/logs?after=${after}`, { signal: controller.signal });
          if (!live || controller.signal.aborted) return;
          const incoming = page.items;
          setLogs((current) => mergeLogs(current, incoming));
          after = page.next_seq;
        } while (page.items.length === 200 && !document.hidden);
        finished = !isActive(next.state) && page.items.length < 200;
        setComplete(finished);
        setError("");
      } catch (cause) {
        if (live && !controller.signal.aborted) setError(errorText(cause));
      } finally {
        if (request === controller) request = undefined;
        if (live && !controller.signal.aborted && !document.hidden && !finished) timer = setTimeout(() => void load(), 1000);
      }
    };
    const wake = () => {
      clearTimeout(timer);
      if (document.hidden) {
        request?.abort();
        request = undefined;
      } else {
        void load();
      }
    };
    reload.current = () => { finished = false; void load(); };
    document.addEventListener("visibilitychange", wake);
    void load();
    return () => {
      live = false;
      clearTimeout(timer);
      request?.abort();
      cancelRequest.current?.abort();
      document.removeEventListener("visibilitychange", wake);
    };
  }, [endpoint]);

  useLayoutEffect(() => {
    if (follow.current && scroll.current) scroll.current.scrollTop = scroll.current.scrollHeight;
  }, [logs]);

  const actions = run?.definition_format === "github-actions";
  const cancelNotice = actions ? "取消执行并等待远端任务及临时资源清理完成。" : CANCEL_NOTICE;
  const cancel = async () => {
    if (!window.confirm(cancelNotice)) return;
    const controller = new AbortController();
    cancelRequest.current = controller;
    setBusy(true);
    try {
      await api(`${endpoint}/actions/cancel`, { method: "POST", body: "{}", signal: controller.signal });
      if (!controller.signal.aborted) reload.current();
    } catch (cause) {
      if (!controller.signal.aborted) setError(errorText(cause));
    } finally {
      if (!controller.signal.aborted) setBusy(false);
    }
  };
  const approve = async (key: string) => {
    setBusy(true);
    try {
      await api(`${endpoint}/jobs/${encodeURIComponent(key)}/approve`, { method: "POST", body: "{}" });
      reload.current();
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(false);
    }
  };

  const badge = stateBadge(run?.state ?? "queued");
  const currentJob = run?.jobs?.find(job => job.key === selectedJob) ?? run?.jobs?.find(job => job.state === "failed" || job.state === "running" || job.state === "waiting") ?? run?.jobs?.[0];
  const visibleSteps = currentJob?.step_indices ? run?.steps.filter(step => currentJob.step_indices!.includes(step.index)) ?? [] : run?.steps ?? [];
  const otherLogs = logs.filter(log => log.step_index < 0);
  return (
    <div className="flex min-h-0 flex-col gap-3 overflow-auto pb-3">
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      {!run ? <p className="text-muted-foreground">载入中…</p> : <>
        <section aria-label="执行概要" className="space-y-3 rounded-lg border border-border bg-card p-4">
          <div className="flex flex-wrap items-center gap-3">
            <h2 className="font-semibold">执行 #{run.number}</h2>
            <Badge variant={badge.variant} className={badge.className}>{badge.label}</Badge>
            {isActive(run.state) && permissions.has("pipelines.run") && (
              <Button variant="outline" size="sm" disabled={busy || run.state === "cancelling"} onClick={() => void cancel()}>
                {busy ? "提交中…" : run.state === "cancelling" ? "正在取消…" : actions ? "取消执行" : "停止等待并标记取消"}
              </Button>
            )}
          </div>
          <dl className="grid gap-3 text-sm sm:grid-cols-3">
            {[
              ["流水线 ID", run.pipeline_id], ["执行环境", run.environment_name || "未指定"], ...(run.host_id !== "0" ? [["目标主机 ID", run.host_id]] : []), ["触发人 ID", run.triggered_by],
              ["耗时", duration(run.started_at, run.finished_at)], ["Commit", run.commit || "—"],
            ].map(([label, value]) => <div key={label} className="min-w-0">
              <dt className="text-muted-foreground">{label}</dt><dd className="break-all">{value}</dd>
            </div>)}
          </dl>
          <a className="inline-flex min-h-8 items-center text-sm text-primary underline" href={href({ workspace, group: "build", item: "artifacts", mode: "list" }, { run_id: run.id })}>查看构建产物</a>
          {isActive(run.state) && <p className="text-xs text-muted-foreground">{cancelNotice}</p>}
        </section>
        {!!run.variables?.length && <section aria-label="执行变量" className="rounded border p-3 text-sm">
          <h2 className="font-medium">执行变量</h2><p className="text-muted-foreground">使用触发时的加密快照，修改当前变量不会改变本次执行。</p>
          <ul>{run.variables.map(v => <li key={v.environment}>{v.environment} ← {v.name} · {v.scope === "environment" ? "执行环境" : v.scope_ref !== "0" ? "流水线专属" : v.scope === "global" ? "全局" : "构建模块"} · v{v.version} · {v.secret ? "密钥" : "普通变量"}</li>)}</ul>
        </section>}
        <div className="grid min-h-0 flex-1 border-y lg:grid-cols-[15rem_minmax(0,1fr)]">
          <aside aria-label="任务列表" className="border-b bg-muted/30 p-3 lg:border-b-0 lg:border-r">
            <h2 className="mb-2 px-2 text-sm font-semibold">任务</h2>
            <div className="space-y-1">{run.jobs?.map(job => {
              const status = stateBadge(job.state);
              return <button key={job.key} type="button" className={`flex w-full items-center justify-between gap-2 rounded px-2 py-2 text-left text-sm hover:bg-accent ${currentJob?.key === job.key ? "bg-accent font-medium" : ""}`} aria-current={currentJob?.key === job.key ? "true" : undefined} onClick={() => setSelectedJob(job.key)}><span className="min-w-0 truncate">{job.name || job.key}</span><Badge variant={status.variant} className={status.className}>{status.label}</Badge></button>;
            })}</div>
          </aside>
          <section aria-label="任务步骤和日志" className="min-w-0 p-3 sm:p-4">
            <div className="mb-3 flex flex-wrap items-center gap-2"><h2 className="font-semibold">{currentJob?.name || currentJob?.key || "执行步骤"}</h2>{currentJob && <span className="text-xs text-muted-foreground">{duration(currentJob.started_at, currentJob.finished_at)}</span>}{complete && <span className="text-xs text-muted-foreground">日志已加载</span>}{currentJob?.state === "waiting" && permissions.has("pipelines.run") && <Button type="button" size="sm" disabled={busy} onClick={() => void approve(currentJob.key)}>确认继续</Button>}</div>
            <div className="divide-y border-y">{visibleSteps.map(step => {
              const status = stateBadge(step.state);
              const stepName = currentJob && step.name.startsWith(`${currentJob.name || currentJob.key} / `) ? step.name.slice((currentJob.name || currentJob.key).length + 3) : step.name;
              const chunks = logs.filter(log => log.step_index === step.index);
              const expanded = expandedSteps[step.index] ?? (step.state === "failed" || step.state === "running" || chunks.length > 0);
              return <div key={step.index}><button type="button" className="flex w-full items-center gap-2 px-2 py-3 text-left text-sm hover:bg-accent" aria-expanded={expanded} onClick={() => setExpandedSteps(current => ({ ...current, [step.index]: !expanded }))}>{expanded ? <ChevronDown className="size-4 shrink-0" /> : <ChevronRight className="size-4 shrink-0" />}<span className="min-w-0 flex-1 truncate">{stepName}</span><Badge variant={status.variant} className={status.className}>{status.label}</Badge><span className="text-xs text-muted-foreground">{duration(step.started_at, step.finished_at)}</span></button>{expanded && <div ref={step.index === visibleSteps[0]?.index ? scroll : undefined} role="region" aria-label={step.index === visibleSteps[0]?.index ? "执行日志" : `${stepName} 日志`} className="max-h-[32rem] min-h-16 overflow-auto border-t bg-background px-3 py-2 font-mono text-xs leading-5" onScroll={() => { const el = scroll.current; if (el) follow.current = el.scrollHeight - el.clientHeight - el.scrollTop < 32; }}>{chunks.length ? chunks.map(log => <LogChunk key={log.seq} log={log} />) : <span className="text-muted-foreground">暂无日志</span>}{step.exit_code !== null && <p className="mt-2 text-muted-foreground">退出码 {step.exit_code}</p>}</div>}</div>;
            })}</div>
            {otherLogs.length > 0 && <div className="mt-4"><h3 className="mb-2 text-sm font-medium">运行日志</h3><div ref={!visibleSteps.length ? scroll : undefined} role="region" aria-label="运行日志" tabIndex={0} className="max-h-64 min-h-20 overflow-auto border bg-background p-3 font-mono text-xs leading-5" onScroll={() => { const el = scroll.current; if (el) follow.current = el.scrollHeight - el.clientHeight - el.scrollTop < 32; }}>{otherLogs.map(log => <LogChunk key={log.seq} log={log} />)}</div></div>}
            {!visibleSteps.length && otherLogs.length === 0 && <p className="py-6 text-sm text-muted-foreground">{currentJob?.state === "waiting" ? "等待人工确认" : complete ? "暂无步骤日志" : "等待任务开始"}</p>}
          </section>
        </div>
      </>}
    </div>
  );
}

// Existing chunks retain object identity and skip rendering on append.
const LogChunk = memo(function LogChunk({ log }: { log: RunLog }) {
  return <pre data-seq={log.seq} className={`whitespace-pre-wrap break-all ${log.stream === "stderr" ? "text-destructive" : ""}`}>
    <span className="text-muted-foreground">[{log.seq} · 步骤 {log.step_index + 1} · {log.stream}] </span>{log.chunk}
  </pre>;
});
