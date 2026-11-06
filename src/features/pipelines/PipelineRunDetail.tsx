import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { Badge } from "../../shared/ui/badge";
import { Tabs } from "radix-ui";
import { ChevronDown,ChevronRight } from "lucide-react";
import { memo,useEffect,useLayoutEffect,useRef,useState } from "react";
import { api,errorText } from "../../shared/api/api";
import { base,named } from "../../shared/api/resource";
import { executionText,useCommonTranslation } from "../../shared/i18n/i18n";
import type { Navigation } from "../../shared/routing/types";
import { Button } from "../../shared/ui/button";
import { duration,isActive,mergeLogs,stateBadge,type LogPage,type PipelineRun,type RunLog } from "./pipeline";

function PipelineState({ value }: { value: string }) {
  const { label, ...props } = stateBadge(value);
  return <Badge {...props}>{label}</Badge>;
}

const CANCEL_NOTICE = "run.stopNotice";

export function PipelineRunDetail({ navigation, workspace, runId, permissions, tab = "logs", onTabChange }: { navigation: Navigation;
  workspace: string; runId: string; permissions: Set<string>; tab?: string; onTabChange?: (tab: string) => void;
}) {
  useCommonTranslation();
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
  const cancelNotice = actions ? executionText("run.cancelNotice") : executionText(CANCEL_NOTICE);
  const cancel = async () => {
    if (!window.confirm(cancelNotice)) return;
    const controller = new AbortController();
    cancelRequest.current = controller;
    setBusy(true);
    try {
      await api(`${endpoint}/cancellations`, { method: "POST", body: "{}", signal: controller.signal });
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
      await api(`${endpoint}/jobs/${encodeURIComponent(key)}/approvals`, { method: "POST", body: "{}" });
      reload.current();
    } catch (cause) {
      setError(errorText(cause));
    } finally {
      setBusy(false);
    }
  };

  const currentJob = run?.jobs?.find(job => job.key === selectedJob) ?? run?.jobs?.find(job => job.state === "failed" || job.state === "running" || job.state === "waiting") ?? run?.jobs?.[0];
  const visibleSteps = currentJob?.step_indices ? run?.steps.filter(step => currentJob.step_indices!.includes(step.index)) ?? [] : run?.steps ?? [];
  const otherLogs = logs.filter(log => log.step_index < 0);
  return (
    <div className="flex min-h-0 min-w-0 flex-col gap-3 overflow-hidden pb-3">
      {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
      {!run ? !error && <LoadingSkeleton variant="page" label={executionText("run.loading")} /> : <>
        <section aria-label={executionText("run.summary")} className="max-h-[35dvh] shrink-0 space-y-3 overflow-auto overscroll-contain rounded-lg border border-border bg-card p-4">
          <div className="flex flex-wrap items-center gap-3">
            <h2 className="font-semibold">{executionText("run.number", { number: run.number })}</h2>
            <PipelineState value={run.state} />
            {isActive(run.state) && permissions.has("pipelines.run") && (
              <Button variant="outline" size="sm" disabled={busy || run.state === "cancelling"} onClick={() => void cancel()}>
                {busy ? executionText("run.submitting") : run.state === "cancelling" ? executionText("run.cancelling") : actions ? executionText("run.cancel") : executionText("run.stopWaiting")}
              </Button>
            )}
          </div>
          <dl className="grid grid-cols-2 gap-3 text-sm sm:grid-cols-3">
            <div className="min-w-0"><dt className="text-muted-foreground">{executionText("shared.common.nav.pipelines")}</dt><dd className="break-all"><a className="text-primary underline" href={navigation.href({ workspace, group: "build", item: "pipelines", id: run.pipeline_id, mode: "view" }, {})}>{named(run.pipeline_name, run.pipeline_id)}</a></dd></div>
            {[
              [executionText("shared.resource.pipeline.environment"), run.environment_name || executionText("run.unspecified")], ...(run.host_id !== "0" ? [[executionText("shared.resource.deployment.targetHost"), named(run.host_name, run.host_id)]] : []), [executionText("shared.resource.execution.triggeredBy"), named(run.triggered_by_name, run.triggered_by)], ...(run.inputs && Object.keys(run.inputs).length ? [[executionText("run.inputs"), Object.entries(run.inputs).map(([key, value]) => `${key}=${String(value)}`).join("，")]] : []),
              [executionText("shared.resource.execution.duration"), duration(run.started_at, run.finished_at)], [executionText("run.commit"), run.commit || "—"],
            ].map(([label, value]) => <div key={label} className="min-w-0">
              <dt className="text-muted-foreground">{label}</dt><dd className="break-all">{value}</dd>
            </div>)}
          </dl>
          <a className="inline-flex min-h-8 items-center text-sm text-primary underline" href={navigation.href({ workspace, group: "build", item: "artifacts", mode: "list" }, { run_id: run.id })}>{executionText("run.artifacts")}</a>
          {isActive(run.state) && <p className="text-xs text-muted-foreground">{cancelNotice}</p>}
        </section>
        <Tabs.Root value={tab === "snapshot" ? "snapshot" : "logs"} onValueChange={onTabChange} className="flex min-h-0 flex-1 flex-col gap-3">
          <Tabs.List aria-label={executionText("run.views")} className="flex shrink-0 gap-4 border-b">
            {(["logs", "snapshot"] as const).map(value => <Tabs.Trigger key={value} value={value} className="border-b-2 border-transparent px-1 py-2 text-sm text-muted-foreground data-[state=active]:border-primary data-[state=active]:text-primary">{executionText(value === "logs" ? "run.stepsAndLogs" : "run.configurationSnapshot")}</Tabs.Trigger>)}
          </Tabs.List>
          <Tabs.Content value="snapshot" className="min-h-0 flex-1 space-y-4 overflow-auto overscroll-contain data-[state=inactive]:hidden">
            <p className="text-sm text-muted-foreground">{executionText("run.configurationSnapshotHelp")}</p>
            {run.definition_snapshot ? <>
              <pre aria-label={executionText("run.workflowSnapshot")} tabIndex={0} className="overflow-auto rounded border bg-card p-4 font-mono text-xs leading-5">{run.definition_snapshot.workflow}</pre>
              <pre aria-label={executionText("run.executionBindings")} tabIndex={0} className="overflow-auto rounded border bg-card p-4 font-mono text-xs leading-5">{JSON.stringify({ executor_bindings: run.definition_snapshot.executor_bindings, checkout_bindings: run.definition_snapshot.checkout_bindings }, null, 2)}</pre>
            </> : <p className="text-sm text-muted-foreground">{executionText("run.snapshotUnavailable")}</p>}
        {!!run.variables?.length && <section aria-label={executionText("run.variables")} className="rounded border p-3 text-sm">
          <h2 className="font-medium">{executionText("run.variables")}</h2><p className="text-muted-foreground">{executionText("run.snapshotHelp")}</p>
          <ul>{run.variables.map(v => <li key={v.environment}>{v.environment} ← {v.name} · {v.scope === "environment" ? executionText("shared.resource.pipeline.environment") : v.scope_ref !== "0" ? executionText("run.pipelineScope") : v.scope === "global" ? executionText("shared.resource.variable.global") : executionText("run.buildScope")} · v{v.version} · {v.secret ? executionText("shared.resource.variable.secret") : executionText("run.regularVariable")}</li>)}</ul>
        </section>}
          </Tabs.Content>
          <Tabs.Content value="logs" className="grid min-h-0 flex-1 grid-rows-[auto_minmax(0,1fr)] overflow-hidden border-y data-[state=inactive]:hidden lg:grid-cols-[15rem_minmax(0,1fr)] lg:grid-rows-1">
          <aside aria-label={executionText("run.jobList")} className="max-h-32 min-h-0 overflow-y-auto overscroll-contain border-b bg-muted/30 p-3 lg:max-h-none lg:border-b-0 lg:border-r" tabIndex={0}>
            <h2 className="mb-2 px-2 text-sm font-semibold">{executionText("shared.resource.cron.task")}</h2>
            <div className="space-y-1">{run.jobs?.map(job => {
              return <button key={job.key} type="button" className={`flex w-full items-center justify-between gap-2 rounded px-2 py-2 text-left text-sm hover:bg-accent ${currentJob?.key === job.key ? "bg-accent font-medium" : ""}`} aria-current={currentJob?.key === job.key ? "true" : undefined} onClick={() => setSelectedJob(job.key)}><span className="min-w-0 truncate">{job.name || job.key}</span><PipelineState value={job.state} /></button>;
            })}</div>
          </aside>
          <section aria-label={executionText("run.stepsAndLogs")} className="min-h-0 min-w-0 overflow-y-auto overscroll-contain p-3 sm:p-4" tabIndex={0}>
            <div className="mb-3 flex flex-wrap items-center gap-2"><h2 className="font-semibold">{currentJob?.name || currentJob?.key || executionText("run.steps")}</h2>{currentJob && <span className="text-xs text-muted-foreground">{duration(currentJob.started_at, currentJob.finished_at)}</span>}{complete && <span className="text-xs text-muted-foreground">{executionText("run.logsLoaded")}</span>}{currentJob && run.environments?.[currentJob.key] && <span className="text-xs text-muted-foreground">{executionText("run.environmentPrefix")}{/^https?:\/\//.test(run.environments[currentJob.key]!.url) ? <a className="underline" href={run.environments[currentJob.key]!.url} target="_blank" rel="noreferrer">{run.environments[currentJob.key]!.name}</a> : run.environments[currentJob.key]!.name}{run.environments[currentJob.key]!.approval ? executionText("run.approvalRequired") : ""}</span>}{currentJob?.state === "waiting" && permissions.has("pipelines.run") && <Button type="button" size="sm" disabled={busy} onClick={() => void approve(currentJob.key)}>{run.environments?.[currentJob.key]?.approval ? executionText("run.approveDeployment") : executionText("run.confirmContinue")}</Button>}</div>
            <div className="divide-y border-y">{visibleSteps.map(step => {
              const stepName = currentJob && step.name.startsWith(`${currentJob.name || currentJob.key} / `) ? step.name.slice((currentJob.name || currentJob.key).length + 3) : step.name;
              const chunks = logs.filter(log => log.step_index === step.index);
              const expanded = expandedSteps[step.index] ?? (step.state === "failed" || step.state === "running" || chunks.length > 0);
              return <div key={step.index}><button type="button" className="flex w-full items-center gap-2 px-2 py-3 text-left text-sm hover:bg-accent" aria-expanded={expanded} onClick={() => setExpandedSteps(current => ({ ...current, [step.index]: !expanded }))}>{expanded ? <ChevronDown className="size-4 shrink-0" /> : <ChevronRight className="size-4 shrink-0" />}<span className="min-w-0 flex-1 truncate">{stepName}</span><PipelineState value={step.state} /><span className="text-xs text-muted-foreground">{duration(step.started_at, step.finished_at)}</span></button>{expanded && <div ref={step.index === visibleSteps[0]?.index ? scroll : undefined} role="region" aria-label={step.index === visibleSteps[0]?.index ? executionText("run.executionLogs") : executionText("run.stepLogs", { name: stepName })} className="max-h-[32rem] min-h-16 overflow-auto overscroll-contain border-t bg-background px-3 py-2 font-mono text-xs leading-5" onScroll={() => { const el = scroll.current; if (el) follow.current = el.scrollHeight - el.clientHeight - el.scrollTop < 32; }}>{chunks.length ? chunks.map(log => <LogChunk key={log.seq} log={log} />) : <span className="text-muted-foreground">{executionText("run.noLogs")}</span>}{step.exit_code !== null && <p className="mt-2 text-muted-foreground">{executionText("run.exitCodeValue", { code: step.exit_code })}</p>}</div>}</div>;
            })}</div>
            {otherLogs.length > 0 && <div className="mt-4"><h3 className="mb-2 text-sm font-medium">{executionText("run.runtimeLogs")}</h3><div ref={!visibleSteps.length ? scroll : undefined} role="region" aria-label={executionText("run.runtimeLogs")} tabIndex={0} className="max-h-64 min-h-20 overflow-auto overscroll-contain border bg-background p-3 font-mono text-xs leading-5" onScroll={() => { const el = scroll.current; if (el) follow.current = el.scrollHeight - el.clientHeight - el.scrollTop < 32; }}>{otherLogs.map(log => <LogChunk key={log.seq} log={log} />)}</div></div>}
            {!visibleSteps.length && otherLogs.length === 0 && <p className="py-6 text-sm text-muted-foreground">{currentJob?.state === "waiting" ? (run.environments?.[currentJob.key]?.approval ? executionText("run.waitDeployment", { name: run.environments[currentJob.key]!.name }) : executionText("run.waitApproval")) : complete ? executionText("run.noStepLogs") : executionText("run.waitJob")}</p>}
          </section>
          </Tabs.Content>
        </Tabs.Root>
      </>}
    </div>
  );
}

// Existing chunks retain object identity and skip rendering on append.
const LogChunk = memo(function LogChunk({ log }: { log: RunLog }) {
  useCommonTranslation();
  return <pre data-seq={log.seq} className={`whitespace-pre-wrap break-all ${log.stream === "stderr" ? "text-destructive" : ""}`}>
    <span className="text-muted-foreground">{executionText("run.logMeta", { seq: log.seq, step: log.step_index + 1, stream: log.stream })}</span>{log.chunk}
  </pre>;
});
