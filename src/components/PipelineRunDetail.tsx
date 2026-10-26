import { memo, useEffect, useLayoutEffect, useRef, useState } from "react";
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

  const cancel = async () => {
    if (!window.confirm(CANCEL_NOTICE)) return;
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

  const badge = stateBadge(run?.state ?? "queued");
  return (
    <div className="flex min-h-0 flex-col gap-3 overflow-auto pb-3">
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      {!run ? <p className="text-muted-foreground">载入中…</p> : <>
        <section aria-label="执行概要" className="space-y-3 rounded-lg border border-border bg-card p-4">
          <div className="flex flex-wrap items-center gap-3">
            <h2 className="font-semibold">执行 #{run.number}</h2>
            <Badge variant={badge.variant} className={badge.className}>{badge.label}</Badge>
            {isActive(run.state) && permissions.has("pipelines.run") && (
              <Button variant="outline" size="sm" disabled={busy} onClick={() => void cancel()}>
                {busy ? "提交中…" : "停止等待并标记取消"}
              </Button>
            )}
          </div>
          <dl className="grid gap-3 text-sm sm:grid-cols-3">
            {[
              ["流水线 ID", run.pipeline_id], ["目标主机 ID", run.host_id], ["触发人 ID", run.triggered_by],
              ["耗时", duration(run.started_at, run.finished_at)], ["Commit", run.commit || "—"],
            ].map(([label, value]) => <div key={label} className="min-w-0">
              <dt className="text-muted-foreground">{label}</dt><dd className="break-all">{value}</dd>
            </div>)}
          </dl>
          <a className="inline-flex min-h-8 items-center text-sm text-primary underline" href={href({ workspace, group: "build", item: "artifacts", mode: "list" }, { run_id: run.id })}>查看构建产物</a>
          <p className="text-xs text-muted-foreground">{CANCEL_NOTICE}</p>
        </section>
        <section aria-label="步骤" className="shrink-0">
          <h2 className="mb-2 text-sm font-medium">步骤</h2>
          <ol className="divide-y divide-border rounded-lg border border-border bg-card">
            {run.steps.map((step) => {
              const badge = stateBadge(step.state);
              return <li key={step.index} className="flex flex-wrap items-center gap-3 px-3 py-2 text-sm">
                <span className="text-muted-foreground">{step.index + 1}.</span>
                <span className="min-w-0 flex-1 break-words">{step.name}</span>
                <Badge variant={badge.variant} className={badge.className}>{badge.label}</Badge>
                <span>退出码：{step.exit_code ?? "—"}</span>
                <span>{duration(step.started_at, step.finished_at)}</span>
              </li>;
            })}
          </ol>
        </section>
        <div className="flex items-center justify-between text-sm">
          <h2 className="font-medium">日志 · {logs.length} 个分片</h2>
          <span className="text-muted-foreground">{complete ? "日志已读取完毕，轮询已停止" : "按步骤读取日志"}</span>
        </div>
        <div ref={scroll} role="region" aria-label="执行日志" tabIndex={0}
          className="min-h-48 flex-1 overflow-auto rounded-lg border border-border bg-card p-3 font-mono text-xs leading-5"
          onScroll={() => {
            const el = scroll.current;
            if (el) follow.current = el.scrollHeight - el.clientHeight - el.scrollTop < 32;
          }}>
          {logs.map((log) => <LogChunk key={log.seq} log={log} />)}
          {!logs.length && <p className="text-muted-foreground">暂无日志。</p>}
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
