import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { EmptyWorkflowCanvas } from "../../shared/editor/EmptyWorkflowCanvas";
import { ActionFragmentYaml, WorkflowFragmentEditor } from "./WorkflowFragmentEditor";
import { ActionsJobSettings, ActionsRunnerFields, ActionsWorkflowSettings } from "./ActionsSettings";
import { ActionsConditionField, ActionsPhase } from "./ActionsExecutionFields";
import { StepMap } from "./ActionMapFields";
import { FieldCaption, FieldHelpContext } from "../../shared/forms/FieldHelp";
import { actionsFieldHelp } from "./ActionsFieldHelp";
import { ActionsVariableHelper } from "./ActionsVariableHelper";
import { Background,Controls,Handle,Position,ReactFlow,type Connection,type Node,type NodeProps } from "@xyflow/react";
import { FitCanvas } from "../../shared/editor/FitCanvas";
import { InsertEdge } from "../../shared/editor/InsertEdge";
import { insertLayout } from "../../shared/editor/insertion-layout";
import "@xyflow/react/dist/style.css";
import { cn } from "cn";
import { ArrowRight,GitFork,GripVertical,Plus,Trash2,Unlink2,X } from "lucide-react";
import { lazy,Suspense,useEffect,useMemo,useRef,useState,type ReactNode } from "react";
import { AutoField } from "../../shared/forms/auto-form";
import { commonText,executionText,useCommonTranslation,workflowText } from "../../shared/i18n/i18n";
import type { Option } from "../../shared/resources/types";
import { Button } from "../../shared/ui/button";
import { DeleteConfirmDialog } from "../../shared/ui/DeleteConfirmDialog";
import { Input } from "../../shared/ui/input";
import { ContextMenu } from "radix-ui";
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuItem } from "../../shared/ui/dropdown-menu";
import { addActionJob,addActionStep,connectActionJobs,duplicateActionJob,duplicateActionStep,editActionJob,editActionStep,editActionWorkflow,insertActionJob,parseActions,removeActionJob,removeActionStep,setActionJobApproval,type ActionJob } from "./actions-workflow";

type Binding = { job_id: string; target_kind: string; target_ref: string; image: string };
type Selection = { job: string; step?: number };
type JobData = { job: ActionJob; selected?: Selection; readOnly: boolean; canDelete: boolean; missingHost: boolean; select: (value: Selection) => void; addJob: (job: string, mode: "serial" | "parallel") => void; addStep: (job: string, kind?: "run" | "checkout" | "action") => void; removeJob: (job: string) => void };
type JobNode = Node<JobData, "job">;
function JobCard({ data }: NodeProps<JobNode>) {
  useCommonTranslation();
  return <div className={`w-72 rounded-lg border bg-card shadow-sm ${data.missingHost ? "border-destructive ring-1 ring-destructive" : data.selected?.job === data.job.id ? "border-primary ring-1 ring-primary" : "border-border"}`}>
    <Handle type="target" position={Position.Left} />
    <div className="flex items-center gap-2 border-b p-3"><span className="job-drag flex size-5 shrink-0 cursor-grab items-center justify-center text-muted-foreground active:cursor-grabbing" title={executionText("editor.dragJob")}><GripVertical className="size-4" /></span><button type="button" className="nodrag min-w-0 flex-1 text-left font-medium focus-visible:ring-2 focus-visible:ring-ring" onClick={() => data.select({ job: data.job.id })}>{data.job.name}</button>
      {!data.readOnly && <Button type="button" variant="ghost" size="icon" className="nodrag nopan size-7" aria-label={executionText("editor.deleteJobName", { name: data.job.name })} title={executionText("editor.deleteJob")} disabled={!data.canDelete} onClick={event => { event.stopPropagation(); data.removeJob(data.job.id); }}><Trash2 className="size-4" /></Button>}
    </div>
    <div className="space-y-1 p-2">{data.missingHost && <button type="button" className="nodrag w-full rounded px-2 py-1 text-left text-xs text-destructive" onClick={() => data.select({ job: data.job.id })}>{executionText("editor.chooseHost")}</button>}{data.job.steps.map((step, index) => <button key={index} data-action-step={index} type="button" className={`nodrag flex w-full items-center gap-2 rounded px-2 py-2 text-left text-sm hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring ${data.selected?.job === data.job.id && data.selected?.step === index ? "bg-accent" : ""}`} onClick={() => data.select({ job: data.job.id, step: index })}><span className="text-xs text-muted-foreground">{index + 1}</span><span className="truncate">{step.name || step.uses || executionText("shared.resource.cron.commandAction")}</span></button>)}{data.job.raw["ferry-approval"] === true && <p className="p-2 text-xs text-muted-foreground">{executionText("editor.approvalHelp")}</p>}{!data.job.steps.length && data.job.raw["ferry-approval"] !== true && <p className="p-2 text-xs text-muted-foreground">{executionText("editor.reusableHelp")}</p>}
      {!data.readOnly && !data.job.raw.uses && data.job.raw["ferry-approval"] !== true && <DropdownMenu><DropdownMenuTrigger asChild><Button type="button" variant="ghost" size="sm" className="nodrag nopan w-full justify-start"><Plus className="size-4" />{executionText("editor.addStep")}</Button></DropdownMenuTrigger><DropdownMenuContent>{(["run", "checkout", "action"] as const).map(kind => <DropdownMenuItem key={kind} onSelect={() => data.addStep(data.job.id, kind)}>{kind === "checkout" ? workflowText("actions.cloneCode") : kind === "run" ? executionText("shared.resource.cron.commandAction") : executionText("editor.useAction")}</DropdownMenuItem>)}</DropdownMenuContent></DropdownMenu>}
    </div>
    {!data.readOnly && <div className="flex gap-1 border-t p-1.5">
      <Button type="button" variant="ghost" size="sm" className="nodrag nopan flex-1" title={executionText("editor.addParallel")} onClick={event => { event.stopPropagation(); data.addJob(data.job.id, "parallel"); }}><GitFork className="size-4" />{executionText("editor.parallel")}</Button>
      <Button type="button" variant="ghost" size="sm" className="nodrag nopan flex-1" title={executionText("editor.addSerial")} onClick={event => { event.stopPropagation(); data.addJob(data.job.id, "serial"); }}><ArrowRight className="size-4" />{executionText("editor.serial")}</Button>
    </div>}
    <Handle type="source" position={Position.Right} />
  </div>;
}
const nodeTypes = { job: JobCard };
const edgeTypes = { insert: InsertEdge };
const YamlEditor = lazy(() => import("./YamlEditor").then(module => ({ default: module.YamlEditor })));
const inputClass = "w-full rounded-md border border-input bg-background px-3 py-2 text-sm";

