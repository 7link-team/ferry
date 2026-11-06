import { commonText } from "../../shared/i18n/i18n";
export interface Deployment {
  id: string; name: string; category?: string; manifest: string; mode: string; target_kind: string;
  target_ref: string; workdir: string; description: string; state: string;
  target_name?: string; revision_number?: number; created_at?: number; updated_at?: number;
  version: number; current_revision: string; latest_revision?: Revision;
  current_targets?: { host_id: string; current_revision: string }[];
}
export interface Revision {
  id: string; deployment_id: string; number: number; action: string; state: string;
  deployment_name?: string; triggered_by?: string; triggered_by_name?: string; created_at?: number;
  rendered: string; artifact: string; output: string; started_at: number; finished_at: number;
  targets?: { id: string; host_id: string; state: string; output: string; artifact: string }[];
}
export interface VariableBinding { kind?: string; path?: string; template_version?: number; service: string; environment: string; name: string; id: string; version: number; scope: string; scope_ref: string; secret: boolean }
export interface Preview { target_hashes?: Record<string, string>; variables?: VariableBinding[]; desired: string; current: string; expected_hash: string; preview_token: string; version: number; notice: string }
export const deploymentActive = (state: string) => state === "queued" || state === "running";
export const deploymentState = (state: string) => ({ pending: commonText("state.notStarted"), skipped: commonText("state.skipped"), idle: commonText("state.undeployed"), queued: commonText("state.queued"), running: commonText("state.executing"), succeeded: commonText("state.succeeded"), failed: commonText("state.failed"), "needs-review": commonText("state.needsReview") }[state] ?? (state ? commonText("state.unknown", { value: state }) : "—"));
/** A deterministic, contiguous change hunk. No claim of a minimal multi-hunk diff. */
export function artifactDiff(before: string, after: string): string {
  if (before === after) return commonText("artifact.identical");
  const a = before.split("\n"), b = after.split("\n");
  let start = 0, end = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  while (end < a.length - start && end < b.length - start && a[a.length - 1 - end] === b[b.length - 1 - end]) end++;
  return [commonText("artifact.saved"), commonText("artifact.desired"),
    ...a.slice(Math.max(0, start - 2), start).map((s) => `  ${s}`),
    ...a.slice(start, a.length - end).map((s) => `- ${s}`),
    ...b.slice(start, b.length - end).map((s) => `+ ${s}`),
    ...b.slice(b.length - end, Math.min(b.length, b.length - end + 2)).map((s) => `  ${s}`),
  ].join("\n");
}
