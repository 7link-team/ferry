export interface Deployment {
  id: string; name: string; manifest: string; mode: string; target_kind: string;
  target_ref: string; workdir: string; description: string; state: string;
  version: number; current_revision: string; latest_revision?: Revision;
}
export interface Revision {
  id: string; deployment_id: string; number: number; action: string; state: string;
  rendered: string; artifact: string; output: string; started_at: number; finished_at: number;
}
export interface VariableBinding { service: string; environment: string; name: string; id: string; version: number; scope: string; scope_ref: string; secret: boolean }
export interface Preview { variables?: VariableBinding[]; desired: string; current: string; expected_hash: string; preview_token: string; version: number; notice: string }
export const deploymentActive = (state: string) => state === "queued" || state === "running";
export const deploymentState = (state: string) => ({ idle: "未部署", queued: "排队中", running: "执行中", succeeded: "成功", failed: "失败", "needs-review": "需核实" }[state] ?? state);
/** A deterministic, contiguous change hunk. No claim of a minimal multi-hunk diff. */
export function artifactDiff(before: string, after: string): string {
  if (before === after) return "产物内容一致。";
  const a = before.split("\n"), b = after.split("\n");
  let start = 0, end = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  while (end < a.length - start && end < b.length - start && a[a.length - 1 - end] === b[b.length - 1 - end]) end++;
  return ["--- 目标主机保存的产物", "+++ 待执行产物",
    ...a.slice(Math.max(0, start - 2), start).map((s) => `  ${s}`),
    ...a.slice(start, a.length - end).map((s) => `- ${s}`),
    ...b.slice(start, b.length - end).map((s) => `+ ${s}`),
    ...b.slice(b.length - end, Math.min(b.length, b.length - end + 2)).map((s) => `  ${s}`),
  ].join("\n");
}
