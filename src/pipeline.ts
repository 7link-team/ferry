/** Wire types and presentation shared by build history and run detail. */
export interface PipelineRun {
  id: string;
  number: number;
  pipeline_id: string;
  host_id: string;
  triggered_by: string;
  state: string;
  commit: string;
  started_at: number;
  finished_at: number;
  steps: PipelineStep[];
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
  if (seconds < 60) return `${seconds} 秒`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分 ${seconds % 60} 秒`;
  return `${Math.floor(seconds / 3600)} 时 ${Math.floor(seconds % 3600 / 60)} 分 ${seconds % 60} 秒`;
}

export function stateBadge(state: string): {
  label: string; variant: "default" | "secondary" | "destructive" | "outline"; className: string;
} {
  switch (state) {
    case "queued": return { label: "排队中", variant: "secondary", className: "" };
    case "running": return { label: "运行中", variant: "default", className: "" };
    case "succeeded": return { label: "成功", variant: "outline", className: "text-success border-success" };
    case "failed": return { label: "失败", variant: "destructive", className: "" };
    case "cancelled": return { label: "已取消", variant: "outline", className: "text-muted-foreground" };
    case "timed_out": return { label: "超时", variant: "outline", className: "text-destructive border-destructive" };
    case "skipped": return { label: "已跳过", variant: "secondary", className: "" };
    default: return { label: state, variant: "outline", className: "" };
  }
}
export const isActive = (state: string) => state === "queued" || state === "running";
