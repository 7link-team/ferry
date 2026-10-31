import { AutoField } from "./auto-form";
import { lazy, Suspense, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ReactFlow, Background, Controls, Handle, Position, type NodeProps, type Node, type Connection, useReactFlow, useNodesInitialized } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { ArrowRight, GitFork, GripVertical, Plus, Trash2, Unlink2, X } from "lucide-react";
import { cn } from "cn";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import { DeleteConfirmDialog } from "./DeleteConfirmDialog";
import type { Option } from "./dsl";
import { addActionJob, addActionStep, connectActionJobs, editActionJob, editActionStep, parseActions, removeActionJob, removeActionStep, type ActionJob } from "../actions-workflow";

type Binding = { job_id: string; target_kind: string; target_ref: string; image: string };
type Selection = { job: string; step?: number };
type JobData = { job: ActionJob; selected?: Selection; readOnly: boolean; canDelete: boolean; missingHost: boolean; select: (value: Selection) => void; addJob: (job: string, mode: "serial" | "parallel") => void; addStep: (job: string) => void; removeJob: (job: string) => void };
type JobNode = Node<JobData, "job">;
function JobCard({ data }: NodeProps<JobNode>) {
  return <div className={`w-72 rounded-lg border bg-card shadow-sm ${data.missingHost ? "border-destructive ring-1 ring-destructive" : data.selected?.job === data.job.id ? "border-primary ring-1 ring-primary" : "border-border"}`}>
    <Handle type="target" position={Position.Left} />
    <div className="flex items-center gap-2 border-b p-3"><span className="job-drag flex size-5 shrink-0 cursor-grab items-center justify-center text-muted-foreground active:cursor-grabbing" title="拖动任务"><GripVertical className="size-4" /></span><button type="button" className="nodrag min-w-0 flex-1 text-left font-medium focus-visible:ring-2 focus-visible:ring-ring" onClick={() => data.select({ job: data.job.id })}>{data.job.name}</button>
      {!data.readOnly && <Button type="button" variant="ghost" size="icon" className="nodrag nopan size-7" aria-label={`删除任务 ${data.job.name}`} title="删除任务" disabled={!data.canDelete} onClick={event => { event.stopPropagation(); data.removeJob(data.job.id); }}><Trash2 className="size-4" /></Button>}
    </div>
    <div className="space-y-1 p-2">{data.missingHost && <button type="button" className="nodrag w-full rounded px-2 py-1 text-left text-xs text-destructive" onClick={() => data.select({ job: data.job.id })}>请选择执行主机</button>}{data.job.steps.map((step, index) => <button key={index} type="button" className={`nodrag flex w-full items-center gap-2 rounded px-2 py-2 text-left text-sm hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring ${data.selected?.job === data.job.id && data.selected?.step === index ? "bg-accent" : ""}`} onClick={() => data.select({ job: data.job.id, step: index })}><span className="text-xs text-muted-foreground">{index + 1}</span><span className="truncate">{step.name || step.uses || "执行命令"}</span></button>)}{data.job.raw["ferry-approval"] === true && <p className="p-2 text-xs text-muted-foreground">等待人工确认后继续</p>}{!data.job.steps.length && data.job.raw["ferry-approval"] !== true && <p className="p-2 text-xs text-muted-foreground">可复用工作流：在 YAML 中配置</p>}
      {!data.readOnly && !data.job.raw.uses && data.job.raw["ferry-approval"] !== true && <Button type="button" variant="ghost" size="sm" className="nodrag nopan w-full justify-start" onClick={event => { event.stopPropagation(); data.addStep(data.job.id); }}><Plus className="size-4" />添加步骤</Button>}
    </div>
    {!data.readOnly && <div className="flex gap-1 border-t p-1.5">
      <Button type="button" variant="ghost" size="sm" className="nodrag nopan flex-1" title="添加并行任务" onClick={event => { event.stopPropagation(); data.addJob(data.job.id, "parallel"); }}><GitFork className="size-4" />并行</Button>
      <Button type="button" variant="ghost" size="sm" className="nodrag nopan flex-1" title="在此任务后添加串行任务" onClick={event => { event.stopPropagation(); data.addJob(data.job.id, "serial"); }}><ArrowRight className="size-4" />串行</Button>
    </div>}
    <Handle type="source" position={Position.Right} />
  </div>;
}
const nodeTypes = { job: JobCard };
const YamlEditor = lazy(() => import("./YamlEditor").then(module => ({ default: module.YamlEditor })));
function FitCanvas({ topology }: { topology: string }) {
  const { fitView } = useReactFlow();
  const initialized = useNodesInitialized();
  const marker = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!initialized) return;
    let timer: ReturnType<typeof setTimeout>;
    const fit = () => { clearTimeout(timer); timer = setTimeout(() => void fitView({ padding: 0.18, maxZoom: 1 }), 80); };
    const observer = new ResizeObserver(fit);
    if (marker.current?.parentElement) observer.observe(marker.current.parentElement);
    fit();
    return () => { clearTimeout(timer); observer.disconnect(); };
  }, [initialized, topology, fitView]);
  return <span ref={marker} hidden />;
}
const inputClass = "w-full rounded-md border border-input bg-background px-3 py-2 text-sm";

