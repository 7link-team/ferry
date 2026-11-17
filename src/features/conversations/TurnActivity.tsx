import { useState, type ReactNode } from "react";
import { AlertTriangle, ShieldQuestion } from "lucide-react";
import { commonText } from "../../shared/i18n/i18n";
import { Button } from "../../shared/ui/button";
import { LONG_TEXT_LIMIT, LongText } from "./LazyDetails";

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
  resolved?: "resumed" | "dismissed";
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
        <dl className="mt-1 grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 text-xs text-muted-foreground">
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
      <dt>{label}</dt>
      <dd className="line-clamp-3 break-words text-foreground/80">{text}</dd>
    </>
  );
}

function RawToggle({ value, label }: { value: unknown; label?: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div>
      <button type="button" data-raw-toggle aria-expanded={open} className="inline-flex min-h-11 items-center text-xs text-muted-foreground underline-offset-2 hover:underline focus-visible:outline focus-visible:outline-ring" onClick={() => setOpen(!open)}>
        {open ? tv("hideRaw") : label ?? tv("rawData")}
      </button>
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
      <Summary tool={decision.tool} input={decision.input} />
      <RawToggle value={decision.input} />
      {canDecide ? (
        <div className="flex flex-wrap gap-2">
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
}: {
  request: string;
  decision: Decision;
  canAct: boolean;
  busy: boolean;
  onContinue: (operation: string) => void;
}) {
  const { title } = describeOperation(decision.tool, decision.input);
  return (
    <div data-ended-decision={request} className="space-y-2 rounded-lg border border-dashed border-muted-foreground/40 bg-muted/40 p-3 text-sm">
      <p className="flex items-center gap-1.5 text-xs font-semibold text-muted-foreground">
        <AlertTriangle className="size-3.5" aria-hidden />
        {tv("endedUndecided")}
      </p>
      <Summary tool={decision.tool} input={decision.input} />
      <RawToggle value={decision.input} />
      {canAct && (
        <div className="flex gap-2">
          <Button size="sm" className="min-h-11" disabled={busy} onClick={() => {
            // Inputs here are the server's redacted viewer representation. Build
            // the visible follow-up only when requested, with the same raw cap.
            const input = JSON.stringify(decision.input, null, 2) ?? "";
            onContinue(`${title}\n${input.slice(0, LONG_TEXT_LIMIT)}${input.length > LONG_TEXT_LIMIT ? `\n${tv("truncatedNote")}` : ""}`);
          }}>
            {tv("continueTask")}
          </Button>
        </div>
      )}
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
  if (decision.decision === true) return tv("decisionAllowed");
  if (decision.decision === false) return tv("decisionDenied");
  if (turnState === "running") return tv("decisionWaiting");
  return tv("decisionEnded");
}

function Row({ children, raw, tone = "" }: { children: ReactNode; raw: unknown; tone?: string }) {
  return (
    <li className={`min-w-0 rounded border p-2 ${tone}`}>
      {children}
      <div className="mt-1">
        <RawToggle value={raw} />
      </div>
    </li>
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
      <div role="group" className="flex gap-1">
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
          <p>{stateLabel} · {tv("usageLine", { input: usage.input_tokens, output: usage.output_tokens })}</p>
          {view === "raw" ? <LongText value={usage} className="mt-1 max-h-64" /> : <RawToggle value={usage} />}
        </div>
      )}
    </div>
  );
}

/** "3 个工具调用 · 1 个平台操作 · 1 个待批准", or the plain label when empty. */
export function timelineSummary(events: readonly ActivityEvent[], decisions: Record<string, Decision>, turn: TurnLike) {
  const tools = new Set(events.filter((event) => event.type === "tool" && event.item_id).map((event) => event.item_id)).size;
  const platform = new Set(events.filter((event) => event.type === "platform_result" && event.request_id).map((event) => event.request_id)).size;
  const pending = Object.values(decisions).filter((decision) => awaitingDecision(turn, decision)).length;
  if (!tools && !platform && !pending) return "";
  return tv("detailsSummary", { tools, platform, pending });
}
