import { useEffect, useMemo, useRef, useState } from "react";
import { ReactFlow, Background, Controls, Handle, Position, type NodeProps, type Node, type Connection } from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import { GripVertical, Plus, X } from "lucide-react";
import { Button } from "./ui/button";
import { Input } from "./ui/input";
import type { Option } from "./dsl";
import { addActionJob, addActionStep, connectActionJobs, editActionJob, editActionStep, parseActions, removeActionJob, removeActionStep, type ActionJob } from "../actions-workflow";

type Binding = { job_id: string; target_kind: string; target_ref: string; image: string };
type Selection = { job: string; step?: number };
type JobData = { job: ActionJob; selected?: Selection; readOnly: boolean; select: (value: Selection) => void };
type JobNode = Node<JobData, "job">;
function JobCard({ data }: NodeProps<JobNode>) {
  return <div className={`w-72 rounded-lg border bg-card shadow-sm ${data.selected?.job === data.job.id ? "border-primary ring-1 ring-primary" : "border-border"}`}>
    <Handle type="target" position={Position.Left} />
    <div className="job-drag flex items-center gap-2 border-b p-3"><GripVertical className="size-4 text-muted-foreground" /><button type="button" className="nodrag min-w-0 flex-1 text-left font-medium focus-visible:ring-2 focus-visible:ring-ring" onClick={() => data.select({ job: data.job.id })}>{data.job.name}</button></div>
    <div className="space-y-1 p-2">{data.job.steps.map((step, index) => <button key={index} type="button" className={`nodrag flex w-full items-center gap-2 rounded px-2 py-2 text-left text-sm hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring ${data.selected?.job === data.job.id && data.selected?.step === index ? "bg-accent" : ""}`} onClick={() => data.select({ job: data.job.id, step: index })}><span className="text-xs text-muted-foreground">{index + 1}</span><span className="truncate">{step.name || step.uses || "执行命令"}</span></button>)}{!data.job.steps.length && <p className="p-2 text-xs text-muted-foreground">可复用工作流：在 YAML 中配置</p>}</div>
    <Handle type="source" position={Position.Right} />
  </div>;
}
const nodeTypes = { job: JobCard };
const inputClass = "w-full rounded-md border border-input bg-background px-3 py-2 text-sm";

