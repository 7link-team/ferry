import { commonText } from "../../shared/i18n/i18n";
import { base } from "../../shared/api/resource";
export interface FailoverSummary {
  mode: string; host_down_after: number;
  fences: { host_id: string; instance: string; created_at: number }[];
  attention: { instance: string; since: number }[];
}
export interface FailoverEvent {
  id: string; kind: string; instance: string; host_id: string; host_name?: string;
  revision_id: string; created_at: number; message: string; detail: Record<string, unknown>;
}
/** Instance names are `<role>-<index>`; role names may themselves contain dashes. */
export function instanceResource(workspace: string, id: string, instance: string): string | undefined {
  const match = /^(.*)-(\d+)$/.exec(instance);
  return match?.[1] ? `${base(workspace, "deployments")}/${encodeURIComponent(id)}/instances/${encodeURIComponent(match[1])}/${match[2]}` : undefined;
}
export const revisionActionLabel = (action: string) => {
  const key = ({ apply: "variable.deploy", rollback: "deployment.rollback", restart: "failover.restart", "auto-replace": "failover.autoReplace", "manual-replace": "failover.manualReplace" } as Record<string, string>)[action];
  return key ? commonText(`resource-domain:${key}`) : commonText("state.unknown", { value: action });
};
export interface Deployment {
  id: string; name: string; category?: string; manifest: string; mode: string; target_kind: string;
  target_ref: string; workdir: string; description: string; state: string;
  target_name?: string; revision_number?: number; created_at?: number; updated_at?: number;
  version: number; current_revision: string; latest_revision?: Revision;
  current_targets?: { host_id: string; current_revision: string }[];
  /** Blueprint deployments: where each role's instances run. */
  blueprint_binding?: BlueprintBinding | null;
  /** Deployment spec (`ferry/deploy-v1`) the server derives the blueprint deployment from. */
  deploy_spec?: string | null;
  failover?: FailoverSummary | null;
}
export interface BlueprintBinding { driver: string; manager?: string; roles: Record<string, { hostId: string; address: string }[]> }
/** Once any host runs a revision, the server freezes the topology (`blueprint.topologyChangeRequiresPlan`). */
export const deploymentDeployed = (app: Pick<Deployment, "current_revision" | "current_targets">) =>
  (!!app.current_revision && app.current_revision !== "0") || !!app.current_targets?.length;
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
