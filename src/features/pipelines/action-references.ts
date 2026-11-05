import type { ActionJob } from "./actions-workflow";
import { actionPreset } from "./action-inputs";

// Names checked against pinned act v0.2.89 run_context.go/step.go and Ferry
// workers/actions/execution.go. Values are deliberately never loaded here.
export const actionEnvironment = [
  ["CI", ""], ["GITHUB_WORKFLOW", "github.workflow"], ["GITHUB_RUN_ATTEMPT", "github.run_attempt"],
  ["GITHUB_RUN_ID", "github.run_id"], ["GITHUB_RUN_NUMBER", "github.run_number"],
  ["GITHUB_ACTION", "github.action"], ["GITHUB_ACTION_PATH", "github.action_path"],
  ["GITHUB_ACTION_REPOSITORY", "github.action_repository"], ["GITHUB_ACTION_REF", "github.action_ref"],
  ["GITHUB_ACTIONS", ""], ["GITHUB_ACTOR", "github.actor"], ["GITHUB_REPOSITORY", "github.repository"],
  ["GITHUB_EVENT_NAME", "github.event_name"], ["GITHUB_EVENT_PATH", "github.event_path"],
  ["GITHUB_WORKSPACE", "github.workspace"], ["GITHUB_SHA", "github.sha"], ["GITHUB_REF", "github.ref"],
  ["GITHUB_REF_NAME", "github.ref_name"], ["GITHUB_REF_TYPE", "github.ref_type"], ["GITHUB_JOB", "github.job"],
  ["GITHUB_REPOSITORY_OWNER", "github.repository_owner"], ["GITHUB_RETENTION_DAYS", "github.retention_days"],
  ["GITHUB_BASE_REF", "github.base_ref"], ["GITHUB_HEAD_REF", "github.head_ref"],
  ["GITHUB_SERVER_URL", "github.server_url"], ["GITHUB_API_URL", "github.api_url"], ["GITHUB_GRAPHQL_URL", "github.graphql_url"],
  ["GITHUB_ENV", ""], ["GITHUB_OUTPUT", ""], ["GITHUB_PATH", ""], ["GITHUB_STEP_SUMMARY", ""],
] as const;

export const referenceProperty = (key: string): string => /^[a-zA-Z_][a-zA-Z0-9_]*$/.test(key) ? `.${key}` : `['${key.replaceAll("'", "''")}']`;
export const expression = (path: string): string => "${{ " + path + " }}";
export function outputSources(jobs: ActionJob[], jobId?: string, stepIndex?: number): { id: string; path: string; outputs: string[] }[] {
  const current = jobs.find(job => job.id === jobId);
  if (!current) return [];
  const steps = current.steps.slice(0, stepIndex ?? current.steps.length).flatMap(step => typeof step.id === "string" && step.id ? [{ id: `steps.${step.id}`, path: `steps${referenceProperty(step.id)}.outputs`, outputs: actionPreset(String(step.uses ?? ""))?.outputs ?? [] }] : []);
  const needs = jobs.filter(job => current.needs.includes(job.id)).map(job => ({ id: `needs.${job.id}`, path: `needs${referenceProperty(job.id)}.outputs`, outputs: job.raw.outputs && typeof job.raw.outputs === "object" && !Array.isArray(job.raw.outputs) ? Object.keys(job.raw.outputs) : [] }));
  return [...steps, ...needs];
}

/** Only earlier steps and dependency ancestors have guaranteed finished uploads. */
export function artifactNames(jobs: ActionJob[], jobId: string, stepIndex: number): string[] {
  const current = jobs.find(job => job.id === jobId);
  if (!current) return [];
  const dependencies = new Set<string>();
  const collect = (id: string) => {
    if (dependencies.has(id) || id === jobId) return;
    dependencies.add(id);
    jobs.find(job => job.id === id)?.needs.forEach(collect);
  };
  current.needs.forEach(collect);
  const steps = [...jobs.filter(job => dependencies.has(job.id)).flatMap(job => job.steps), ...current.steps.slice(0, stepIndex)];
  return [...new Set(steps.flatMap(step => {
    if (!/^actions\/upload-artifact@/.test(String(step.uses ?? ""))) return [];
    const inputs = step.with;
    // Whole-map expressions have no statically known artifact name.
    if (inputs !== undefined && (!inputs || typeof inputs !== "object" || Array.isArray(inputs))) return [];
    const name = (inputs as Record<string, unknown> | undefined)?.name;
    // Expressions resolve in the producer's context; copying them may change meaning.
    return name === undefined ? ["artifact"] : typeof name === "string" && name && !name.includes("${{") ? [name] : [];
  }))];
}

export function insertActionReference(control: HTMLInputElement | HTMLTextAreaElement, value: string): boolean {
  if (!control.isConnected || control.readOnly || control.disabled) return false;
  const start = control.selectionStart ?? control.value.length, end = control.selectionEnd ?? start;
  const next = control.value.slice(0, start) + value + control.value.slice(end);
  const prototype = control instanceof HTMLTextAreaElement ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
  Object.getOwnPropertyDescriptor(prototype, "value")!.set!.call(control, next);
  control.dispatchEvent(new Event("input", { bubbles: true }));
  control.focus();
  control.setSelectionRange(start + value.length, start + value.length);
  return true;
}
