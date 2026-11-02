import { act } from "react";
import { createRoot,type Root } from "react-dom/client";
import { afterEach,beforeEach,expect,it,vi } from "vitest";
import { DEFAULT_ACTIONS,parseActions } from "../../features/pipelines/actions-workflow";
import { ActionsEditor } from "../../features/pipelines/ActionsEditor";
import { executionText,i18n } from "../../shared/i18n/i18n";
import { setLocale,type Locale } from "../../shared/i18n/locale";
import catalogs from "../../shared/i18n/locales/execution-domain.json";
import { DeploymentDetail } from "../pages/DeploymentDetail";
import { PipelineRunDetail } from "../pages/PipelineRunDetail";

const locales = Object.keys(catalogs) as Locale[];
let root: Root, host: HTMLDivElement;
const language = async (locale: Locale) => act(async () => { setLocale(locale); await i18n.changeLanguage(locale); });
const fill = async (input: HTMLTextAreaElement, value: string) => act(async () => {
  Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
});
beforeEach(async () => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  Object.defineProperty(document, "hidden", { configurable: true, value: false });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  await language("en-US");
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

it("has complete actual execution translations and matching interpolation contracts", () => {
  const english = catalogs["en-US"];
  for (const catalog of Object.values(catalogs)) {
    expect(Object.keys(catalog).sort()).toEqual(Object.keys(english).sort());
    for (const [key, text] of Object.entries(catalog)) {
      expect(text.trim(), key).not.toBe("");
      expect(text.match(/\{\{[^}]+\}\}/g)?.sort() ?? [], key).toEqual(english[key as keyof typeof english].match(/\{\{[^}]+\}\}/g)?.sort() ?? []);
    }
  }
});

it("refreshes a workflow validation error in all six languages without changing YAML", async () => {
  const definition = "name: User workflow\njobs: {}\n";
  const changed = vi.fn();
  await act(async () => root.render(<ActionsEditor draft={{ definition }} options={[]} readOnly={false} onChange={changed} onValidityChange={() => {}} />));
  for (const locale of locales) {
    await language(locale);
    expect(host.querySelector('[role="alert"]')!.textContent).toBe(catalogs[locale]["workflow.jobsRequired"]);
    expect(host.querySelector('[role="tablist"]')!.getAttribute("aria-label")).toBe(catalogs[locale]["editor.mode"]);
    expect(() => parseActions("jobs:\n  user_job:\n    needs: missing\n    steps: []\n")).toThrow(executionText("workflow.dependencyMissing", { id: "missing" }));
  }
  expect(changed).not.toHaveBeenCalled();
  expect(parseActions(DEFAULT_ACTIONS).jobs[0]!.steps[0]!.name).toBe("步骤1");
});

it("keeps loaded pipeline logs and collapsed steps while translating the execution shell", async () => {
  const rawLog = "用户日志 / user log / 日本語ログ\n";
  const fetcher = vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => url.includes("/logs?")
    ? { items: [{ seq: 1, step_index: 0, stream: "stdout", chunk: rawLog }], next_seq: 1 }
    : { id: "1", number: 7, state: "succeeded", pipeline_id: "2", pipeline_name: "User pipeline", host_id: "3", host_name: "User host", triggered_by: "4", triggered_by_name: "Operator", started_at: 1, finished_at: 5000, commit: "abc", jobs: [{ key: "build", name: "User job", state: "succeeded", step_indices: [0], started_at: 1, finished_at: 5000 }], steps: [{ index: 0, name: "User step", state: "succeeded", exit_code: 0, started_at: 1, finished_at: 5000 }] } }));
  vi.stubGlobal("fetch", fetcher);
  await act(async () => root.render(<PipelineRunDetail workspace="ops" runId="1" permissions={new Set()} />));
  expect(host.textContent).toContain(rawLog);
  const count = fetcher.mock.calls.length;
  for (const locale of locales) {
    await language(locale);
    expect(host.textContent).toContain(executionText("run.number", { number: 7 }));
    expect(host.textContent).toContain(catalogs[locale]["run.logsLoaded"]);
    expect(host.textContent).toContain(rawLog);
  }
  const step = host.querySelector<HTMLButtonElement>('button[aria-expanded="true"]')!;
  await act(async () => step.click());
  await language("ja-JP");
  expect(host.querySelector('button[aria-expanded="false"]')).not.toBeNull();
  expect(fetcher.mock.calls.length).toBe(count);
});

it("preserves a deployment verification note and raw remote output across all languages", async () => {
  const rawOutput = "user command output: echo 部署\n";
  const fetcher = vi.fn(async () => ({ ok: true, status: 200, json: async () => ({ id: "1", name: "User application", state: "needs-review", target_ref: "2", version: 1, latest_revision: { id: "r1", number: 3, state: "needs-review", action: "apply", output: rawOutput, artifact: "services: {}" } }) }));
  vi.stubGlobal("fetch", fetcher);
  await act(async () => root.render(<DeploymentDetail workspace="ops" deploymentId="1" permissions={new Set(["deployments.apply"])} />));
  await fill(host.querySelector<HTMLTextAreaElement>('textarea')!, "Unsaved verification note");
  const count = fetcher.mock.calls.length;
  for (const locale of locales) {
    await language(locale);
    expect(host.textContent).toContain(catalogs[locale]["deployment.reviewHelp"]);
    expect(host.querySelector<HTMLTextAreaElement>('textarea')!.value).toBe("Unsaved verification note");
    expect(host.textContent).toContain(rawOutput);
    expect(host.textContent).toContain("User application");
  }
  expect(fetcher.mock.calls.length).toBe(count);
});