export function ActionsEditor({ draft, options, readOnly, fillHeight = false, requireExecutorBinding = true, mode, onModeChange, onChange, onValidityChange, renderCheckout, renderCheckoutBatch }: {
  draft: Record<string, unknown>; options: Option[]; readOnly: boolean;
  fillHeight?: boolean;
  requireExecutorBinding?: boolean;
  mode?: "visual" | "yaml";
  onModeChange?: (mode: "visual" | "yaml") => void;
  onChange: (patch: Record<string, unknown>) => void; onValidityChange: (invalid: boolean) => void;
  renderCheckout?: (jobId: string, stepIndex: number) => ReactNode;
  renderCheckoutBatch?: (jobId: string) => ReactNode;
}) {
  const { i18n } = useCommonTranslation();
  const source = String(draft.definition ?? "");
  const [internalTab, setInternalTab] = useState<"visual" | "yaml">("visual");
  const originalSource = useRef(source);
  const tab = mode ?? internalTab;
  const setTab = onModeChange ?? setInternalTab;
  const [selection, setSelection] = useState<Selection>();
  const [workflowSettings, setWorkflowSettings] = useState(false);
  const select = (value: Selection) => { setWorkflowSettings(false); setSelection(value); };
  const [measurements, setMeasurements] = useState<Record<string, { width: number; height: number }>>({});
  const [selectedNodes, setSelectedNodes] = useState<string[]>([]);
  const [selectedEdge, setSelectedEdge] = useState<{ source: string; target: string }>();
  const [error, setError] = useState("");
  const [pendingDelete, setPendingDelete] = useState<{ action: string; target: string; run: () => void }>();
  const [pendingApproval, setPendingApproval] = useState<string>();
  const panel = useRef<HTMLElement>(null);
  const variableTarget = useRef<HTMLInputElement | HTMLTextAreaElement>(null);
  const [variableHelper, setVariableHelper] = useState(false);
  const undo = useRef<Record<string, unknown>[]>([]), redo = useRef<Record<string, unknown>[]>([]);
  const snapshot = () => ({ definition: source, executor_bindings: draft.executor_bindings ?? [], checkout_bindings: draft.checkout_bindings ?? [], graph_layout: draft.graph_layout ?? {} });
  function commit(patch: Record<string, unknown>) { undo.current = [...undo.current.slice(-99), snapshot()]; redo.current = []; onChange(patch); }
  function history(forward: boolean) {
    if (readOnly) return;
    const stack = forward ? redo.current : undo.current, next = stack.pop();
    if (!next) return;
    (forward ? undo.current : redo.current).push(snapshot()); onChange(next); setError("");
  }
  const parsed = useMemo(() => { try { return { ...parseActions(source, true), error: "" }; } catch (cause) { return { document: undefined, jobs: [] as ActionJob[], error: cause instanceof Error ? cause.message : String(cause) }; } }, [source, i18n.language]);
  const bindings = (draft.executor_bindings ?? []) as Binding[];
  const missingHosts = requireExecutorBinding ? parsed.jobs.filter(job => job.raw["ferry-approval"] !== true && !bindings.some(binding => binding.job_id === job.id && (binding.target_kind === "pool" || (binding.target_ref && binding.target_ref !== "0")))) : [];
  useEffect(() => { onValidityChange(!!parsed.error || !parsed.jobs.length || missingHosts.length > 0); return () => onValidityChange(false); }, [parsed.error, parsed.jobs.length, missingHosts.length, onValidityChange]);
  useEffect(() => { panel.current?.querySelector<HTMLInputElement>("input")?.focus(); }, [selection?.job, selection?.step, workflowSettings]);
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
      return { id: job.id, measured: measurements[job.id] ?? {}, selected: selectedNodes.includes(job.id), type: "job" as const, dragHandle: ".job-drag", position: positions.get(job.id)!, data: { job, readOnly, canDelete: parsed.jobs.length > 1, missingHost: missingHosts.some(row => row.id === job.id), selected: selection, select, addJob, addStep, removeJob } };
    });
  }, [parsed.jobs, layout, selection, readOnly, selectedNodes, measurements, source]);
  const edges = parsed.jobs.flatMap(job => job.needs.map(id => ({ id: `${id}->${job.id}`, source: id, target: job.id, type: "insert", data: readOnly ? {} : { insert: () => {
    try {
      const inserted = insertActionJob(source, id, job.id);
      const graph_layout = insertLayout(nodes.map(node => ({id:node.id,...node.position,width:measurements[node.id]?.width ?? 288,height:measurements[node.id]?.height ?? 155+(parsed.jobs.find(job=>job.id===node.id)?.steps.length ?? 1)*40})), parsed.jobs.flatMap(job=>job.needs.map(from=>({from,to:job.id}))), id, job.id, {id:inserted.id,x:0,y:0,width:288,height:195});
      change(() => inserted.source, {graph_layout});
      select({ job: inserted.id }); setSelectedEdge(undefined);
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  } } })));
  const job = parsed.jobs.find(row => row.id === selection?.job);
  const step = selection?.step === undefined ? undefined : job?.steps[selection.step];
  const binding = bindings.find(row => row.job_id === job?.id);
  const targetKind = binding?.target_kind === "pool" ? "pool" : binding?.target_kind === "host-group" ? "host-group" : "host";
  const targetOptions = options.filter(option => option.value.startsWith(`${targetKind}:`));
  const targetPrefix = `${commonText(targetKind === "host" ? "nav.hosts" : "nav.hostGroups")} · `;
  function change(transform: () => string, extra: Record<string, unknown> = {}) {
    try {
      if (readOnly) return;
      const definition = transform();
      const nextJobs = parseActions(definition, true).jobs;
      const ids = new Set(nextJobs.map(job => job.id));
      commit({ definition, checkout_bindings: checkoutBindings.filter(binding => nextJobs.some(job => job.id === binding.job_id && job.steps.some(step => step.id === binding.step_id && /^actions\/checkout@/.test(String(step.uses ?? ""))))), executor_bindings: bindings.filter(binding => ids.has(binding.job_id) && nextJobs.find(job => job.id === binding.job_id)?.raw["ferry-approval"] !== true), graph_layout: Object.fromEntries(Object.entries(layout).filter(([id]) => ids.has(id))), ...extra }); setError("");
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)); }
  }
  function update(values: Record<string, unknown>) {
    if (!job) return;
    change(() => selection?.step === undefined ? editActionJob(source, job.id, values) : editActionStep(source, job.id, selection.step, values));
  }
  function updateBinding(values: Partial<Binding>) {
    if (!job) return;
    if (readOnly) return;
    commit({ executor_bindings: [...bindings.filter(row => row.job_id !== job.id), { job_id: job.id, target_kind: "", target_ref: "", image: "host", ...binding, ...values }] });
  }
  function addJob(anchor: string, mode: "serial" | "parallel") {
    if (readOnly) return;
    try {
      const added = addActionJob(source, false, { job: anchor, mode });
      commit({ definition: added.source });
      select({ job: added.id });
      setError("");
    } catch (cause) { setError(String(cause)); }
  }
  function removeJob(id: string) {
    const target = parsed.jobs.find(row => row.id === id)?.name ?? id;
    setPendingDelete({ action: "editor.deleteJob", target, run: () => {
      change(() => removeActionJob(source, id));
      setSelection(undefined);
    } });
  }
  function addStep(id: string, kind: "run" | "checkout" | "action" = "run") {
    if (readOnly) return;
    try {
      const index = parsed.jobs.find(row => row.id === id)?.steps.length;
      if (index === undefined) return;
      commit({ definition: addActionStep(source, id, kind) });
      select({ job: id, step: index });
      setError("");
    } catch (cause) { setError(String(cause)); }
  }
  function duplicate() {
    if (readOnly || !job) return;
    if (selection?.step !== undefined) {
      change(() => duplicateActionStep(source, job.id, selection.step!));
      select({ job: job.id, step: job.steps.length });
    } else {
      const copy = duplicateActionJob(source, job.id);
      const position = nodes.find(node => node.id === job.id)?.position ?? {x:24,y:32};
      change(() => copy.source, { executor_bindings: [...bindings, ...bindings.filter(binding => binding.job_id === job.id).map(binding => ({...binding, job_id:copy.id}))], checkout_bindings: [...checkoutBindings, ...checkoutBindings.filter(binding => binding.job_id === job.id).map(binding => ({...binding,job_id:copy.id}))], graph_layout: {...layout,[copy.id]:{x:position.x,y:position.y+(measurements[job.id]?.height ?? 155+job.steps.length*40)+48}} });
      select({job:copy.id});
    }
  }
  function deleteSelection() {
    if (readOnly || !job) return;
    if (selection?.step === undefined) { if (parsed.jobs.length > 1) removeJob(job.id); }
    else if (job.steps.length > 1) {
      const index = selection.step;
      setPendingDelete({action:"editor.deleteStep",target:`${job.name} / ${step?.name ?? index+1}`,run:()=>{change(()=>removeActionStep(source,job.id,index));setSelection(undefined);}});
    }
  }
  const onConnect = (edge: Connection) => { if (edge.source && edge.target) change(() => connectActionJobs(source, edge.source!, edge.target!)); };
  return <FieldHelpContext.Provider value={actionsFieldHelp}><div className={cn("min-w-0 space-y-3", fillHeight && "flex min-h-0 flex-1 flex-col")} onKeyDown={event => {
    if (tab !== "visual" || event.nativeEvent.isComposing || (event.target as HTMLElement).closest("input,textarea,select,[contenteditable],[role='dialog'],[role='menu']")) return;
    const command = event.ctrlKey || event.metaKey;
    if (command && event.key.toLowerCase() === "z") {event.preventDefault();history(event.shiftKey);}
    else if (command && event.key.toLowerCase() === "y") {event.preventDefault();history(true);}
    else if (command && event.key.toLowerCase() === "d") {event.preventDefault();duplicate();}
    else if (event.key === "Delete" || event.key === "Backspace") {event.preventDefault();deleteSelection();}
    else if (event.key === "Escape") {setSelection(undefined);setSelectedEdge(undefined);setWorkflowSettings(false);}
    else if (event.key === "Enter") { const id=(event.target as HTMLElement).closest<HTMLElement>(".react-flow__node")?.dataset.id;if(id)select({job:id}); }
  }} onFocusCapture={event => {
    const element = event.target;
    if ((element instanceof HTMLInputElement || element instanceof HTMLTextAreaElement) && element.closest("[data-action-expression]") && !element.closest("[data-action-helper]") && !element.readOnly && !element.disabled && (element instanceof HTMLTextAreaElement || ["text", "search", ""].includes(element.type))) variableTarget.current = element;
  }}>
    {variableHelper && !readOnly && <ActionsVariableHelper jobs={parsed.jobs} job={job?.id} step={selection?.step} envNames={[...new Set([...Object.keys(parsed.document?.toJS()?.env ?? {}), ...Object.keys(job?.raw.env && typeof job.raw.env === "object" ? job.raw.env : {})])]} target={variableTarget} close={() => setVariableHelper(false)} />}
    {pendingDelete && <DeleteConfirmDialog action={executionText(pendingDelete.action)} target={pendingDelete.target} description={executionText("editor.deleteHelp")} onClose={() => setPendingDelete(undefined)} onConfirm={pendingDelete.run} />}
    {pendingApproval && <DeleteConfirmDialog action={workflowText("actions.convertApproval")} target={parsed.jobs.find(row => row.id === pendingApproval)?.name ?? pendingApproval} description={workflowText("actions.convertApprovalHelp")} onClose={() => setPendingApproval(undefined)} onConfirm={() => change(() => setActionJobApproval(source, pendingApproval, true))} />}
    {mode === undefined && <div className="flex gap-1" role="tablist" aria-label={executionText("editor.mode")}>{([["visual", executionText("shared.common.form.visual")], ["yaml", workflowText("advancedYaml")]] as const).map(([key, title]) => <Button key={key} type="button" role="tab" aria-selected={tab === key} variant={tab === key ? "secondary" : "ghost"} onClick={() => setTab(key)}>{title}</Button>)}</div>}
    {tab === "visual" && !parsed.error && <div className="flex flex-wrap items-center justify-between gap-3"><span className="text-xs text-muted-foreground">GitHub Actions</span><div className="flex gap-2">{!readOnly && <Button type="button" variant="outline" size="sm" aria-pressed={variableHelper} onClick={() => setVariableHelper(current => !current)}>{workflowText("actions.variableHelper")}</Button>}<Button type="button" variant="outline" size="sm" aria-pressed={workflowSettings} onClick={() => { setSelection(undefined); setWorkflowSettings(true); }}>{workflowText("actions.workflowSettings")}</Button></div></div>}
    {tab === "yaml" && missingHosts.length > 0 && <div role="alert" className="flex flex-wrap items-center gap-2 text-sm text-destructive">{executionText("editor.missingHosts")}{missingHosts.map(row => <Button key={row.id} type="button" variant="outline" size="sm" onClick={() => { setSelection({ job: row.id }); setTab("visual"); }}>{row.name}</Button>)}</div>}
    {tab === "yaml" ? <Suspense fallback={<LoadingSkeleton variant="editor" label={executionText("editor.loadingYaml")} className="h-[520px]" />}><YamlEditor value={source} originalValue={originalSource.current} readOnly={readOnly} fillHeight={fillHeight} onChange={definition => {
      try { parseActions(definition); } catch { onChange({ definition }); return; }
      change(() => definition);
    }} /></Suspense> : <div className={cn("flex min-h-[520px] flex-col overflow-hidden rounded-lg border lg:flex-row", fillHeight ? "flex-1 lg:min-h-0" : "lg:h-[620px]")}>
      <ContextMenu.Root><ContextMenu.Trigger asChild disabled={readOnly}><div className="relative h-[520px] min-w-0 shrink-0 lg:h-full lg:flex-1 lg:shrink" aria-label={executionText("editor.canvas")} onContextMenuCapture={event => {
        const target=event.target as HTMLElement, card=target.closest<HTMLElement>(".react-flow__node");
        if(card?.dataset.id){const index=target.closest<HTMLElement>("[data-action-step]")?.dataset.actionStep;select({job:card.dataset.id,...(index===undefined?{}:{step:Number(index)})});}
      }}>
        {selectedEdge && !readOnly && <div className="absolute right-3 top-3 z-10 flex max-w-[calc(100%-1.5rem)] items-center gap-2 rounded-md border bg-card pl-3 pr-1 shadow-sm"><span className="truncate text-xs">{parsed.jobs.find(row => row.id === selectedEdge.source)?.name ?? selectedEdge.source} → {parsed.jobs.find(row => row.id === selectedEdge.target)?.name ?? selectedEdge.target}</span><Button type="button" variant="ghost" size="icon" className="size-8 shrink-0" aria-label={executionText("editor.deleteDependency")} title={executionText("editor.deleteDependency")} onClick={() => {
      const edge = selectedEdge;
      setPendingDelete({ action: "editor.deleteDependency", target: `${parsed.jobs.find(row => row.id === edge.source)?.name ?? edge.source} → ${parsed.jobs.find(row => row.id === edge.target)?.name ?? edge.target}`, run: () => {
        change(() => editActionJob(source, edge.target, { needs: parsed.jobs.find(row => row.id === edge.target)!.needs.filter(id => id !== edge.source) }));
        setSelectedEdge(undefined);
      } });
    }}><Unlink2 className="size-4" /></Button></div>}
        {!parsed.error && !parsed.jobs.length && <EmptyWorkflowCanvas title={workflowText("actions.emptyTitle")} description={workflowText("actions.emptyHelp")} action={workflowText("addBuildJob")} disabled={readOnly} onAdd={() => { const added=addActionJob(source); change(() => added.source); select({job:added.id}); }} />}
        {!parsed.error && <ReactFlow nodes={nodes} edges={edges} nodeTypes={nodeTypes} edgeTypes={edgeTypes} nodesDraggable={!readOnly} nodesConnectable={!readOnly} edgesFocusable={!readOnly} deleteKeyCode={null} onConnect={onConnect} onEdgeClick={(_, edge) => setSelectedEdge({ source: edge.source, target: edge.target })} onPaneClick={() => setSelectedEdge(undefined)} onNodesChange={changes => {
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
        }} fitView minZoom={0.25} maxZoom={1.5} ariaLabelConfig={{ "controls.zoomIn.ariaLabel": executionText("editor.zoomIn"), "controls.zoomOut.ariaLabel": executionText("editor.zoomOut"), "controls.fitView.ariaLabel": executionText("editor.fitCanvas") }}><FitCanvas topology={parsed.jobs.map(job => `${job.id}:${job.needs.join(",")}:${job.steps.length}`).join(";")} /><Background /><Controls showInteractive={false} /></ReactFlow>}
      </div></ContextMenu.Trigger>{!readOnly && <ContextMenu.Portal><ContextMenu.Content className="z-50 min-w-48 rounded-md border bg-popover p-1 text-popover-foreground shadow-md" collisionPadding={8}>
        <ContextMenu.Item disabled={!job} className="rounded px-3 py-2 text-sm outline-none focus:bg-accent data-[disabled]:opacity-50" onSelect={()=>panel.current?.querySelector<HTMLInputElement>("input")?.focus()}>{workflowText("parameters")}</ContextMenu.Item>
        <ContextMenu.Item disabled={!job} className="rounded px-3 py-2 text-sm outline-none focus:bg-accent data-[disabled]:opacity-50" onSelect={duplicate}>{workflowText("duplicateNode")} · Ctrl / ⌘ D</ContextMenu.Item>
        <ContextMenu.Item disabled={!undo.current.length} className="rounded px-3 py-2 text-sm outline-none focus:bg-accent data-[disabled]:opacity-50" onSelect={()=>history(false)}>{workflowText("undo")} · Ctrl / ⌘ Z</ContextMenu.Item>
        <ContextMenu.Item disabled={!redo.current.length} className="rounded px-3 py-2 text-sm outline-none focus:bg-accent data-[disabled]:opacity-50" onSelect={()=>history(true)}>{workflowText("redo")} · Ctrl / ⌘ Shift Z</ContextMenu.Item>
        <ContextMenu.Separator className="my-1 h-px bg-border" />
        <ContextMenu.Item disabled={!job || (step ? job.steps.length <= 1 : parsed.jobs.length <= 1)} className="rounded px-3 py-2 text-sm text-destructive outline-none focus:bg-accent data-[disabled]:opacity-50" onSelect={deleteSelection}>{workflowText("delete")} · Del / ⌫</ContextMenu.Item>
      </ContextMenu.Content></ContextMenu.Portal>}</ContextMenu.Root>
      {workflowSettings && !parsed.error && <aside ref={panel} aria-label={workflowText("actions.workflowSettings")} className={cn("w-full max-h-[620px] space-y-4 overflow-y-auto border-t bg-background p-4 lg:w-80 lg:shrink-0 lg:border-l lg:border-t-0", fillHeight && "lg:max-h-none")}>
        <div className="flex items-center justify-between gap-2"><h3 className="font-semibold">{workflowText("actions.workflowSettings")}</h3><Button type="button" variant="ghost" size="icon" aria-label={executionText("editor.closeSettings")} onClick={() => setWorkflowSettings(false)}><X className="size-4" /></Button></div>
        <ActionsWorkflowSettings value={parsed.document!.toJS()} readOnly={readOnly} update={values => change(() => editActionWorkflow(source, values))} />
      </aside>}
      {job && <aside ref={panel} aria-label={step ? executionText("editor.stepSettings") : executionText("editor.jobSettings")} className={cn("w-full space-y-4 overflow-y-auto border-t bg-background p-4 max-h-[620px] lg:w-80 lg:shrink-0 lg:border-l lg:border-t-0", fillHeight && "lg:max-h-none")}>
        <div className="flex items-center justify-between gap-2"><h3 className="min-w-0 font-semibold">{step ? executionText("editor.stepSettings") : executionText("editor.jobSettings")}</h3><div className="flex shrink-0 items-center gap-1">{!readOnly && step && selection?.step !== undefined && <Button type="button" variant="destructive" size="icon" aria-label={executionText("editor.deleteStepName", { name: step.name || step.uses || selection.step + 1 })} title={executionText("editor.deleteStep")} onClick={() => {
          const jobId = job.id, index = selection.step!;
          setPendingDelete({ action: "editor.deleteStep", target: `${job.name} / ${step.name || step.uses || index + 1}`, run: () => {
            change(() => removeActionStep(source, jobId, index));
            setSelection(undefined);
          } });
        }}><Trash2 className="size-4" /></Button>}<Button type="button" variant="ghost" size="icon" aria-label={executionText("editor.closeSettings")} onClick={() => setSelection(undefined)}><X className="size-4" /></Button></div></div>
        {!step && <AutoField label={<span className="text-sm">{executionText("shared.common.common.name")}</span>} className="block space-y-2"><Input aria-label={executionText("editor.jobName")} value={String(job.raw.name ?? "")} disabled={readOnly} onChange={e => update({ name: e.target.value })} /></AutoField>}
        {!step ? <>
          <ActionsPhase phase="before">
          <fieldset className="space-y-2"><legend className="text-sm"><FieldCaption label={executionText("editor.dependencies")} /></legend>{parsed.jobs.filter(row => row.id !== job.id).map(row => <label key={row.id} className="flex min-h-10 items-center gap-2 text-sm"><input type="checkbox" checked={job.needs.includes(row.id)} disabled={readOnly} onChange={e => update({ needs: e.target.checked ? [...job.needs, row.id] : job.needs.filter(id => id !== row.id) })} />{row.name}</label>)}<p className="text-xs text-muted-foreground">{executionText("editor.dependenciesHelp")}</p></fieldset>
          {job.raw["ferry-approval"] !== true && <ActionsConditionField value={job.raw.if} readOnly={readOnly} job change={condition => update({ if: condition })} />}
          {job.raw.uses === undefined && job.raw["ferry-approval"] !== true && <StepMap label={workflowText("environmentVariables")} value={job.raw.env} readOnly={readOnly} change={env => update({ env })} />}
          {job.raw.uses === undefined && job.raw["ferry-approval"] !== true && <ActionsJobSettings value={job.raw} readOnly={readOnly} update={update} />}
          {job.raw.uses === undefined && job.raw["ferry-approval"] !== true && renderCheckoutBatch?.(job.id)}
          </ActionsPhase>
          <ActionsPhase phase="run">
          {job.raw.uses !== undefined && job.raw["ferry-approval"] !== true && <ActionsJobSettings value={job.raw} readOnly={readOnly} update={update} />}
          <AutoField label={<span className="text-sm">{executionText("editor.jobType")}</span>} className="block space-y-2"><select aria-label={executionText("editor.jobType")} className={inputClass} disabled={readOnly || job.raw.uses !== undefined} value={job.raw.uses !== undefined ? "reusable" : job.raw["ferry-approval"] === true ? "approval" : "actions"} onChange={e => {
            const manual = e.target.value === "approval";
            if (manual) setPendingApproval(job.id);
            else change(() => setActionJobApproval(source, job.id, false));
          }}><option value="actions">{executionText("editor.executionJob")}</option><option value="approval">{executionText("editor.manualGate")}</option>{job.raw.uses !== undefined && <option value="reusable">{workflowText("actions.reusableWorkflow")}</option>}</select></AutoField>
          {job.raw["ferry-approval"] !== true && <><p className="text-xs text-muted-foreground">{executionText("editor.jobHelp", { id: job.id })}</p>
          <AutoField label={<span className="text-sm">{executionText("editor.targetType")}</span>} className="block space-y-2"><select aria-label={executionText("editor.targetType")} className={inputClass} disabled={readOnly} value={targetKind} onChange={e => updateBinding({ target_kind: e.target.value, target_ref: "" })}><option value="host">{executionText("shared.common.nav.hosts")}</option><option value="host-group">{executionText("shared.common.nav.hostGroups")}</option><option value="pool">{executionText("editor.sharedPool")}</option></select></AutoField>
          {targetKind !== "pool" && <AutoField label={<span className="text-sm">{targetKind === "host" ? executionText("editor.executionHost") : executionText("editor.executionHostGroup")}</span>} className="block space-y-2"><select aria-label={executionText("editor.executor")} aria-invalid={!binding?.target_ref} className={cn(inputClass, !binding?.target_ref && "border-destructive")} disabled={readOnly} value={binding?.target_kind === targetKind ? binding.target_ref : ""} onChange={e => updateBinding({ target_kind: targetKind, target_ref: e.target.value })}><option value="" disabled>{executionText("editor.selectTarget", { kind: executionText(targetKind === "host" ? "shared.common.nav.hosts" : "shared.common.nav.hostGroups") })}</option>{targetOptions.map(option => <option key={option.value} value={option.value.slice(targetKind.length + 1)}>{option.label.startsWith(targetPrefix) ? option.label.slice(targetPrefix.length) : option.label.replace(/^(主机|主机组)\s*[·路]\s*/, "")}</option>)}</select>{!binding?.target_ref && <span className="text-xs text-destructive">{executionText("editor.targetRequired")}</span>}</AutoField>}
          {targetKind === "pool" && <p className="text-xs text-muted-foreground">{executionText("editor.sharedPoolHelp")}</p>}
          <AutoField label={<span className="text-sm">{executionText("editor.runMode")}</span>} className="block space-y-2"><select aria-label={executionText("editor.jobRunMode")} className={inputClass} disabled={readOnly} value={binding?.image && binding.image !== "host" ? "container" : "host"} onChange={e => updateBinding({ image: e.target.value === "host" ? "host" : "node:20-bookworm" })}><option value="host">{executionText("editor.runOnHost")}</option><option value="container">{executionText("editor.runInDocker")}</option></select></AutoField>
          {binding?.image && binding.image !== "host" && <AutoField label={<span className="text-sm">{executionText("editor.executionImage")}</span>} className="block space-y-2"><Input aria-label={executionText("editor.executionImage")} value={binding.image} disabled={readOnly} onChange={e => updateBinding({ image: e.target.value })} /></AutoField>}
          {job.raw.uses === undefined && <ActionsRunnerFields value={job.raw["runs-on"]} readOnly={readOnly} change={value => update({ "runs-on": value })} />}
          </>}
          {job.raw["ferry-approval"] !== true && <AutoField label={<span className="text-sm">{executionText("editor.timeout")}</span>} className="block space-y-2"><Input aria-label={executionText("editor.timeout")} type="text" inputMode="numeric" placeholder={executionText("editor.defaultTimeout")} value={String(job.raw["timeout-minutes"] ?? "")} disabled={readOnly} onChange={e => update({ "timeout-minutes": e.target.value ? (/^\d+$/.test(e.target.value) ? Number(e.target.value) : e.target.value) : null })} /></AutoField>}
          </ActionsPhase>
          {job.raw["ferry-approval"] !== true && <ActionsPhase phase="after"><StepMap output label={workflowText("actions.jobOutputs")} value={job.raw.outputs} readOnly={readOnly} change={outputs => update({ outputs })} /></ActionsPhase>}
        </> : <>
          <WorkflowFragmentEditor key={`${job.id}:${selection!.step}`} source={source} job={job.id} index={selection!.step!} original={originalSource.current} readOnly={readOnly} checkoutBound={checkoutBindings.some(binding => binding.job_id === job.id && binding.step_id === step.id)} checkout={/^actions\/checkout@/.test(String(step.uses ?? "")) && selection?.step !== undefined ? renderCheckout?.(job.id, selection.step) : undefined} onChange={definition => change(() => definition)} />
        </>}
        {!step && <ActionFragmentYaml key={job.id} source={source} job={job.id} original={originalSource.current} readOnly={readOnly} onChange={definition => change(() => definition)} />}
      </aside>}
    </div>}
    {(parsed.error || error || !parsed.jobs.length) && <p role="alert" className="whitespace-pre-wrap text-sm text-destructive">{parsed.error || error || executionText("workflow.jobsRequired")}</p>}
  </div></FieldHelpContext.Provider>;
}
