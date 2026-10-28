import { parseDocument, type Document } from "yaml";

export type ActionStep = Record<string, unknown> & { name?: string; run?: string; uses?: string; env?: Record<string, string> };
export interface ActionJob { id: string; name: string; needs: string[]; runsOn: string[]; steps: ActionStep[]; raw: Record<string, unknown> }
export const DEFAULT_ACTIONS = `name: Build
on: workflow_dispatch
jobs:
  build:
    name: 构建
    runs-on: self-hosted
    timeout-minutes: 30
    steps:
      - name: 构建
        run: echo hello
`;
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
const strings = (value: unknown): string[] => value == null ? [] : Array.isArray(value) ? value.map(String) : [String(value)];

export function parseActions(source: string): { document: Document; jobs: ActionJob[] } {
  const document = parseDocument(source, { uniqueKeys: true });
  if (document.errors.length) throw new Error(document.errors.map(error => error.message).join("\n"));
  const value = document.toJS({ maxAliasCount: 50 });
  if (!object(value) || !object(value.jobs) || !Object.keys(value.jobs).length) throw new Error("工作流需要至少一个 jobs 任务。");
  const jobs = Object.entries(value.jobs).map(([id, raw]) => {
    if (!/^[a-zA-Z_][a-zA-Z0-9_-]*$/.test(id) || !object(raw)) throw new Error(`任务标识 ${id} 无效。`);
    if (raw.steps !== undefined && (!Array.isArray(raw.steps) || raw.steps.some(step => !object(step)))) throw new Error(`任务 ${id} 的 steps 必须是步骤数组。`);
    return { id, name: String(raw.name || id), needs: strings(raw.needs), runsOn: strings(raw["runs-on"]), steps: (raw.steps || []) as ActionStep[], raw };
  });
  const byId = new Map(jobs.map(job => [job.id, job]));
  const visited = new Set<string>(), visiting = new Set<string>();
  function visit(id: string) {
    const job = byId.get(id);
    if (!job) throw new Error(`依赖任务 ${id} 不存在。`);
    if (visiting.has(id)) throw new Error("任务依赖存在循环，无法执行。");
    if (visited.has(id)) return;
    visiting.add(id); job.needs.forEach(visit); visiting.delete(id); visited.add(id);
  }
  jobs.forEach(job => visit(job.id));
  return { document, jobs };
}
function serialize(document: Document) { const source = document.toString({ lineWidth: 0 }); parseActions(source); return source; }
function patch(document: Document, path: (string | number)[], values: Record<string, unknown>) {
  for (const [key, value] of Object.entries(values)) {
    if (value === undefined || value === null) document.deleteIn([...path, key]);
    else document.setIn([...path, key], value);
  }
}
export function editActionJob(source: string, job: string, values: Record<string, unknown>): string {
  const { document, jobs } = parseActions(source);
  if (!jobs.some(row => row.id === job)) throw new Error("任务不存在。");
  patch(document, ["jobs", job], values); return serialize(document);
}
export function editActionStep(source: string, job: string, index: number, values: Record<string, unknown>): string {
  const { document, jobs } = parseActions(source);
  if (!jobs.find(row => row.id === job)?.steps[index]) throw new Error("步骤不存在。");
  patch(document, ["jobs", job, "steps", index], values); return serialize(document);
}
export function connectActionJobs(source: string, from: string, to: string): string {
  const { jobs } = parseActions(source);
  const target = jobs.find(job => job.id === to);
  if (!target || !jobs.some(job => job.id === from)) throw new Error("任务不存在。");
  return editActionJob(source, to, { needs: [...new Set([...target.needs, from])] });
}
export function removeActionJob(source: string, id: string): string {
  const { document, jobs } = parseActions(source);
  if (jobs.length <= 1) throw new Error("至少保留一个任务。");
  document.deleteIn(["jobs", id]);
  for (const job of jobs.filter(job => job.id !== id && job.needs.includes(id))) patch(document, ["jobs", job.id], { needs: job.needs.filter(need => need !== id) });
  return serialize(document);
}
export function addActionJob(source: string, checkout = false, placement?: { job: string; mode: "serial" | "parallel" }): { source: string; id: string } {
  const { document, jobs } = parseActions(source);
  let suffix = 1; while (jobs.some(job => job.id === `job_${suffix}`)) suffix++;
  const id = `job_${suffix}`;
  const anchor = placement && jobs.find(job => job.id === placement.job);
  if (placement && !anchor) throw new Error("任务不存在。");
  const needs = anchor ? placement?.mode === "serial" ? [anchor.id] : anchor.needs : [];
  document.setIn(["jobs", id], { name: "新任务", needs, "runs-on": "self-hosted", "timeout-minutes": 30, steps: [...(checkout ? [{ name: "拉取代码", uses: "actions/checkout@v4" }] : []), { name: "执行命令", run: "echo hello" }] });
  return { source: serialize(document), id };
}
export function addActionStep(source: string, job: string, kind: "run" | "checkout" = "run"): string {
  const { document, jobs } = parseActions(source);
  const row = jobs.find(row => row.id === job); if (!row) throw new Error("任务不存在。");
  document.addIn(["jobs", job, "steps"], kind === "checkout" ? { name: "拉取代码", uses: "actions/checkout@v4" } : { name: "执行命令", run: "echo hello" });
  return serialize(document);
}
export function removeActionStep(source: string, job: string, index: number): string {
  const { document, jobs } = parseActions(source);
  if ((jobs.find(row => row.id === job)?.steps.length || 0) <= 1) throw new Error("任务至少保留一个步骤。");
  document.deleteIn(["jobs", job, "steps", index]); return serialize(document);
}

/** Each independently dispatched job starts with its own checkout. */
export function ensureActionCheckout(source: string): string {
  const { document, jobs } = parseActions(source);
  let changed = false;
  for (const job of jobs) {
    if (job.raw.uses || job.steps.some(step => /^actions\/checkout@/.test(String(step.uses ?? "")))) continue;
    const steps = document.getIn(["jobs", job.id, "steps"]);
    if (!steps || !("items" in Object(steps))) throw new Error(`任务 ${job.id} 缺少 steps。`);
    (steps as { items: unknown[] }).items.unshift(document.createNode({ name: "拉取代码", uses: "actions/checkout@v4" }));
    changed = true;
  }
  return changed ? serialize(document) : source;
}
