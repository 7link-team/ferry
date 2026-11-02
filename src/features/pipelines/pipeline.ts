import { commonText } from "../../shared/i18n/i18n";
/** Wire types and presentation shared by build history and run detail. */
export interface PipelineRun {
  id: string;
  number: number;
  pipeline_id: string;
  pipeline_name?: string;
  host_id: string;
  host_name?: string;
  environment_id?: string;
  environment_name?: string;
  triggered_by: string;
  triggered_by_name?: string;
  /** workflow_dispatch inputs the run was started with. */
  inputs?: Record<string, unknown>;
  /** GitHub `environment` declared by each job, by job key. */
  environments?: Record<string, { name: string; url: string; approval: boolean }>;
  state: string;
  commit: string;
  started_at: number;
  finished_at: number;
  steps: PipelineStep[];
  definition_format?: string;
  jobs?: { key: string; name?: string; step_indices?: number[]; host_id: string; state: string; started_at: number; finished_at: number }[];
  variables?: { environment: string; name: string; scope: string; scope_ref: string; version: number; secret: boolean }[];
}
export interface PipelineStep {
  index: number;
  name: string;
  state: string;
  exit_code: number | null;
  started_at: number;
  finished_at: number;
}
export interface RunLog {
  seq: number;
  step_index: number;
  stream: string;
  chunk: string;
}
export interface LogPage { items: RunLog[]; next_seq: number }

/** Preserve existing objects (and DOM keys), discard duplicates, order by seq. */
export function mergeLogs(current: RunLog[], incoming: RunLog[]): RunLog[] {
  const seen = new Set(current.map((log) => log.seq));
  const added = incoming.filter((log) => {
    if (seen.has(log.seq)) return false;
    seen.add(log.seq);
    return true;
  });
  return added.length ? [...current, ...added].sort((a, b) => a.seq - b.seq) : current;
}

export function duration(start: number, end: number, now = Date.now()): string {
  if (!start) return "—";
  const seconds = Math.max(0, Math.floor(((end || now) - start) / 1000));
  if (seconds < 60) return commonText("duration.seconds", { seconds });
  if (seconds < 3600) return commonText("duration.minutes", { minutes: Math.floor(seconds / 60), seconds: seconds % 60 });
  return commonText("duration.hours", { hours: Math.floor(seconds / 3600), minutes: Math.floor(seconds % 3600 / 60), seconds: seconds % 60 });
}

export function stateBadge(state: string): {
  label: string; variant: "default" | "secondary" | "destructive" | "outline"; className: string;
} {
  switch (state) {
    case "queued": return { label: commonText("state.queued"), variant: "secondary", className: "" };
    case "cancelling": return { label: commonText("state.cancelling"), variant: "secondary", className: "" };
    case "running": return { label: commonText("state.running"), variant: "default", className: "" };
    case "waiting": return { label: commonText("state.waitingApproval"), variant: "secondary", className: "" };
    case "pending": return { label: commonText("state.waitingConcurrency"), variant: "secondary", className: "" };
    case "approved": return { label: commonText("state.approved"), variant: "secondary", className: "" };
    case "succeeded": return { label: commonText("state.succeeded"), variant: "outline", className: "text-success border-success" };
    case "failed": return { label: commonText("state.failed"), variant: "destructive", className: "" };
    case "cancelled": return { label: commonText("state.cancelled"), variant: "outline", className: "text-muted-foreground" };
    case "timed_out": return { label: commonText("state.timedOut"), variant: "outline", className: "text-destructive border-destructive" };
    case "skipped": return { label: commonText("state.skipped"), variant: "secondary", className: "" };
    default: return { label: state ? commonText("state.unknown", { value: state }) : "—", variant: "outline", className: "" };
  }
}
export const isActive = (state: string) => state === "queued" || state === "running" || state === "cancelling";
