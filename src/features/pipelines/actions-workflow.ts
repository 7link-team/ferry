import { Document, isAlias, isCollection, isMap, isScalar, parseDocument, visit, type YAMLMap } from "yaml";
import { executionText } from "../../shared/i18n/i18n";

export type ActionStep = Record<string, unknown> & { name?: string; run?: string; uses?: string; env?: Record<string, string> };
export interface ActionJob { id: string; name: string; needs: string[]; runsOn: string[]; steps: ActionStep[]; raw: Record<string, unknown> }
export function defaultActionRun(index = 1): string { return 'echo "CI_${{ github.job }}_STEP_' + index + '"'; }
export const DEFAULT_ACTIONS = `name: 新流水线
on: workflow_dispatch
jobs:
  build:
    name: 任务 1
    runs-on: self-hosted
    timeout-minutes: 30
    steps:
      - name: 步骤1
        run: ${defaultActionRun()}
`;
const object = (value: unknown): value is Record<string, unknown> => !!value && typeof value === "object" && !Array.isArray(value);
const strings = (value: unknown): string[] => value == null ? [] : Array.isArray(value) ? value.map(String) : [String(value)];

export interface DispatchInput { name: string; type: string; required: boolean; default?: unknown; options: string[]; description: string }

/** The workflow_dispatch inputs a workflow declares, in declaration order. */
export function dispatchInputs(source: string): DispatchInput[] {
  try {
    const on = (parseDocument(source).toJS() as Record<string, unknown> | null)?.on;
    const dispatch = on && typeof on === "object" ? (on as Record<string, unknown>).workflow_dispatch : undefined;
    const inputs = dispatch && typeof dispatch === "object" ? (dispatch as Record<string, unknown>).inputs : undefined;
    if (!inputs || typeof inputs !== "object") return [];
    return Object.entries(inputs as Record<string, unknown>).map(([name, raw]) => {
      const input = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
      return {
        name, type: typeof input.type === "string" ? input.type : "string", required: input.required === true,
        ...(input.default === undefined ? {} : { default: input.default }),
        options: Array.isArray(input.options) ? input.options.map(String) : [],
        description: typeof input.description === "string" ? input.description : "",
      };
    });
  } catch {
    return [];
  }
}

export function parseActions(source: string): { document: Document; jobs: ActionJob[] } {
  const document = parseDocument(source, { uniqueKeys: true });
  if (document.errors.length) throw new Error(document.errors.map(error => error.message).join("\n"));
  const value = document.toJS({ maxAliasCount: 50 });
  if (!object(value) || !object(value.jobs) || !Object.keys(value.jobs).length) throw new Error(executionText("workflow.jobsRequired"));
  const jobs = Object.entries(value.jobs).map(([id, raw]) => {
    if (!/^[a-zA-Z_][a-zA-Z0-9_-]*$/.test(id) || !object(raw)) throw new Error(executionText("workflow.invalidJobId", { id }));
    if (raw.steps !== undefined && (!Array.isArray(raw.steps) || raw.steps.some(step => !object(step)))) throw new Error(executionText("workflow.stepsArray", { id }));
    const runner = raw["runs-on"];
    return { id, name: String(raw.name || id), needs: strings(raw.needs), runsOn: strings(object(runner) ? runner.labels : runner), steps: (raw.steps || []) as ActionStep[], raw };
  });
  const byId = new Map(jobs.map(job => [job.id, job]));
  const visited = new Set<string>(), visiting = new Set<string>();
  function visit(id: string) {
    const job = byId.get(id);
    if (!job) throw new Error(executionText("workflow.dependencyMissing", { id }));
    if (visiting.has(id)) throw new Error(executionText("workflow.cycle"));
    if (visited.has(id)) return;
    visiting.add(id); job.needs.forEach(visit); visiting.delete(id); visited.add(id);
  }
  jobs.forEach(job => visit(job.id));
  return { document, jobs };
}
function serialize(document: Document) { const source = document.toString({ lineWidth: 0 }); parseActions(source); return source; }
function patch(document: Document, path: (string | number)[], values: Record<string, unknown>, deleteNull = true) {
  for (const [key, value] of Object.entries(values)) {
    const target = [...path, key];
    if (value === undefined || (value === null && deleteNull)) document.deleteIn(target);
    else {
      const existing = document.getIn(target, true);
      if (object(value) && isMap(existing)) {
        // Replacing a whole map loses comments/quotes/anchors on unchanged entries.
        // Form rows describe the complete map; remove only deleted keys, then patch.
        for (const entry of [...existing.items]) {
          const name = String(entry.key);
          if (!Object.hasOwn(value, name)) existing.delete(name);
        }
        patch(document, target, value, false);
      } else if (JSON.stringify(document.getIn(target)) !== JSON.stringify(value)) {
        document.setIn(target, value);
      }
    }
  }
}
export function editActionWorkflow(source: string, values: Record<string, unknown>): string {
  const { document } = parseActions(source);
  patch(document, [], values); return serialize(document);
}
export function editActionJob(source: string, job: string, values: Record<string, unknown>): string {
  const { document, jobs } = parseActions(source);
  if (!jobs.some(row => row.id === job)) throw new Error(executionText("workflow.jobMissing"));
  patch(document, ["jobs", job], values); return serialize(document);
}
/** Approval jobs allow only name/needs/ferry-approval; never leave executable fields behind. */
export function setActionJobApproval(source: string, id: string, approval: boolean): string {
  const { jobs } = parseActions(source);
  const job = jobs.find(row => row.id === id);
  if (!job || job.raw.uses !== undefined) throw new Error(executionText("workflow.jobMissing"));
  if (!approval) return editActionJob(source, id, { "ferry-approval": null, "runs-on": "self-hosted", steps: [{ name: "步骤1", run: defaultActionRun() }] });
  const removed = Object.fromEntries(Object.keys(job.raw).filter(key => key !== "name" && key !== "needs").map(key => [key, null]));
  return editActionJob(source, id, { ...removed, "ferry-approval": true });
}
export function editActionStep(source: string, job: string, index: number, values: Record<string, unknown>): string {
  const { document, jobs } = parseActions(source);
  if (!jobs.find(row => row.id === job)?.steps[index]) throw new Error(executionText("workflow.stepMissing"));
  patch(document, ["jobs", job, "steps", index], values); return serialize(document);
}

