import { useEffect, useRef, useState } from "react";
import { api, errorText } from "../../shared/api/api";
import { commonText, workflowText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { LoadingSkeleton } from "../../shared/ui/loading-skeleton";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "../../shared/ui/select";
import { parseGraph, type GraphNode } from "../workflows/graph";
import { validRunInputs, WorkflowRunInputs } from "../workflows/WorkflowNodeParameters";
type ScheduleInputs = { target_ref?: string; entry_node_id?: string; inputs?: Record<string, unknown> };
type Props = { workspace: string; draft: ScheduleInputs; readOnly: boolean; onPatch: (patch: ScheduleInputs) => void; onValidityChange: (blocked: boolean) => void };

export function WorkflowScheduleInputs(props: Props) {
  const { workspace, draft, readOnly, onValidityChange } = props;
  const current = useRef(props);
  current.current = props;
  const [entries, setEntries] = useState<GraphNode[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [reload, setReload] = useState(0);
  const [invalid, setInvalid] = useState(false);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true); setError(""); setEntries([]); setInvalid(false);
    void (async () => {
      const workflow = await api<{ published_revision_id: string | null; lifecycle: string }>(`/workspaces/${workspace}/workflows/${draft.target_ref}`, { signal: controller.signal });
      if (!workflow.published_revision_id || workflow.lifecycle !== "active") throw new Error(commonText("resource-domain:cron.workflowPublishedRequired"));
      const revision = await api<{ definition: string }>(`/workspaces/${workspace}/workflows/${draft.target_ref}/revisions/${workflow.published_revision_id}`, { signal: controller.signal });
      if (controller.signal.aborted) return;
      const nodes = parseGraph(revision.definition).nodes.filter(node => node.type === "core.trigger.manual");
      setEntries(nodes);
      if (!current.current.readOnly && !current.current.draft.entry_node_id && nodes.length === 1) current.current.onPatch({ entry_node_id: nodes[0]!.id });
    })().catch(error => { if (!controller.signal.aborted) setError(errorText(error)); })
      .finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [workspace, draft.target_ref, reload]);
  const entry = entries.find(node => node.id === draft.entry_node_id);
  const schema = (entry?.params.input_schema ?? { type: "object" }) as Record<string, unknown>;
  const inputs = draft.inputs ?? {};
  const blocked = loading || !!error || !entry || invalid || !validRunInputs(schema, inputs);
  useEffect(() => { onValidityChange(!readOnly && blocked); }, [blocked, readOnly, onValidityChange]);
  useEffect(() => () => onValidityChange(false), [onValidityChange]);
  if (loading) return <LoadingSkeleton variant="form" />;
  if (readOnly) return <div className="min-w-0 space-y-3">{error && <p role="alert" className="break-words text-sm text-destructive">{error}</p>}<p className="break-words text-sm">{workflowText("entry")}: {entry?.name ?? draft.entry_node_id}</p><pre className="max-h-64 overflow-auto whitespace-pre-wrap break-words rounded-md bg-muted p-3 text-xs">{JSON.stringify(inputs, null, 2)}</pre></div>;
  if (error) return <div role="alert" className="space-y-2"><p className="break-words text-sm text-destructive">{error}</p><Button type="button" variant="outline" onClick={() => setReload(value => value + 1)}>{commonText("common.retry")}</Button></div>;
  return <div className="min-w-0 space-y-4">
    <p className="text-sm text-muted-foreground">{commonText("resource-domain:cron.workflowHelp")}</p>
    <Select value={draft.entry_node_id ?? ""} onValueChange={value => { setInvalid(false); props.onPatch({ entry_node_id: value, inputs: {} }); }}>
      <SelectTrigger aria-label={workflowText("entry")} className="min-h-11 w-full"><SelectValue placeholder={workflowText("entry")} /></SelectTrigger>
      <SelectContent>{entries.map(node => <SelectItem key={node.id} value={node.id}>{node.name || node.id}</SelectItem>)}</SelectContent>
    </Select>
    {!entries.length && <p role="alert" className="text-sm text-destructive">{commonText("resource-domain:cron.workflowEntryRequired")}</p>}
    {!!entries.length && !entry && <p role="alert" className="text-sm text-destructive">{commonText("resource-domain:cron.workflowEntryChoose")}</p>}
    {entry && <WorkflowRunInputs schema={schema} value={inputs} change={inputs => props.onPatch({ inputs })} invalid={setInvalid} />}
  </div>;
}