export function ActionsEditor({ draft, options, readOnly, fillHeight = false, mode, onModeChange, onChange, onValidityChange, renderCheckout }: {
  draft: Record<string, unknown>; options: Option[]; readOnly: boolean;
  fillHeight?: boolean;
  mode?: "visual" | "yaml";
  onModeChange?: (mode: "visual" | "yaml") => void;
  onChange: (patch: Record<string, unknown>) => void; onValidityChange: (invalid: boolean) => void;
  renderCheckout?: (jobId: string, stepIndex: number) => ReactNode;
}) {
  const source = String(draft.definition ?? "");
  const [internalTab, setInternalTab] = useState<"visual" | "yaml">("visual");
  const tab = mode ?? internalTab;
  const setTab = onModeChange ?? setInternalTab;
  const [selection, setSelection] = useState<Selection>();
  const [measurements, setMeasurements] = useState<Record<string, { width: number; height: number }>>({});
  const [selectedNodes, setSelectedNodes] = useState<string[]>([]);
  const [selectedEdge, setSelectedEdge] = useState<{ source: string; target: string }>();
  const [error, setError] = useState("");
  const [pendingDelete, setPendingDelete] = useState<{ action: string; target: string; run: () => void }>();
  const panel = useRef<HTMLElement>(null);
  const parsed = useMemo(() => { try { return { ...parseActions(source), error: "" }; } catch (cause) { return { jobs: [] as ActionJob[], error: cause instanceof Error ? cause.message : String(cause) }; } }, [source]);
  const bindings = (draft.executor_bindings ?? []) as Binding[];
  const missingHosts = parsed.jobs.filter(job => job.raw["ferry-approval"] !== true && !bindings.some(binding => binding.job_id === job.id && binding.target_ref && binding.target_ref !== "0"));
  useEffect(() => { onValidityChange(!!parsed.error || missingHosts.length > 0); return () => onValidityChange(false); }, [parsed.error, missingHosts.length, onValidityChange]);
  useEffect(() => { panel.current?.querySelector<HTMLInputElement>("input")?.focus(); }, [selection?.job, selection?.step]);
  const layout = (draft.graph_layout ?? {}) as Record<string, { x: number; y: number }>;
  // checkout_bindings use { job_id, step_id, ...repository fields }; step_id is the stable YAML step.id assigned by the checkout form.
  const checkoutBindings = (draft.checkout_bindings ?? []) as { job_id: string; step_id: string }[];
  const nodes = useMemo(() => {
    const levels = new Map<string, number>(), y = new Map<number, number>();
    const byId = new Map(parsed.jobs.map(job => [job.id, job]));
    const level = (job: ActionJob): number => {
      if (levels.has(job.id)) return levels.get(job.id)!;
      const depth = job.needs.length ? Math.max(...job.needs.map(id => level(byId.get(id)!))) + 1 : 0;
      levels.set(job.id, depth); return depth;
    };
    const size = (job: ActionJob) => ({ width: measurements[job.id]?.width || 288, height: measurements[job.id]?.height || 155 + job.steps.length * 40 });
    const positions = new Map<string, { x: number; y: number }>();
    const occupied = parsed.jobs.filter(job => layout[job.id]).map(job => ({ ...layout[job.id]!, ...size(job) }));
    for (const job of parsed.jobs) if (layout[job.id]) positions.set(job.id, layout[job.id]!);
    for (const job of [...parsed.jobs].sort((a, b) => level(a) - level(b))) {
      if (positions.has(job.id)) continue;
      const depth = level(job);
      const x = Math.max(depth * 360 + 24, ...job.needs.map(id => (positions.get(id)?.x ?? 24) + size(byId.get(id)!).width + 72));
      let top = y.get(depth) ?? 32;
      const dimensions = size(job);
      while (true) {
        const collisions = occupied.filter(other => x < other.x + other.width + 28 && x + dimensions.width + 28 > other.x && top < other.y + other.height + 28 && top + dimensions.height + 28 > other.y);
        if (!collisions.length) break;
        top = Math.max(...collisions.map(other => other.y + other.height + 28));
      }
      positions.set(job.id, { x, y: top });
      occupied.push({ x, y: top, ...dimensions });
      y.set(depth, top + dimensions.height + 28);
    }
    return parsed.jobs.map(job => {
      return { id: job.id, measured: measurements[job.id] ?? {}, selected: selectedNodes.includes(job.id), type: "job" as const, dragHandle: ".job-drag", position: positions.get(job.id)!, data: { job, readOnly, canDelete: parsed.jobs.length > 1, missingHost: missingHosts.some(row => row.id === job.id), selected: selection, select: setSelection, addJob, addStep, removeJob } };
    });
  }, [parsed.jobs, layout, selection, readOnly, selectedNodes, measurements, source]);
  const edges = parsed.jobs.flatMap(job => job.needs.map(id => ({ id: `${id}->${job.id}`, source: id, target: job.id })));
  const job = parsed.jobs.find(row => row.id === selection?.job);
  const step = selection?.step === undefined ? undefined : job?.steps[selection.step];
  const parameters = step?.with && typeof step.with === "object" && !Array.isArray(step.with) ? step.with as Record<string, unknown> : {};
  const binding = bindings.find(row => row.job_id === job?.id);
  const targetKind = binding?.target_kind === "host-group" ? "host-group" : "host";
  const targetOptions = options.filter(option => option.value.startsWith(`${targetKind}:`));
  function change(transform: () => string) {
    try {
      if (readOnly) return;
      const definition = transform();
      const nextJobs = parseActions(definition).jobs;
      const ids = new Set(nextJobs.map(job => job.id));
      onChange({ definition, checkout_bindings: checkoutBindings.filter(binding => nextJobs.some(job => job.id === binding.job_id && job.steps.some(step => step.id === binding.step_id && /^actions\/checkout@/.test(String(step.uses ?? ""))))), executor_bindings: bindings.filter(binding => ids.has(binding.job_id) && nextJobs.find(job => job.id === binding.job_id)?.raw["ferry-approval"] !== true), graph_layout: Object.fromEntries(Object.entries(layout).filter(([id]) => ids.has(id))) }); setError("");
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  }
  function update(values: Record<string, unknown>) {
    if (!job) return;
    change(() => selection?.step === undefined ? editActionJob(source, job.id, values) : editActionStep(source, job.id, selection.step, values));
  }
  function updateBinding(values: Partial<Binding>) {
    if (!job) return;
    onChange({ executor_bindings: [...bindings.filter(row => row.job_id !== job.id), { job_id: job.id, target_kind: "", target_ref: "", image: "host", ...binding, ...values }] });
  }
  function addJob(anchor: string, mode: "serial" | "parallel") {
    if (readOnly) return;
    try {
      const added = addActionJob(source, false, { job: anchor, mode });
      onChange({ definition: added.source });
      setSelection({ job: added.id });
      setError("");
    } catch (cause) { setError(String(cause)); }
  }
  function removeJob(id: string) {
    const target = parsed.jobs.find(row => row.id === id)?.name ?? id;
    setPendingDelete({ action: "删除任务", target, run: () => {
      change(() => removeActionJob(source, id));
      setSelection(undefined);
    } });
  }
  function addStep(id: string) {
    if (readOnly) return;
    try {
      const index = parsed.jobs.find(row => row.id === id)?.steps.length;
      if (index === undefined) return;
      onChange({ definition: addActionStep(source, id) });
      setSelection({ job: id, step: index });
      setError("");
    } catch (cause) { setError(String(cause)); }
  }
  const onConnect = (edge: Connection) => { if (edge.source && edge.target) change(() => connectActionJobs(source, edge.source!, edge.target!)); };
  return <div className={cn("min-w-0 space-y-3", fillHeight && "flex min-h-0 flex-1 flex-col")}>
    {pendingDelete && <DeleteConfirmDialog action={pendingDelete.action} target={pendingDelete.target} description="将从当前流水线草稿移除，保存流水线后生效。" onClose={() => setPendingDelete(undefined)} onConfirm={pendingDelete.run} />}
    {mode === undefined && <div className="flex gap-1" role="tablist" aria-label="流水线编辑模式">{([["visual", "可视化"], ["yaml", "YAML"]] as const).map(([key, title]) => <Button key={key} type="button" role="tab" aria-selected={tab === key} variant={tab === key ? "secondary" : "ghost"} onClick={() => setTab(key)}>{title}</Button>)}</div>}
    {tab === "yaml" && missingHosts.length > 0 && <div role="alert" className="flex flex-wrap items-center gap-2 text-sm text-destructive">未选择执行主机：{missingHosts.map(row => <Button key={row.id} type="button" variant="outline" size="sm" onClick={() => { setSelection({ job: row.id }); setTab("visual"); }}>{row.name}</Button>)}</div>}
    {selectedEdge && !readOnly && <Button type="button" variant="outline" size="icon" aria-label="删除依赖连接" title="删除依赖连接" onClick={() => {
      const edge = selectedEdge;
      setPendingDelete({ action: "删除依赖连接", target: `${parsed.jobs.find(row => row.id === edge.source)?.name ?? edge.source} → ${parsed.jobs.find(row => row.id === edge.target)?.name ?? edge.target}`, run: () => {
        change(() => editActionJob(source, edge.target, { needs: parsed.jobs.find(row => row.id === edge.target)!.needs.filter(id => id !== edge.source) }));
        setSelectedEdge(undefined);
      } });
    }}><Unlink2 className="size-4" /></Button>}
    {tab === "yaml" ? <Suspense fallback={<div role="status" className="grid h-[520px] place-items-center text-sm text-muted-foreground">正在加载 YAML 编辑器…</div>}><YamlEditor value={source} readOnly={readOnly} fillHeight={fillHeight} onChange={definition => {
      try { parseActions(definition); } catch { onChange({ definition }); return; }
      change(() => definition);
    }} /></Suspense> : <div className={cn("flex min-h-[520px] flex-col overflow-hidden rounded-lg border lg:flex-row", fillHeight ? "flex-1 lg:min-h-0" : "lg:h-[620px]")}>
      <div className="h-[520px] min-w-0 shrink-0 lg:h-full lg:flex-1 lg:shrink" aria-label="流水线画布">
        {!parsed.error && <ReactFlow nodes={nodes} edges={edges} nodeTypes={nodeTypes} nodesDraggable={!readOnly} nodesConnectable={!readOnly} edgesFocusable={!readOnly} deleteKeyCode={null} onConnect={onConnect} onEdgeClick={(_, edge) => setSelectedEdge({ source: edge.source, target: edge.target })} onPaneClick={() => setSelectedEdge(undefined)} onNodesChange={changes => {
          const dimensions = changes.flatMap(change => change.type === "dimensions" && change.dimensions ? [[change.id, change.dimensions] as const] : []);
          if (dimensions.length) setMeasurements(current => ({ ...current, ...Object.fromEntries(dimensions) }));
          const selections = changes.filter(change => change.type === "select");
          if (selections.length) setSelectedNodes(current => {
            const ids = new Set(current);
            for (const change of selections) { if (change.selected) ids.add(change.id); else ids.delete(change.id); }
            return [...ids];
          });
          if (readOnly) return;
          const positions = changes.flatMap(change => change.type === "position" && change.position ? [[change.id, change.position] as const] : []);
          if (positions.length) onChange({ graph_layout: { ...layout, ...Object.fromEntries(positions) } });
        }} fitView minZoom={0.25} maxZoom={1.5} ariaLabelConfig={{ "controls.zoomIn.ariaLabel": "放大画布", "controls.zoomOut.ariaLabel": "缩小画布", "controls.fitView.ariaLabel": "适应画布" }}><FitCanvas topology={parsed.jobs.map(job => `${job.id}:${job.needs.join(",")}:${job.steps.length}`).join(";")} /><Background /><Controls showInteractive={false} /></ReactFlow>}
      </div>
      {job && <aside ref={panel} aria-label={step ? "步骤配置" : "任务配置"} className={cn("w-full space-y-4 overflow-y-auto border-t bg-background p-4 max-h-[620px] lg:w-80 lg:shrink-0 lg:border-l lg:border-t-0", fillHeight && "lg:max-h-none")}>
        <div className="flex items-center justify-between"><h3 className="font-semibold">{step ? "步骤配置" : "任务配置"}</h3><Button type="button" variant="ghost" size="icon" aria-label="关闭配置面板" onClick={() => setSelection(undefined)}><X className="size-4" /></Button></div>
        <AutoField label={<span className="text-sm">名称</span>} className="block space-y-2"><Input aria-label={step ? "步骤名称" : "任务名称"} value={String(step?.name ?? (!step ? job.raw.name : "") ?? "")} disabled={readOnly} onChange={e => update({ name: e.target.value })} /></AutoField>
        {!step ? <>
          <AutoField label={<span className="text-sm">任务类型</span>} className="block space-y-2"><select aria-label="任务类型" className={inputClass} disabled={readOnly} value={job.raw["ferry-approval"] === true ? "approval" : "actions"} onChange={e => {
            const manual = e.target.value === "approval";
            change(() => editActionJob(source, job.id, manual ? { "ferry-approval": true, "runs-on": null, steps: null, "timeout-minutes": null } : { "ferry-approval": null, "runs-on": "self-hosted", steps: [{ name: "步骤1", run: "echo hello" }] }));
          }}><option value="actions">执行任务</option><option value="approval">人工卡点</option></select></AutoField>
          {job.raw["ferry-approval"] !== true && <><p className="text-xs text-muted-foreground">任务标识：{job.id}。同一任务内的步骤共享执行器和工作目录。</p>
          <AutoField label={<span className="text-sm">执行目标类型</span>} className="block space-y-2"><select aria-label="执行目标类型" className={inputClass} disabled={readOnly} value={targetKind} onChange={e => updateBinding({ target_kind: e.target.value, target_ref: "" })}><option value="host">主机</option><option value="host-group">主机组</option></select></AutoField>
          <AutoField label={<span className="text-sm">{targetKind === "host" ? "执行主机" : "执行主机组"}</span>} className="block space-y-2"><select aria-label="任务执行器" aria-invalid={!binding?.target_ref} className={cn(inputClass, !binding?.target_ref && "border-destructive")} disabled={readOnly} value={binding?.target_kind === targetKind ? binding.target_ref : ""} onChange={e => updateBinding({ target_kind: targetKind, target_ref: e.target.value })}><option value="" disabled>请选择{targetKind === "host" ? "主机" : "主机组"}</option>{targetOptions.map(option => <option key={option.value} value={option.value.slice(targetKind.length + 1)}>{option.label.replace(/^(主机|主机组)\s*[·路]\s*/, "")}</option>)}</select>{!binding?.target_ref && <span className="text-xs text-destructive">保存前必须为此任务选择执行目标</span>}</AutoField>
          <AutoField label={<span className="text-sm">运行方式</span>} className="block space-y-2"><select aria-label="任务运行方式" className={inputClass} disabled={readOnly} value={binding?.image && binding.image !== "host" ? "container" : "host"} onChange={e => updateBinding({ image: e.target.value === "host" ? "host" : "node:20-bookworm" })}><option value="host">直接在执行主机运行</option><option value="container">在 Docker 容器运行</option></select></AutoField>
          {binding?.image && binding.image !== "host" && <AutoField label={<span className="text-sm">执行镜像</span>} className="block space-y-2"><Input aria-label="执行镜像" value={binding.image} disabled={readOnly} onChange={e => updateBinding({ image: e.target.value })} /></AutoField>}
          <AutoField label={<span className="text-sm">执行器标签（runs-on）</span>} className="block space-y-2"><Input value={job.runsOn.join(", ")} disabled={readOnly} onChange={e => update({ "runs-on": e.target.value.split(",").map(value => value.trim()).filter(Boolean) })} /></AutoField>
          </>}
          <fieldset className="space-y-2"><legend className="text-sm">前置任务</legend>{parsed.jobs.filter(row => row.id !== job.id).map(row => <label key={row.id} className="flex min-h-10 items-center gap-2 text-sm"><input type="checkbox" checked={job.needs.includes(row.id)} disabled={readOnly} onChange={e => update({ needs: e.target.checked ? [...job.needs, row.id] : job.needs.filter(id => id !== row.id) })} />{row.name}</label>)}<p className="text-xs text-muted-foreground">全部前置任务完成后执行；取消勾选可删除连接。</p></fieldset>
        </> : <>
          <AutoField label={<span className="text-sm">步骤类型</span>} className="block space-y-2"><select aria-label="步骤类型" className={inputClass} value={/^actions\/checkout@/.test(String(step.uses ?? "")) ? "checkout" : step.uses ? "uses" : "run"} disabled={readOnly} onChange={e => update(e.target.value !== "run" ? { uses: e.target.value === "checkout" ? "actions/checkout@v4" : "actions/setup-node@v4", run: null, shell: null } : { run: "echo hello", uses: null, with: null })}><option value="run">执行命令</option><option value="checkout">拉取代码</option><option value="uses">使用 Action</option></select></AutoField>
          {/^actions\/checkout@/.test(String(step.uses ?? "")) && selection?.step !== undefined && renderCheckout?.(job.id, selection.step)}
          {step.uses ? <><AutoField label={<span className="text-sm">Action</span>} className="block space-y-2"><Input aria-label="Action 引用" value={step.uses} disabled={readOnly} onChange={e => update({ uses: e.target.value })} /><span className="text-xs text-muted-foreground">例如 actions/setup-node@v4</span></AutoField>
            <div className="space-y-2"><div className="flex items-center justify-between"><span className="text-sm">参数 (with)</span>{!readOnly && <Button type="button" variant="ghost" size="sm" onClick={() => { let index = 1; while (`input_${index}` in parameters) index++; update({ with: { ...parameters, [`input_${index}`]: "" } }); }}><Plus className="size-4" />添加参数</Button>}</div>
              {Object.entries(parameters).map(([key, value], index) => <div key={index} className="flex items-center gap-2"><Input aria-label="参数名" className="min-w-0 flex-1" value={key} disabled={readOnly} onChange={e => { const next = e.target.value.trim(); if (!next || (next !== key && next in parameters)) return; update({ with: Object.fromEntries(Object.entries(parameters).map(([name, item]) => [name === key ? next : name, item])) }); }} /><Input aria-label={`${key} 参数值`} className="min-w-0 flex-1" value={String(value ?? "")} disabled={readOnly} onChange={e => update({ with: { ...parameters, [key]: e.target.value } })} />{!readOnly && <Button type="button" variant="ghost" size="icon" aria-label={`删除参数 ${key}`} title="删除参数" onClick={() => setPendingDelete({ action: "删除参数", target: key, run: () => { const next = { ...parameters }; delete next[key]; update({ with: Object.keys(next).length ? next : null }); } })}><Trash2 className="size-4" /></Button>}</div>)}
            </div></> : <>
            <AutoField label={<span className="text-sm">执行命令</span>} className="block space-y-2"><textarea aria-label="执行命令" className={`${inputClass} min-h-36 font-mono`} value={step.run ?? ""} readOnly={readOnly} onChange={e => update({ run: e.target.value })} /></AutoField>
            <AutoField label={<span className="text-sm">Shell</span>} className="block space-y-2"><Input aria-label="Shell" placeholder="使用默认 Shell" value={String(step.shell ?? "")} disabled={readOnly} onChange={e => update({ shell: e.target.value || null })} /></AutoField>
            <AutoField label={<span className="text-sm">工作目录</span>} className="block space-y-2"><Input value={String(step["working-directory"] ?? "")} disabled={readOnly} placeholder="默认仓库根目录" onChange={e => update({ "working-directory": e.target.value || null })} /></AutoField>
          </>}
        </>}
        {(step || job.raw["ferry-approval"] !== true) && <AutoField label={<span className="text-sm">超时（分钟）</span>} className="block space-y-2"><Input aria-label="超时（分钟）" type="text" inputMode="numeric" placeholder={step ? "继承任务剩余时间" : "默认 30 分钟"} value={String((step ?? job.raw)["timeout-minutes"] ?? "")} disabled={readOnly} onChange={e => update({ "timeout-minutes": e.target.value ? (/^\d+$/.test(e.target.value) ? Number(e.target.value) : e.target.value) : null })} /></AutoField>}
        {!readOnly && step && selection?.step !== undefined && <Button type="button" variant="destructive" size="icon" aria-label={`删除步骤 ${step.name || step.uses || selection.step + 1}`} title="删除步骤" onClick={() => {
          const jobId = job.id, index = selection.step!;
          setPendingDelete({ action: "删除步骤", target: `${job.name} / ${step.name || step.uses || index + 1}`, run: () => {
            change(() => removeActionStep(source, jobId, index));
            setSelection(undefined);
          } });
        }}><Trash2 className="size-4" /></Button>}
      </aside>}
    </div>}
    {(parsed.error || error) && <p role="alert" className="whitespace-pre-wrap text-sm text-destructive">{parsed.error || error}</p>}
  </div>;
}