export function actionFragment(source: string, job: string, index?: number): string {
  const { document } = parseActions(source);
  const path = index === undefined ? ["jobs", job] : ["jobs", job, "steps", index];
  const node = document.getIn(path, true);
  if (!isMap(node)) throw new Error(executionText("workflow.stepMissing"));
  // Aliases can legitimately refer to workflow-level anchors outside this fragment.
  // The complete workflow is validated again before applying the fragment.
  return new Document(node.clone()).toString({ lineWidth: 0, verifyAliasOrder: false });
}

export function editActionFragment(source: string, job: string, index: number | undefined, fragment: string): string {
  const { document, jobs } = parseActions(source);
  const target = jobs.find(row => row.id === job);
  if (!target || (index !== undefined && !target.steps[index])) throw new Error(executionText("workflow.stepMissing"));
  const edited = parseDocument(fragment, { uniqueKeys: true });
  if (edited.errors.length) throw new Error(edited.errors.map(error => error.message).join("\n"));
  if (!isMap(edited.contents)) throw new Error(executionText("workflow.stepsArray", { id: job }));
  document.setIn(index === undefined ? ["jobs", job] : ["jobs", job, "steps", index], edited.contents);
  return serialize(document);
}
export function connectActionJobs(source: string, from: string, to: string): string {
  const { jobs } = parseActions(source);
  const target = jobs.find(job => job.id === to);
  if (!target || !jobs.some(job => job.id === from)) throw new Error(executionText("workflow.jobMissing"));
  return editActionJob(source, to, { needs: [...new Set([...target.needs, from])] });
}
export function removeActionJob(source: string, id: string): string {
  const { document, jobs } = parseActions(source);
  if (jobs.length <= 1) throw new Error(executionText("workflow.keepJob"));
  document.deleteIn(["jobs", id]);
  for (const job of jobs.filter(job => job.id !== id && job.needs.includes(id))) patch(document, ["jobs", job.id], { needs: job.needs.filter(need => need !== id) });
  return serialize(document);
}
export function addActionJob(source: string, checkout = false, placement?: { job: string; mode: "serial" | "parallel" }): { source: string; id: string } {
  const { document, jobs } = parseActions(source);
  let suffix = 1; while (jobs.some(job => job.id === `job_${suffix}`)) suffix++;
  const id = `job_${suffix}`;
  const anchor = placement && jobs.find(job => job.id === placement.job);
  if (placement && !anchor) throw new Error(executionText("workflow.jobMissing"));
  const needs = anchor ? placement?.mode === "serial" ? [anchor.id] : anchor.needs : [];
  document.setIn(["jobs", id], { name: "新任务", needs, "runs-on": "self-hosted", "timeout-minutes": 30, steps: [...(checkout ? [{ name: "步骤1", uses: "actions/checkout@v4" }] : []), { name: checkout ? "步骤2" : "步骤1", run: defaultActionRun(checkout ? 2 : 1) }] });
  return { source: serialize(document), id };
}
export function addActionStep(source: string, job: string, kind: "run" | "checkout" = "run"): string {
  const { document, jobs } = parseActions(source);
  const row = jobs.find(row => row.id === job); if (!row) throw new Error(executionText("workflow.jobMissing"));
  const name = `步骤${row.steps.length + 1}`;
  document.addIn(["jobs", job, "steps"], kind === "checkout" ? { name, uses: "actions/checkout@v4" } : { name, run: defaultActionRun(row.steps.length + 1) });
  return serialize(document);
}