export function ActionsEditor({ draft, options, readOnly, onChange, onValidityChange }: {
  draft: Record<string, unknown>; options: Option[]; readOnly: boolean;
  onChange: (patch: Record<string, unknown>) => void; onValidityChange: (invalid: boolean) => void;
}) {
  const source = String(draft.definition ?? "");
  const [tab, setTab] = useState("visual");
  const [selection, setSelection] = useState<Selection>();
  const [selectedNodes, setSelectedNodes] = useState<string[]>([]);
  const [error, setError] = useState("");
  const panel = useRef<HTMLElement>(null);
  const parsed = useMemo(() => { try { return { ...parseActions(source), error: "" }; } catch (cause) { return { jobs: [] as ActionJob[], error: cause instanceof Error ? cause.message : String(cause) }; } }, [source]);
  useEffect(() => { onValidityChange(!!parsed.error); return () => onValidityChange(false); }, [parsed.error, onValidityChange]);
  useEffect(() => { panel.current?.querySelector<HTMLInputElement>("input")?.focus(); }, [selection?.job, selection?.step]);
  const layout = (draft.graph_layout ?? {}) as Record<string, { x: number; y: number }>;
  const bindings = (draft.executor_bindings ?? []) as Binding[];
  const nodes = useMemo(() => {
    const levels = new Map<string, number>(), y = new Map<number, number>();
    const level = (job: ActionJob): number => {
      if (levels.has(job.id)) return levels.get(job.id)!;
      const depth = job.needs.length ? Math.max(...job.needs.map(id => level(parsed.jobs.find(row => row.id === id)!))) + 1 : 0;
      levels.set(job.id, depth); return depth;
    };
    return parsed.jobs.map(job => {
      const depth = level(job), top = y.get(depth) || 32; y.set(depth, top + 115 + job.steps.length * 42);
      return { id: job.id, selected: selectedNodes.includes(job.id), type: "job" as const, dragHandle: ".job-drag", position: layout[job.id] || { x: depth * 360 + 24, y: top }, data: { job, readOnly, selected: selection, select: setSelection } };
    });
  }, [parsed.jobs, layout, selection, readOnly, selectedNodes]);
  const edges = parsed.jobs.flatMap(job => job.needs.map(id => ({ id: `${id}->${job.id}`, source: id, target: job.id })));
  const job = parsed.jobs.find(row => row.id === selection?.job);
  const step = selection?.step === undefined ? undefined : job?.steps[selection.step];
  const binding = bindings.find(row => row.job_id === job?.id);
  function change(transform: () => string) {
    try {
      if (readOnly) return;
      const definition = transform();
      const ids = new Set(parseActions(definition).jobs.map(job => job.id));
      onChange({ definition, executor_bindings: bindings.filter(binding => ids.has(binding.job_id)), graph_layout: Object.fromEntries(Object.entries(layout).filter(([id]) => ids.has(id))) }); setError("");
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
  const onConnect = (edge: Connection) => { if (edge.source && edge.target) change(() => connectActionJobs(source, edge.source!, edge.target!)); };
  return <div className="min-w-0 space-y-3">
    <div className="flex flex-wrap items-center justify-between gap-2"><div className="flex gap-1" role="tablist" aria-label="流水线编辑模式">{([["visual", "可视化"], ["yaml", "YAML"]] as const).map(([key, title]) => <Button key={key} type="button" role="tab" aria-selected={tab === key} variant={tab === key ? "secondary" : "ghost"} onClick={() => setTab(key)}>{title}</Button>)}</div>{!readOnly && <Button type="button" variant="outline" disabled={!!parsed.error} onClick={() => { try { const added = addActionJob(source, !!draft.repository_id || !!(draft.repository_selection as { url?: string } | undefined)?.url); onChange({ definition: added.source }); setSelection({ job: added.id }); setError(""); } catch (cause) { setError(String(cause)); } }}><Plus className="size-4" />添加任务</Button>}</div>
    {tab === "yaml" ? <label className="block space-y-2"><span className="text-sm font-medium">GitHub Actions YAML</span><textarea aria-label="GitHub Actions YAML" className={`${inputClass} min-h-[440px] font-mono`} spellCheck={false} value={source} readOnly={readOnly} onChange={event => onChange({ definition: event.target.value })} /></label> : <div className="flex min-h-[520px] flex-col overflow-hidden rounded-lg border xl:h-[620px] xl:flex-row">
      <div className="h-[520px] min-w-0 flex-1 xl:h-full" aria-label="流水线画布">
        {!parsed.error && <ReactFlow nodes={nodes} edges={edges} nodeTypes={nodeTypes} nodesDraggable={!readOnly} nodesConnectable={!readOnly} edgesFocusable={!readOnly} deleteKeyCode={null} onConnect={onConnect} onNodesChange={changes => {
          const selections = changes.filter(change => change.type === "select");
          if (selections.length) setSelectedNodes(current => {
            const ids = new Set(current);
            for (const change of selections) { if (change.selected) ids.add(change.id); else ids.delete(change.id); }
            return [...ids];
          });
          if (readOnly) return;
          const positions = changes.flatMap(change => change.type === "position" && change.position ? [[change.id, change.position] as const] : []);
          if (positions.length) onChange({ graph_layout: { ...layout, ...Object.fromEntries(positions) } });
        }} fitView minZoom={0.25} maxZoom={1.5} ariaLabelConfig={{ "controls.zoomIn.ariaLabel": "放大画布", "controls.zoomOut.ariaLabel": "缩小画布", "controls.fitView.ariaLabel": "适应画布" }}><Background /><Controls showInteractive={false} /></ReactFlow>}
      </div>
      {job && <aside ref={panel} aria-label={step ? "步骤配置" : "任务配置"} className="w-full space-y-4 overflow-y-auto border-t bg-background p-4 xl:w-80 xl:shrink-0 xl:border-l xl:border-t-0">
        <div className="flex items-center justify-between"><h3 className="font-semibold">{step ? "步骤配置" : "任务配置"}</h3><Button type="button" variant="ghost" size="icon" aria-label="关闭配置面板" onClick={() => setSelection(undefined)}><X className="size-4" /></Button></div>
        <label className="block space-y-2"><span className="text-sm">名称</span><Input aria-label={step ? "步骤名称" : "任务名称"} value={String(step?.name ?? (!step ? job.raw.name : "") ?? "")} disabled={readOnly} onChange={e => update({ name: e.target.value })} /></label>
        {!step ? <>
          <p className="text-xs text-muted-foreground">任务标识：{job.id}。同一任务内的步骤共享执行器和工作目录。</p>
          <label className="block space-y-2"><span className="text-sm">执行器</span><select aria-label="任务执行器" className={inputClass} disabled={readOnly} value={binding?.target_ref ? `${binding.target_kind}:${binding.target_ref}` : ""} onChange={e => { const [kind, ref] = e.target.value.split(":"); updateBinding({ target_kind: kind || "", target_ref: ref || "" }); }}><option value="">使用流水线默认执行器</option>{options.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}</select></label>
          <label className="block space-y-2"><span className="text-sm">运行方式</span><select aria-label="任务运行方式" className={inputClass} disabled={readOnly} value={binding?.image && binding.image !== "host" ? "container" : "host"} onChange={e => updateBinding({ image: e.target.value === "host" ? "host" : "node:20-bookworm" })}><option value="host">直接在执行主机运行</option><option value="container">在 Docker 容器运行</option></select></label>
          {binding?.image && binding.image !== "host" && <label className="block space-y-2"><span className="text-sm">执行镜像</span><Input aria-label="执行镜像" value={binding.image} disabled={readOnly} onChange={e => updateBinding({ image: e.target.value })} /></label>}
          <label className="block space-y-2"><span className="text-sm">执行器标签（runs-on）</span><Input value={job.runsOn.join(", ")} disabled={readOnly} onChange={e => update({ "runs-on": e.target.value.split(",").map(value => value.trim()).filter(Boolean) })} /></label>
          <label className="block space-y-2"><span className="text-sm">前置任务</span><select aria-label="前置任务" multiple className={`${inputClass} min-h-24`} value={job.needs} disabled={readOnly} onChange={e => update({ needs: [...e.target.selectedOptions].map(option => option.value) })}>{parsed.jobs.filter(row => row.id !== job.id).map(row => <option key={row.id} value={row.id}>{row.name}</option>)}</select></label>
          {!readOnly && <Button type="button" variant="outline" disabled={!!job.raw.uses} onClick={() => change(() => addActionStep(source, job.id))}>添加执行步骤</Button>}
        </> : <>
          <label className="block space-y-2"><span className="text-sm">步骤类型</span><select aria-label="步骤类型" className={inputClass} value={step.uses ? "uses" : "run"} disabled={readOnly} onChange={e => update(e.target.value === "uses" ? { uses: "actions/checkout@v4", run: null, shell: null } : { run: "echo hello", uses: null, with: null })}><option value="run">执行命令</option><option value="uses">使用 Action</option></select></label>
          {step.uses ? <label className="block space-y-2"><span className="text-sm">Action</span><Input aria-label="Action 引用" value={step.uses} disabled={readOnly} onChange={e => update({ uses: e.target.value })} /><span className="text-xs text-muted-foreground">例如 actions/checkout@v4；高级 with 参数在 YAML 中编辑。</span></label> : <>
            <label className="block space-y-2"><span className="text-sm">执行命令</span><textarea aria-label="执行命令" className={`${inputClass} min-h-36 font-mono`} value={step.run ?? ""} readOnly={readOnly} onChange={e => update({ run: e.target.value })} /></label>
            <label className="block space-y-2"><span className="text-sm">Shell</span><Input aria-label="Shell" placeholder="默认使用执行环境 Shell" value={String(step.shell ?? "")} disabled={readOnly} onChange={e => update({ shell: e.target.value || null })} /></label>
            <label className="block space-y-2"><span className="text-sm">工作目录</span><Input value={String(step["working-directory"] ?? "")} disabled={readOnly} placeholder="默认仓库根目录" onChange={e => update({ "working-directory": e.target.value || null })} /></label>
          </>}
        </>}
        <label className="block space-y-2"><span className="text-sm">超时（分钟）</span><Input aria-label="超时（分钟）" type="text" inputMode="numeric" placeholder={step ? "继承任务剩余时间" : "默认 30 分钟"} value={String((step ?? job.raw)["timeout-minutes"] ?? "")} disabled={readOnly} onChange={e => update({ "timeout-minutes": e.target.value ? (/^\d+$/.test(e.target.value) ? Number(e.target.value) : e.target.value) : null })} /></label>
        {!readOnly && <Button type="button" variant="destructive" onClick={() => { change(() => step && selection?.step !== undefined ? removeActionStep(source, job.id, selection.step) : removeActionJob(source, job.id)); setSelection(undefined); }}>{step ? "删除步骤" : "删除任务"}</Button>}
      </aside>}
    </div>}
    {(parsed.error || error) && <p role="alert" className="whitespace-pre-wrap text-sm text-destructive">{parsed.error || error}</p>}
    <p className="text-xs text-muted-foreground">连接任务表示执行依赖；点击任务或步骤打开配置。YAML 中的高级字段会保留，实际执行能力以校验结果为准。</p>
  </div>;
}
