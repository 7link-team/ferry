import { useState, type ReactNode } from "react";
import { AlertTriangle, ShieldQuestion, X } from "lucide-react";
import { Dialog } from "radix-ui";
import { commonText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { LONG_TEXT_LIMIT, LongText } from "./LazyDetails";
import { AutoField, AutoForm, FieldControl } from "../../shared/forms/auto-form";
import { api, errorText } from "../../shared/api/api";

const tv = (key: string, values?: Record<string, unknown>) => commonText(`conversations:${key}`, values);

export interface Decision {
  tool: string;
  input: unknown;
  decision: boolean | null;
  decided_by?: string;
  dispatching: boolean;
  sent: boolean;
  /** The turn ended before anyone decided; the operation never ran. */
  ended_undecided?: boolean;
  /** The owner continued or dismissed an ended, undecided request. */
  resolved?: "resumed" | "dismissed" | "answered" | "paused" | "abandoned";
  response?: { answer: string; action: string; option_id?: string | null };
  automatic?: boolean;
  approval_mode?: string;
}
export interface ActivityEvent {
  type: string;
  item_id?: string;
  name?: string;
  state?: string;
  request_id?: string;
  result?: unknown;
  input?: unknown;
  /** What a platform result answered: tool, method and path (never a body). */
  call?: { tool?: string; method?: string | null; path?: string | null };
  outcome?: string;
  usage?: { input_tokens: number; output_tokens: number } | null;
}
interface TurnLike {
  state: string;
  owner_id: string;
  decisions: Record<string, Decision>;
}

/** Waiting for someone now, or ended undecided and not yet continued/dismissed. */
export const awaitingDecision = (turn: TurnLike, decision: Decision) =>
  liveDecision(turn, decision) || endedDecision(turn, decision);
export const liveDecision = (turn: TurnLike, decision: Decision) => decision.decision === null && turn.state === "running";
export const endedDecision = (turn: TurnLike, decision: Decision) =>
  decision.decision === null && !["queued", "running", "stopping"].includes(turn.state) && !decision.resolved;

/** Suggested choices never replace free text or the ability to stop a task. */
export function QuestionCard({ request, decision, canAnswer, busy, executing, onAnswer }: {
  request: string; decision: Decision; canAnswer: boolean; busy: boolean; executing: boolean;
  onAnswer: (response: { action: "continue" | "pause" | "abandon"; answer: string; option_id?: string }) => Promise<void>;
}) {
  const question = record(decision.input) ?? {};
  const options = Array.isArray(question.options) ? question.options.flatMap(item => {
    const value = record(item);
    return value && typeof value.id === "string" && typeof value.label === "string" ? [value] : [];
  }) : [];
  const [text, setText] = useState(decision.response?.answer ?? "");
  const [selected, setSelected] = useState(decision.response?.option_id ?? "");
  const [sending, setSending] = useState(false);
  const [error, setError] = useState("");
  const resolved = decision.resolved;
  const settled = resolved === "answered" || resolved === "abandoned";
  const disabled = busy || sending;
  const submit = async (action: "continue" | "pause" | "abandon") => {
    setSending(true); setError("");
    try { await onAnswer({ action, answer: text, ...(selected ? { option_id: selected } : {}) }); }
    catch (cause) { setError(errorText(cause)); }
    finally { setSending(false); }
  };
  return <section data-user-question={request} className="min-w-0 space-y-3 rounded-xl border border-primary/40 bg-card p-3 text-sm [overflow-wrap:anywhere] sm:p-4">
    <div className="flex items-start gap-2"><ShieldQuestion aria-hidden className="mt-0.5 size-4 shrink-0 text-primary" /><div className="min-w-0 flex-1"><p className="text-xs font-medium text-primary">{tv(resolved === "answered" ? "questionAnswered" : resolved === "abandoned" ? "questionAbandoned" : resolved === "paused" ? "questionPaused" : "questionWaiting")}</p><h3 className="mt-1 break-words font-semibold">{String(question.title ?? "")}</h3></div></div>
    <p className="whitespace-pre-wrap break-words">{String(question.question ?? "")}</p>
    {typeof question.reason === "string" && question.reason && <p className="whitespace-pre-wrap break-words text-muted-foreground">{String(question.reason)}</p>}
    {settled ? <p className="whitespace-pre-wrap break-words">{decision.response?.answer || options.find(option => option.id === decision.response?.option_id)?.label as string || tv(resolved === "abandoned" ? "questionAbandoned" : "questionAnswered")}</p> : canAnswer ? <AutoForm className="space-y-3" onSubmit={event => { event.preventDefault(); void submit("continue"); }}>
      {options.length > 0 && <div className="grid gap-2" role="group" aria-label={tv("questionSuggestions")}>{options.map(option => <button key={String(option.id)} type="button" disabled={disabled} aria-pressed={selected === option.id} className={`min-h-11 min-w-0 max-w-full whitespace-normal rounded-lg border p-3 text-left [overflow-wrap:anywhere] focus-visible:outline-2 focus-visible:outline-ring ${selected === option.id ? "border-primary bg-primary/10" : "border-border hover:bg-muted"}`} onClick={() => setSelected(selected === option.id ? "" : String(option.id))}><span className="flex flex-wrap items-center gap-2 font-medium"><span>{String(option.label)}</span>{question.recommended_option_id === option.id && <span className="text-xs text-primary">{tv("questionRecommended")}</span>}</span>{typeof option.description === "string" && option.description && <span className="mt-1 block whitespace-pre-wrap break-words text-xs text-muted-foreground">{String(option.description)}</span>}</button>)}</div>}
      <AutoField name={`question-${request}`} label={tv("questionResponse")} help={tv("questionResponseHelp")}><FieldControl name={`question-${request}`} field={{name:tv("questionResponse"),type:"textarea",config:{maxLength:4096}}} value={text} onChange={value=>setText(String(value))} disabled={disabled} /></AutoField>
      {executing && <p role="status" className="text-xs text-muted-foreground">{tv("questionFinishing")}</p>}
      {error && <p role="alert" className="break-words text-destructive">{error}</p>}
      <div className="grid grid-cols-[repeat(auto-fit,minmax(min(100%,8rem),1fr))] gap-2 [&>button]:h-auto [&>button]:min-h-11 [&>button]:min-w-0 [&>button]:whitespace-normal"><Button type="submit" disabled={disabled || executing || !text.trim() && !selected}>{tv("questionContinue")}</Button><Button type="button" variant="outline" disabled={disabled} onClick={()=>void submit("pause")}>{tv("questionPause")}</Button><Button type="button" variant="outline" disabled={disabled} onClick={()=>void submit("abandon")}>{tv("questionAbandon")}</Button></div>
    </AutoForm> : <p className="text-xs text-muted-foreground">{tv("questionOwnerOnly")}</p>}
  </section>;
}

export function ConversationApproval({ endpoint, mode, version, canManage, executing, onSaved, showLabel = false }: {
  endpoint: string; mode: string; version: number; canManage: boolean; executing: boolean;
  onSaved: (mode: string, version: number) => void; showLabel?: boolean;
}) {
  const [open,setOpen]=useState(false);const [draft,setDraft]=useState(mode);const [revision,setRevision]=useState(version);
  const [busy,setBusy]=useState(false);const [error,setError]=useState("");
  const labels = {request:"approvalRequest",assisted:"approvalAssisted",yolo:"approvalYolo"} as const;
  const label=(value:string)=>tv(labels[value as keyof typeof labels]??"approvalRequest");
  const save=async()=>{setBusy(true);setError("");try { const saved=await api<{version:number}>(endpoint+"/approval-mode",{method:"PUT",body:JSON.stringify({mode:draft,version:revision})});onSaved(draft,saved.version);setOpen(false); } catch(cause){setError(errorText(cause));}finally{setBusy(false);}};
  if(!canManage) return <span className="inline-flex min-h-11 items-center px-2 text-xs" title={tv("approvalMode")}>{label(mode)}</span>;
  return <Dialog.Root open={open} onOpenChange={value=>{if(busy)return;setOpen(value);if(value){setDraft(mode);setRevision(version);setError("");}}}><Dialog.Trigger asChild><Button type="button" variant="ghost" className="min-h-11 min-w-11 px-2 text-xs" disabled={executing} aria-label={`${tv("approvalMode")} · ${label(mode)}`}><ShieldQuestion className="size-4"/><span className={showLabel ? "" : "hidden sm:inline"}>{label(mode)}</span></Button></Dialog.Trigger><Dialog.Portal><Dialog.Overlay className="fixed inset-0 z-50 bg-black/35"/><Dialog.Content aria-describedby={undefined} className="fixed left-1/2 top-1/2 z-50 flex max-h-[90dvh] w-[calc(100%-2rem)] max-w-lg -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-xl border bg-background text-foreground shadow-xl"><header className="flex shrink-0 items-center gap-2 border-b p-4"><Dialog.Title className="min-w-0 flex-1 font-semibold">{tv("approvalMode")}</Dialog.Title><Dialog.Close asChild><Button variant="ghost" size="icon" className="min-h-11 min-w-11" aria-label={commonText("common.close")}><X/></Button></Dialog.Close></header><div className="min-h-0 space-y-3 overflow-y-auto p-4"><AutoField name="approval-mode" label={tv("approvalMode")} required><FieldControl name="approval-mode" field={{name:tv("approvalMode"),type:"select",config:{options:Object.keys(labels).map(value=>({value,label:label(value)}))}}} value={draft} required onChange={value=>setDraft(String(value))} /></AutoField><p className="text-sm text-muted-foreground">{tv("approvalScopeHelp")}</p><p className={`rounded-lg border p-3 text-sm ${draft==="yolo"?"border-destructive/50 bg-destructive/10 text-destructive":"border-border bg-muted/30"}`}>{tv(draft==="yolo"?"approvalYoloHelp":draft==="assisted"?"approvalAssistedHelp":"approvalRequestHelp")}</p>{error&&<p role="alert" className="text-sm text-destructive">{error}</p>}</div><footer className="grid shrink-0 grid-cols-2 gap-2 border-t p-4 pb-[max(1rem,env(safe-area-inset-bottom))]"><Dialog.Close asChild><Button variant="outline" className="min-h-11" disabled={busy}>{commonText("common.cancel")}</Button></Dialog.Close><Button className="min-h-11" disabled={busy} onClick={()=>void save()}>{commonText("common.save")}</Button></footer></Dialog.Content></Dialog.Portal></Dialog.Root>;
}

/** A failed platform request (HTTP status ≥ 400 or ok: false). */
export function platformFailed(result: unknown) {
  if (!result || typeof result !== "object") return false;
  const value = result as { ok?: unknown; status?: unknown };
  return value.ok === false || (typeof value.status === "number" && value.status >= 400);
}

const RESOURCES: Record<string, string> = {
  workflows: "resourceWorkflows",
  "cron-jobs": "resourceCronJobs",
  "notification-channels": "resourceNotificationChannels",
  pipelines: "resourcePipelines",
};
const VERBS: Record<string, string> = {
  GET: "operationRead",
  POST: "operationCreate",
  PUT: "operationUpdate",
  PATCH: "operationUpdate",
  DELETE: "operationDelete",
};
function parsed(value: unknown): unknown {
  if (typeof value !== "string") return value;
  try {
    return JSON.parse(value);
  } catch {
    return value;
  }
}
const record = (value: unknown) =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : undefined;

/** One line saying what a request does, in words: "创建工作流 /workflows". */
export function operationTitle(tool: string, method?: string | null, path?: string | null) {
  if (tool === "ferry_discover") return tv("operationDiscover");
  if (tool === "ferry_ask_user") return tv("questionWaiting");
  if (!path) return tool;
  const clean = path.split("?")[0] ?? path;
  const segments = clean.split("/").filter(Boolean);
  // The resource is the last collection name: /workflows/12 → workflows.
  const name = [...segments].reverse().find((segment) => RESOURCES[segment]) ?? segments[0] ?? "";
  const resource = RESOURCES[name] ? tv(RESOURCES[name]) : "";
  const verb = VERBS[(method ?? "GET").toUpperCase()];
  if (!verb || !resource) return `${(method ?? "GET").toUpperCase()} ${clean}`;
  return `${tv(verb, { resource })} · ${(method ?? "GET").toUpperCase()} ${clean}`;
}

/** A readable summary of a decision's input: title plus the key body fields. */
export function describeOperation(tool: string, input: unknown) {
  const value = record(parsed(input));
  if (!value || typeof value.path !== "string") {
    const command = value?.command;
    return {
      title: typeof command === "string" ? `${tool}: ${command}` : Array.isArray(command) ? `${tool}: ${command.join(" ")}` : tool,
      fields: Object.entries(value ?? {}).filter(([key]) => key !== "command").slice(0, 8).map(([key, item]): [string, string] => [key, typeof item === "string" ? item.slice(0, 240) : item && typeof item === "object" ? Object.keys(item).slice(0, 8).join(", ") : String(item)]),
    };
  }
  const method = typeof value.method === "string" ? value.method : "GET";
  const fields: [string, string][] = [];
  const body = record(parsed(value.body));
  if (body) {
    if (typeof body.name === "string" && body.name) fields.push([tv("fieldName"), body.name]);
    if (typeof body.description === "string" && body.description) fields.push([tv("fieldDescription"), body.description]);
    const definition = record(parsed(body.definition));
    const nodes = Array.isArray(definition?.nodes) ? (definition!.nodes as unknown[]) : undefined;
    if (nodes) {
      const types = [...new Set(nodes.map((node) => record(node)?.type).filter((type): type is string => typeof type === "string"))];
      fields.push([tv("fieldNodes"), `${nodes.length}${types.length ? ` · ${types.join(", ")}` : ""}`]);
    }
    const schedule = body.schedule ?? body.cron ?? body.expression;
    if (typeof schedule === "string" && schedule) fields.push([tv("fieldSchedule"), schedule]);
    const summarized = new Set(["name", "description", "definition", "schedule", "cron", "expression", "version"]);
    for (const [key, item] of Object.entries(body).filter(([key]) => !summarized.has(key)).slice(0, 8)) {
      if (typeof item === "string" || typeof item === "number" || typeof item === "boolean" || item === null)
        fields.push([key, String(item).slice(0, 240)]);
    }
    if (["PUT", "PATCH"].includes(method.toUpperCase())) {
      const keys = Object.keys(body).filter((key) => key !== "version");
      if (keys.length) fields.push([tv("fieldChanges"), keys.join(", ")]);
    }
  }
  return { title: operationTitle(tool, method, value.path), fields };
}

function Summary({ tool, input }: { tool: string; input: unknown }) {
  const { title, fields } = describeOperation(tool, input);
  return (
    <>
      <p data-operation-title className="break-words font-medium text-foreground">{title}</p>
      {!!fields.length && (
        <dl className="mt-1 grid grid-cols-[minmax(0,1fr)_minmax(0,2fr)] gap-x-3 gap-y-0.5 text-xs text-muted-foreground [overflow-wrap:anywhere]">
          {fields.map(([label, text]) => (
            <FieldRow key={label} label={label} text={text} />
          ))}
        </dl>
      )}
    </>
  );
}
function FieldRow({ label, text }: { label: string; text: string }) {
  return (
    <>
      <dt className="min-w-0">{label}</dt>
      <dd className="min-w-0 line-clamp-3 text-foreground/80">{text}</dd>
    </>
  );
}

function RawToggle({ value, label, children }: { value: unknown; label?: string; children?: ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <div className="flex min-w-0 flex-wrap items-start gap-3">
      {children && <div className="min-w-0 flex-1 basis-40 pt-2.5 [overflow-wrap:anywhere]">{children}</div>}
      <button type="button" data-raw-toggle aria-expanded={open} className="ml-auto inline-flex min-h-11 max-w-full shrink-0 items-center whitespace-normal text-xs text-muted-foreground underline-offset-2 [overflow-wrap:anywhere] hover:underline focus-visible:outline focus-visible:outline-ring" onClick={() => setOpen(!open)}>
        {open ? tv("hideRaw") : label ?? tv("rawData")}
      </button>
      </div>
      {open && <LongText value={value} className="mt-1 max-h-56 rounded bg-muted/60 p-2 text-xs" />}
    </div>
  );
}

/** A decision waiting now: prominent, in the conversation flow, never folded away. */
export function PendingDecisionCard({
  request,
  decision,
  canDecide,
  ownerName,
  busy,
  onDecide,
}: {
  request: string;
  decision: Decision;
  canDecide: boolean;
  ownerName: string;
  busy: boolean;
  onDecide: (allow: boolean) => void;
}) {
  return (
    <div data-pending-decision={request} className="space-y-2 rounded-lg border border-amber-500/60 bg-amber-50 p-3 text-sm shadow-sm dark:bg-amber-950/30">
      <p className="flex items-center gap-1.5 text-xs font-semibold text-amber-700 dark:text-amber-300">
        <ShieldQuestion className="size-3.5" aria-hidden />
        {tv("approvalNeeded")}
      </p>
      <RawToggle value={decision.input}><Summary tool={decision.tool} input={decision.input} /></RawToggle>
      {canDecide ? (
        <div className="flex flex-wrap gap-2 [&>button]:h-auto [&>button]:min-w-0 [&>button]:max-w-full [&>button]:whitespace-normal [&>button]:[overflow-wrap:anywhere]">
          <Button size="sm" className="min-h-11" disabled={busy || decision.dispatching} onClick={() => onDecide(true)}>
            {tv("allow")}
          </Button>
          <Button size="sm" className="min-h-11" variant="outline" disabled={busy || decision.dispatching} onClick={() => onDecide(false)}>
            {tv("deny")}
          </Button>
        </div>
      ) : (
        <p className="text-xs text-muted-foreground">{tv("waitingFor", { name: ownerName })}</p>
      )}
    </div>
  );
}

/** An approval nobody answered before the reply ended: it did not run. */
export function EndedDecisionCard({
  request,
  decision,
  canAct,
  busy,
  onContinue,
  continueDisabled = false,
  onDismiss,
}: {
  request: string;
  decision: Decision;
  canAct: boolean;
  busy: boolean;
  onContinue: (operation: string) => void;
  continueDisabled?: boolean;
  onDismiss?: () => Promise<void>;
}) {
  const { title } = describeOperation(decision.tool, decision.input);
  const [dismissing, setDismissing] = useState(false);
  const [error, setError] = useState("");
  const dismiss = async () => {
    setDismissing(true);
    setError("");
    try { await onDismiss?.(); }
    catch (cause) { setError(errorText(cause)); }
    finally { setDismissing(false); }
  };
  return (
    <div data-ended-decision={request} className="space-y-2 rounded-lg border border-dashed border-muted-foreground/40 bg-muted/40 p-3 text-sm">
      <p className="flex items-center gap-1.5 text-xs font-semibold text-muted-foreground">
        <AlertTriangle className="size-3.5" aria-hidden />
        {tv("endedUndecided")}
      </p>
      <RawToggle value={decision.input}><Summary tool={decision.tool} input={decision.input} /></RawToggle>
      {canAct && (
        <div className="flex flex-wrap gap-2">
          <Button size="sm" className="min-h-11" disabled={busy || dismissing || continueDisabled} onClick={() => {
            // Inputs here are the server's redacted viewer representation. Build
            // the visible follow-up only when requested, with the same raw cap.
            const input = JSON.stringify(decision.input, null, 2) ?? "";
            onContinue(`${title}\n${input.slice(0, LONG_TEXT_LIMIT)}${input.length > LONG_TEXT_LIMIT ? `\n${tv("truncatedNote")}` : ""}`);
          }}>
            {tv("continueTask")}
          </Button>
          {onDismiss && <Button type="button" size="sm" variant="outline" className="min-h-11" disabled={busy || dismissing} onClick={() => void dismiss()}>{tv("dismissOperation")}</Button>}
        </div>
      )}
      {error && <p role="alert" className="break-words text-destructive">{error}</p>}
    </div>
  );
}

const viewKey = (viewerId: string) => `conversation.details-view:${viewerId}`;
function storedView(viewerId: string): "readable" | "raw" {
  try {
    return window.localStorage.getItem(viewKey(viewerId)) === "raw" ? "raw" : "readable";
  } catch {
    return "readable";
  }
}
function storeView(viewerId: string, view: "readable" | "raw") {
  try {
    window.localStorage.setItem(viewKey(viewerId), view);
  } catch {
    // A private window or blocked storage only loses the preference.
  }
}

type Entry =
  | { kind: "tool"; key: string; name: string; state?: string; input?: unknown }
  | { kind: "platform"; key: string; event: ActivityEvent }
  | { kind: "decision"; key: string; decision: Decision };

/** Tool calls, platform requests and decisions of one turn, in arrival order. */
export function timelineEntries(events: readonly ActivityEvent[], decisions: Record<string, Decision>) {
  const entries = new Map<string, Entry>();
  for (const event of events) {
    if (event.type === "tool" && event.item_id) {
      const key = `tool:${event.item_id}`;
      const previous = entries.get(key);
      entries.set(key, { kind: "tool", key, name: event.name ?? (previous?.kind === "tool" ? previous.name : ""), ...(event.state ? { state: event.state } : {}), input: event.input ?? (previous?.kind === "tool" ? previous.input : undefined) });
    } else if (event.type === "approval" && event.request_id && decisions[event.request_id]) {
      const key = `decision:${event.request_id}`;
      if (!entries.has(key)) entries.set(key, { kind: "decision", key, decision: decisions[event.request_id]! });
    } else if (event.type === "platform_result" && event.request_id) {
      const key = `platform:${event.request_id}`;
      entries.set(key, { kind: "platform", key, event });
    }
  }
  // Decisions whose approval event is not loaded still belong to the turn.
  for (const [request, decision] of Object.entries(decisions)) {
    const key = `decision:${request}`;
    entries.set(key, { kind: "decision", key, decision });
  }
  return [...entries.values()];
}

function errorMessage(result: Record<string, unknown>) {
  if (typeof result.error === "string") return result.error;
  const data = record(result.data);
  const nested = record(data?.error);
  const text = data?.message ?? nested?.message ?? data?.error;
  return typeof text === "string" ? text : "";
}
function createdName(result: Record<string, unknown>) {
  const data = record(result.data);
  const name = data?.name ?? data?.title;
  if (typeof name === "string" && name) return data?.id !== undefined ? `${name} #${String(data.id)}` : name;
  return data?.id !== undefined && (typeof data.id === "string" || typeof data.id === "number") ? `#${data.id}` : "";
}

function decisionText(turnState: string, decision: Decision) {
  if(decision.tool==="ferry_ask_user") return tv(decision.resolved==="answered"?"questionAnswered":decision.resolved==="paused"?"questionPaused":decision.resolved==="abandoned"?"questionAbandoned":"questionWaiting");
  if(decision.automatic) return tv("automaticApproval");
  if (decision.decision === true) return tv("decisionAllowed");
  if (decision.decision === false) return tv("decisionDenied");
  if (decision.resolved === "dismissed") return tv("decisionDismissed");
  if (decision.resolved === "resumed") return tv("decisionResumed");
  if (turnState === "running") return tv("decisionWaiting");
  return tv("decisionEnded");
}

function Row({ children, raw, tone = "" }: { children: ReactNode; raw: unknown; tone?: string }) {
  return (
    <li className={`min-w-0 rounded border p-2 ${tone}`}>
      <RawToggle value={raw}>{children}</RawToggle>
    </li>
  );
}

/** The transcript stays in place while execution history scrolls independently. */
export function ExecutionDetailsPanel({ summary, children }: { summary: string; children: () => ReactNode }) {
  const [open, setOpen] = useState(false);
  return (
    <Dialog.Root open={open} onOpenChange={setOpen}>
      <Dialog.Trigger asChild>
        <Button type="button" data-execution-details variant="ghost" className="h-auto min-h-11 max-w-full justify-start whitespace-normal px-0 text-left text-xs text-muted-foreground">
          {summary}
        </Button>
      </Dialog.Trigger>
      <Dialog.Portal>
        <Dialog.Overlay className="fixed inset-0 z-50 bg-black/35" />
        <Dialog.Content data-execution-panel aria-describedby={undefined} className="fixed inset-y-0 right-0 z-50 flex h-dvh w-full min-w-0 flex-col border-l bg-background text-foreground shadow-xl outline-none md:max-w-2xl lg:max-w-3xl">
          <header className="flex shrink-0 items-start gap-3 border-b px-4 py-3 pt-[max(.75rem,env(safe-area-inset-top))]">
            <Dialog.Title className="min-w-0 flex-1 break-words pt-2.5 text-sm font-semibold">{summary}</Dialog.Title>
            <Dialog.Close asChild><Button type="button" variant="ghost" size="icon" className="min-h-11 min-w-11 shrink-0" aria-label={commonText("common.close")}><X className="size-4" /></Button></Dialog.Close>
          </header>
          <div data-execution-panel-scroll className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 pb-[max(1rem,env(safe-area-inset-bottom))] text-xs">
            {open && children()}
          </div>
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

/** Readable (default) or raw history of a turn's tools, requests and decisions. */
export function ExecutionTimeline({
  events,
  decisions,
  turnState,
  usage,
  stateLabel,
  deciderName,
  viewerId,
  deciders,
}: {
  events: readonly ActivityEvent[];
  decisions: Record<string, Decision>;
  turnState: string;
  usage: { input_tokens: number; output_tokens: number } | null | undefined;
  stateLabel: string;
  deciderName?: string;
  viewerId: string;
  deciders?: readonly { id: string; name: string }[];
}) {
  const [view, setView] = useState(() => storedView(viewerId));
  const entries = timelineEntries(events, decisions);
  const choose = (next: "readable" | "raw") => {
    setView(next);
    storeView(viewerId, next);
  };
  return (
    <div className="mt-2 space-y-2" data-execution-timeline={view}>
      <div role="group" className="sticky top-0 z-10 flex gap-1 border-b bg-background py-1">
        {(["readable", "raw"] as const).map((value) => (
          <Button key={value} type="button" size="sm" variant={view === value ? "secondary" : "ghost"} className="min-h-11 px-2 text-xs" aria-pressed={view === value} onClick={() => choose(value)}>
            {tv(value)}
          </Button>
        ))}
      </div>
      {view === "raw" ? (
        <ul className="space-y-1">
          {entries.map((entry) => (
            <li key={entry.key} className="min-w-0 rounded border p-2">
              <p>{entry.kind === "tool" ? entry.name : entry.kind === "platform" ? tv("platformResult") : entry.decision.tool}</p>
              <LongText value={entry.kind === "tool" ? { name: entry.name, state: entry.state, input: entry.input } : entry.kind === "platform" ? entry.event : entry.decision} className="mt-1 max-h-64" />
            </li>
          ))}
        </ul>
      ) : (
        <ul className="space-y-1">
          {entries.map((entry) => {
            if (entry.kind === "tool")
              return (
                <Row key={entry.key} raw={{ name: entry.name, state: entry.state, input: entry.input }}>
                  <p className="break-words">
                    <span className="text-muted-foreground">{tv("toolCall")} · </span>
                    <span className="font-medium text-foreground">{entry.name}</span>
                    {entry.state && <span> · {entry.state === "started" || entry.state === "running" ? tv("toolRunning") : entry.state === "completed" || entry.state === "finished" ? tv("toolFinished") : entry.state}</span>}
                  </p>
                  {entry.input !== undefined && <Summary tool={entry.name} input={entry.input} />}
                </Row>
              );
            if (entry.kind === "platform") {
              const result = record(entry.event.result) ?? {};
              const failed = platformFailed(entry.event.result);
              const call = entry.event.call;
              const title = call ? operationTitle(call.tool ?? "ferry_request", call.method, call.path) : tv("platformResult");
              const status = typeof result.status === "number" ? result.status : undefined;
              const message = failed ? errorMessage(result) : "";
              const method = call?.method?.toUpperCase();
              const created = !failed && method && ["POST", "PUT", "PATCH"].includes(method) ? createdName(result) : "";
              return (
                <Row key={entry.key} raw={entry.event.result} tone={failed ? "border-destructive/50" : ""}>
                  <p data-platform-entry={failed ? "failed" : "ok"} className="break-words">
                    <span className="font-medium text-foreground">{title}</span>
                    {" → "}
                    {status !== undefined && <span>{status} · </span>}
                    <span className={failed ? "text-destructive" : "text-emerald-700 dark:text-emerald-400"}>{tv(failed ? "platformFailed" : "platformOk")}</span>
                    {created && <span> · {tv(method === "POST" ? "platformCreated" : "platformUpdated", { name: created })}</span>}
                  </p>
                  {message && <p className="mt-1 break-words text-destructive">{message}</p>}
                  {failed && typeof result.guidance === "string" && <p className="mt-1 break-words">{result.guidance}</p>}
                </Row>
              );
            }
            const decidedBy = entry.decision.decided_by
              ? deciders?.find((member) => member.id === entry.decision.decided_by)?.name ?? entry.decision.decided_by
              : deciderName;
            return (
              <Row key={entry.key} raw={entry.decision.input}>
                <p className="break-words">
                  <span className="font-medium text-foreground">{describeOperation(entry.decision.tool, entry.decision.input).title}</span>
                  {" · "}
                  <span>{decisionText(turnState, entry.decision)}</span>
                  {entry.decision.decision !== null && decidedBy && <span> · {tv("decisionBy", { name: decidedBy })}</span>}
                </p>
              </Row>
            );
          })}
        </ul>
      )}
      {usage && (
        <div className="py-1">
          {view === "raw" ? <><p>{stateLabel} · {tv("usageLine", { input: usage.input_tokens, output: usage.output_tokens })}</p><LongText value={usage} className="mt-1 max-h-64" /></> : <RawToggle value={usage}><p>{stateLabel} · {tv("usageLine", { input: usage.input_tokens, output: usage.output_tokens })}</p></RawToggle>}
        </div>
      )}
    </div>
  );
}

/** "3 个工具调用 · 1 个平台操作 · 1 个待批准", or the plain label when empty. */
export function timelineSummary(events: readonly ActivityEvent[], decisions: Record<string, Decision>, turn: TurnLike) {
  const tools = new Set(events.filter((event) => event.type === "tool" && event.item_id).map((event) => event.item_id)).size;
  const platform = new Set(events.filter((event) => event.type === "platform_result" && event.request_id).map((event) => event.request_id)).size;
  const pending = Object.values(decisions).filter((decision) => decision.tool!=="ferry_ask_user"&&awaitingDecision(turn, decision)).length;
  if (!tools && !platform && !pending) return "";
  return tv("detailsSummary", { tools, platform, pending });
}