/** Replace only the chosen dependency; sibling branches and join dependencies survive. */
export function insertActionJob(source: string, from: string, to: string): { source: string; id: string } {
  const { jobs } = parseActions(source);
  const target = jobs.find(job => job.id === to);
  if (!target?.needs.includes(from)) throw new Error(executionText("workflow.jobMissing"));
  const inserted = addActionJob(source, false, { job: from, mode: "serial" });
  return { id: inserted.id, source: editActionJob(inserted.source, to, { needs: target.needs.map(id => id === from ? inserted.id : id) }) };
}
export function removeActionStep(source: string, job: string, index: number): string {
  const { document, jobs } = parseActions(source);
  if ((jobs.find(row => row.id === job)?.steps.length || 0) <= 1) throw new Error(executionText("workflow.keepStep"));
  document.deleteIn(["jobs", job, "steps", index]); return serialize(document);
}

/** Clone syntax nodes so comments/quotes survive; copied anchors own their aliases. */
function copyActionNode(document: Document, path: (string | number)[]) {
  const original = document.getIn(path, true);
  if (!isMap(original)) throw new Error(executionText("workflow.stepMissing"));
  const copy = original.clone() as YAMLMap;
  const used = new Set<string>(), renamed = new Map<string, string>();
  visit(document, (_key, node) => {
    if ((isScalar(node) || isCollection(node)) && node.anchor) used.add(node.anchor);
  });
  visit(copy, (_key, node) => {
    if (!(isScalar(node) || isCollection(node)) || !node.anchor) return;
    const base = node.anchor;
    let name = `${base}_copy`, suffix = 2;
    while (used.has(name)) name = `${base}_copy_${suffix++}`;
    used.add(name); renamed.set(base, name); node.anchor = name;
  });
  visit(copy, (_key, node) => {
    if (isAlias(node) && renamed.has(node.source)) node.source = renamed.get(node.source)!;
  });
  return copy;
}

export function duplicateActionJob(source: string, id: string): { source: string; id: string } {
  const { document, jobs } = parseActions(source);
  const original = jobs.find(job => job.id === id);
  if (!original) throw new Error(executionText("workflow.jobMissing"));
  let key = `${id}_copy`, suffix = 2;
  while (jobs.some(job => job.id === key)) key = `${id}_copy_${suffix++}`;
  const copy = copyActionNode(document, ["jobs", id]);
  copy.set("name", `${original.name} (copy)`);
  document.setIn(["jobs", key], copy);
  return { source: serialize(document), id: key };
}

export function duplicateActionStep(source: string, id: string, index: number): string {
  const { document, jobs } = parseActions(source);
  const job = jobs.find(job => job.id === id), original = job?.steps[index];
  if (!original || !job) throw new Error(executionText("workflow.stepMissing"));
  const copy = copyActionNode(document, ["jobs", id, "steps", index]);
  if (typeof original.id === "string") {
    const base = original.id;
    let next = `${base}_copy`, suffix = 2;
    while (job.steps.some(step => step.id === next)) next = `${base}_copy_${suffix++}`;
    copy.set("id", next);
  }
  document.addIn(["jobs", id, "steps"], copy);
  return serialize(document);
}

/** Each independently dispatched job starts with its own checkout. */
export function ensureActionCheckout(source: string): string {
  const { document, jobs } = parseActions(source);
  let changed = false;
  for (const job of jobs) {
    if (job.raw.uses || job.steps.some(step => /^actions\/checkout@/.test(String(step.uses ?? "")))) continue;
    const steps = document.getIn(["jobs", job.id, "steps"]);
    if (!steps || !("items" in Object(steps))) throw new Error(executionText("workflow.stepsMissing", { id: job.id }));
    (steps as { items: unknown[] }).items.unshift(document.createNode({ name: "拉取代码", uses: "actions/checkout@v4" }));
    changed = true;
  }
  return changed ? serialize(document) : source;
}
