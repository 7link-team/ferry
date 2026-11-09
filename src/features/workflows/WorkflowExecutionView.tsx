import { useEffect, useRef, useState } from "react";
import { Background, Controls, MarkerType, ReactFlow, type NodeTypes } from "@xyflow/react";
import { api, errorText } from "../../shared/api/api";
import { workflowText as t } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { connectionDirection, type Graph } from "./graph";
import { outputPortLabel } from "./WorkflowNodePicker";
import { WorkflowDataView } from "./WorkflowDataView";
import { WorkflowPreviewReview } from "./WorkflowPreviewReview";
import type { NodeExecutionData } from "./workflow-data";

export const executionStateText = (state: unknown) => {
  const value=String(state === "pending" ? "queued" : state || "not_started"), key=`execution.state.${value}`;
  const text=t(key as Parameters<typeof t>[0]);
  return text===key ? value : text;
};
const failed = (state: unknown) => state === "failed" || state === "cancelled";
const color = (state: unknown) => failed(state) ? "var(--destructive)" : state === "running" ? "var(--primary)" : ["succeeded", "cached", "pinned", "reused", "bypassed"].includes(String(state)) ? "#16a34a" : "var(--muted-foreground)";

/** Frozen execution graph and recorded node data; no parameter editing or reruns. */
export function WorkflowExecutionView({ workspace, execution, graph, attempts, nodeTypes, permissions = new Set() }: {
  workspace: string; execution: Record<string, unknown>; graph: Graph | undefined; attempts: Record<string, unknown>[]; nodeTypes: NodeTypes; permissions?: Set<string>;
}) {
  const [selected, setSelected] = useState("");
  const [selectedAttempt, setSelectedAttempt] = useState("");
  const [tab, setTab] = useState("process");
  const [data, setData] = useState<NodeExecutionData>();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const loadedNode = useRef("");
  const latest = new Map(attempts.map(row => [String(row.node_id), row]));
  const chosen = graph?.nodes.find(node => node.id === selected) ?? graph?.nodes.find(node => latest.get(node.id)?.state === "failed") ?? graph?.nodes.find(node => latest.get(node.id)?.state === "running") ?? graph?.nodes[0];
  const nodeAttempts = attempts.filter(row => row.node_id === chosen?.id);
  const attempt = nodeAttempts.find(row => row.id === selectedAttempt) ?? nodeAttempts.at(-1);
  useEffect(() => {
    const key = `${execution.id}:${attempt?.id ?? ""}`;
    if (loadedNode.current !== key) { setData(undefined); loadedNode.current = key; }
    setError(""); setLoading(false);
    if (!attempt?.id) return;
    const abort = new AbortController(); setLoading(true);
    void api<NodeExecutionData>(`/workspaces/${workspace}/workflow-executions/${execution.id}/node-runs/${attempt.id}/data`, {signal:abort.signal})
      .then(value => { if (!abort.signal.aborted) setData(value); })
      .catch(cause => { if (!abort.signal.aborted) setError(errorText(cause)); })
      .finally(() => { if (!abort.signal.aborted) setLoading(false); });
    return () => abort.abort();
  }, [workspace, execution.id, attempt?.id, attempt?.state, attempt?.logs_updated_at]);
  if (!graph) return <LoadingSkeleton variant="editor" />;
  const links = graph.connections;
  const nodes = graph.nodes.map(node => ({id:node.id,type:"workflow",position:{x:node.position[0],y:node.position[1]},selected:chosen?.id===node.id,
    data:{node,state:String(latest.get(node.id)?.state ?? "not_started"),
      bottomPorts:links.filter(link=>link.from.node===node.id&&connectionDirection(graph,link)==="down").map(link=>`${link.from.port}:${link.from.index}`),
      topPorts:links.filter(link=>link.to.node===node.id&&connectionDirection(graph,link)==="down").map(link=>`${link.to.port}:${link.to.index}`)}}));
  const edges = links.map((link,index)=>{
    const source=latest.get(link.from.node)?.state, target=latest.get(link.to.node)?.state;
    const down=connectionDirection(graph,link)==="down";
    const stroke=color(target === "not_started" || !target ? source : target);
    return {id:`execution-link-${index}`,source:link.from.node,target:link.to.node,
      sourceHandle:`${link.from.port}:${link.from.index}${down?":bottom":""}`,targetHandle:`${link.to.port}:${link.to.index}${down?":top":""}`,
      ...(graph?.nodes.some(node=>node.id===link.from.node && ["core.if","core.switch","host.preview"].includes(node.type)) ? {label:outputPortLabel(graph.nodes.find(node=>node.id===link.from.node)!,link.from),labelStyle:{fill:"var(--foreground)"},labelBgStyle:{fill:"var(--background)"}} : {}),
      animated:execution.state === "running" && (target === "running" || source === "running"),
      markerEnd:{type:MarkerType.ArrowClosed,color:stroke},style:{stroke,strokeWidth:2}};
  });
  const logs = data?.output?.ports.flatMap(port=>port.items.flatMap(item=>["stdout","stderr"].flatMap(key=>typeof item.json[key]==="string" ? [{key,text:String(item.json[key])}] : []))) ?? [];
  return <div data-workflow-execution-view className="space-y-3">
    <p className="text-xs text-muted-foreground">{t("execution.viewerHelp")}</p>
    <div className="grid min-w-0 gap-3 lg:grid-cols-[minmax(0,3fr)_minmax(18rem,2fr)]">
      <div className="h-[380px] min-w-0 overflow-hidden rounded-lg border bg-muted/10 motion-reduce:[&_.react-flow__edge-path]:!animate-none lg:h-[520px]" aria-label={t("execution.graph")}>
        <ReactFlow key={String(execution.id)} nodes={nodes} edges={edges} nodeTypes={nodeTypes} nodesDraggable={false} nodesConnectable={false} deleteKeyCode={null} fitView minZoom={0.2} maxZoom={1.5} onNodeClick={(_,node)=>setSelected(node.id)}><Background /><Controls showInteractive={false} /></ReactFlow>
      </div>
      <section className="min-w-0 space-y-3 rounded-lg border p-3" aria-label={t("execution.nodeDetails")}>
        <label className="block space-y-1 text-sm"><span>{t("execution.nodeDetails")}</span><select className="h-9 w-full rounded-md border bg-background px-2" aria-label={t("execution.nodeDetails")} value={chosen?.id ?? ""} onChange={event=>setSelected(event.target.value)}>{graph.nodes.map(node=><option key={node.id} value={node.id}>{node.name} · {executionStateText(latest.get(node.id)?.state)}</option>)}</select></label>
        <p role="status" className={`text-sm font-medium ${failed(attempt?.state)?"text-destructive":""}`}>{executionStateText(attempt?.state)}</p>
        {nodeAttempts.length>1 && <label className="block space-y-1 text-xs"><span>{t("attempt")}</span><select className="h-8 w-full rounded border bg-background px-2" value={String(attempt?.id??"")} onChange={event=>setSelectedAttempt(event.target.value)}>{nodeAttempts.map((row,index)=><option key={String(row.id)} value={String(row.id)}>{index+1} · {t("attempt")} {String(row.attempt)} · {executionStateText(row.state)}</option>)}</select></label>}
        {chosen?.type === "host.preview" && attempt?.id && permissions.has("holes.read") ? <WorkflowPreviewReview workspace={workspace} execution={String(execution.id)} node={chosen.id} canReview={permissions.has("workflows.run")} refresh={`${String(attempt.state)}:${String(attempt.version)}`} /> : null}
        <div role="tablist" aria-label={t("execution.nodeDetails")} className="flex gap-1 border-b pb-2">{(["process","input","output"] as const).map(key=><Button key={key} type="button" variant={tab===key?"secondary":"ghost"} size="sm" role="tab" aria-selected={tab===key} onClick={()=>setTab(key)}>{t(`execution.${key}`)}</Button>)}</div>
        <div className="max-h-[380px] space-y-3 overflow-y-auto break-words" role="tabpanel">
          {tab === "process" ? <>
            <dl className="grid grid-cols-2 gap-2 text-xs"><dt>{t("execution.startedAt")}</dt><dd>{Number(attempt?.started_at)>0?new Date(Number(attempt?.started_at)).toLocaleString():"—"}</dd><dt>{t("execution.finishedAt")}</dt><dd>{Number(attempt?.finished_at)>0?new Date(Number(attempt?.finished_at)).toLocaleString():"—"}</dd><dt>{t("execution.duration")}</dt><dd>{Number(attempt?.started_at)>0?`${Math.max(0,(Number(attempt?.finished_at)||Date.now())-Number(attempt?.started_at))} ms`:"—"}</dd><dt>{t("itemsIn")}</dt><dd>{String(attempt?.items_in??0)}</dd><dt>{t("itemsOut")}</dt><dd>{String(attempt?.items_out??0)}</dd></dl>
            {attempt?.state === "failed" && execution.end_reason ? <p role="alert" className="text-sm text-destructive">{String(execution.end_reason)}</p>:null}
            {data?.logs ? <><pre data-node-execution-logs className="max-h-64 overflow-auto whitespace-pre-wrap rounded bg-muted p-2 font-mono text-xs">{data.logs}</pre>{data.logs_truncated && <p role="status" className="text-xs text-amber-700 dark:text-amber-400">{t("execution.logsTruncated")}</p>}</> : loading ? <LoadingSkeleton variant="form" /> : logs.length ? logs.map((log,index)=><section key={index}><h4 className="text-xs font-medium">{log.key}</h4><pre className="mt-1 max-h-60 overflow-auto whitespace-pre-wrap rounded bg-muted p-2 font-mono text-xs">{log.text.slice(0,200_000)}</pre></section>) : <p className="text-xs text-muted-foreground">{t("execution.noTextLogs")}</p>}
          </> : loading ? <LoadingSkeleton variant="form" /> : tab === "input" ? <WorkflowDataView items={data?.input.items??[]} /> : <>
            {(data?.output?.ports??[]).map(port=><section key={`${port.name}:${port.index}`} className="space-y-2"><h4 className="text-sm font-medium">{port.name==="main"?t("mainPort"):port.name}</h4><WorkflowDataView items={port.items} /></section>)}
            {!data?.output && <p className="text-sm text-muted-foreground">{t("inspector.noOutput")}</p>}
          </>}
          {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
        </div>
      </section>
    </div>
  </div>;
}
