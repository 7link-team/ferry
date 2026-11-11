import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { api } from "../../shared/api/api";
import { WorkflowScheduleInputs } from "./WorkflowScheduleInputs";
import { cronLinks, schedulingForms, type CronJob, type CronRun } from "./resource-config";

vi.mock("../../shared/api/api", async importOriginal => ({ ...await importOriginal<object>(), api: vi.fn() }));
const roots: { root: ReturnType<typeof createRoot>; element: HTMLDivElement }[] = [];
afterEach(async () => { for (const { root, element } of roots.splice(0)) { await act(async () => root.unmount()); element.remove(); } vi.resetAllMocks(); });
const definition = JSON.stringify({ engine: "ferry-graph", schema: 1, nodes: [{ id: "entry", name: "Published entry", type: "core.trigger.manual", params: { input_schema: { type: "object", properties: { message: { type: "string" } }, required: ["message"] } } }], connections: [] });
const ready = () => vi.mocked(api).mockImplementation(async path => (path.endsWith("/revisions/PUBLISHED") ? { definition } : { published_revision_id: "PUBLISHED", lifecycle: "active" }) as never);
async function mount(draft: Partial<CronJob> = {}, readOnly = false) {
  const element = document.createElement("div"); document.body.append(element);
  const root = createRoot(element); roots.push({ root, element });
  const patch = vi.fn(); const validity = vi.fn();
  function Harness() {
    const [value, setValue] = useState<Partial<CronJob>>({ target_ref: "WORKFLOW", inputs: {}, ...draft });
    return <WorkflowScheduleInputs workspace="SPACE" draft={value} readOnly={readOnly} onPatch={next => { patch(next); setValue(value => ({ ...value, ...next })); }} onValidityChange={validity} />;
  }
  await act(async () => root.render(<Harness />));
  return { element, patch, validity };
}
it("loads the published graph, selects its single entry and validates schema inputs", async () => {
  ready();
  const { element, patch, validity } = await mount();
  expect(vi.mocked(api).mock.calls.map(call => call[0])).toEqual(["/workspaces/SPACE/workflows/WORKFLOW", "/workspaces/SPACE/workflows/WORKFLOW/revisions/PUBLISHED"]);
  expect(patch).toHaveBeenCalledWith({ entry_node_id: "entry" });
  expect(element.textContent).toContain("Published entry");
  expect(validity).toHaveBeenLastCalledWith(true);
});
it("accepts valid saved parameters without rewriting them", async () => {
  ready(); const { patch, validity } = await mount({ entry_node_id: "entry", inputs: { message: "Scheduled" } });
  expect(patch).not.toHaveBeenCalled(); expect(validity).toHaveBeenLastCalledWith(false);
});
it("blocks an unpublished target and can retry after publication", async () => {
  vi.mocked(api).mockResolvedValue({ published_revision_id: null, lifecycle: "active" });
  const { element, validity } = await mount();
  expect(element.querySelector('[role="alert"]')).not.toBeNull(); expect(validity).toHaveBeenLastCalledWith(true);
  ready(); await act(async () => element.querySelector("button")!.click());
  expect(element.textContent).toContain("Published entry");
});
it("rejects an entry removed from the published version", async () => {
  ready(); const { patch, element, validity } = await mount({ entry_node_id: "REMOVED", inputs: { message: "Keep" } });
  expect(patch).not.toHaveBeenCalled(); expect(validity).toHaveBeenLastCalledWith(true); expect(element.querySelector('[role="alert"]')).not.toBeNull();
});
it("keeps the stored configuration readable after archival without editable controls", async () => {
  vi.mocked(api).mockResolvedValue({ published_revision_id: "PUBLISHED", lifecycle: "archived" });
  const { element, patch, validity } = await mount({ entry_node_id: "entry", inputs: { message: "Stored value" } }, true);
  expect(element.textContent).toContain("Stored value"); expect(element.querySelector("input,textarea,[role=combobox]")).toBeNull();
  expect(patch).not.toHaveBeenCalled(); expect(validity).toHaveBeenLastCalledWith(false);
});
it("separates pipeline environments and links workflow runs to the visual execution page", () => {
  const fields = schedulingForms({ workspace: "SPACE" } as never)["cron-jobs"]!.sections!.flatMap(section => section.fields ?? []);
  const workflow = { scope: "build", action: "workflow", target_kind: "workflow", target_ref: "W" };
  expect(fields.some(field => field.key === "scope")).toBe(false);
  for (const scope of ["build", "deploy"]) expect(fields.filter(field => field.key === "target_kind" && (!field.when || field.when({ scope })))).toHaveLength(0);
  expect(fields.filter(field => field.key === "target_kind" && field.when?.({ scope: "host" }))).toHaveLength(1);
  const action = fields.find(field => field.key === "action" && field.when?.(workflow))!;
  expect(action.patchOnChange!("workflow", workflow)).toEqual({ target_kind: "workflow", inputs: {} });
  expect(action.resetOnChange).toEqual(expect.arrayContaining(["target_ref", "environment_id", "entry_node_id", "inputs"]));
  const deploy = fields.find(field => field.key === "action" && field.when?.({ scope: "deploy" }))!;
  expect(deploy.patchOnChange!("redeploy", {})).toEqual({ target_kind: "deployment", inputs: {} });
  expect(deploy.patchOnChange!("sync-marketplace", {})).toEqual({ target_kind: "marketplace-source", inputs: {} });
  const environment = fields.find(field => field.key === "environment_id")!;
  expect(environment.when!(workflow)).toBe(false); expect(environment.when!({ ...workflow, action: "pipeline", target_kind: "pipeline" })).toBe(true);
  expect(fields.filter(field => field.key === "action" && field.when?.(workflow)).flatMap(field => field.options as unknown[])).toContainEqual(expect.objectContaining({ value: "workflow" }));
  const link = cronLinks("SPACE")[1]!; const row = { downstream_kind: "workflow-executions", downstream_id: "RUN", workflow_id: "WORKFLOW" } as CronRun;
  expect(link.to!(row)).toMatchObject({ item: "workflows", id: "WORKFLOW", mode: "view" }); expect(link.query!(row)).toEqual({ execution: "RUN" });
});
it("clears workflow-specific validation when switching away from an incomplete workflow action", async () => {
  ready();
  const { validity } = await mount();
  expect(validity).toHaveBeenLastCalledWith(true);
  const mounted = roots.pop()!;
  await act(async () => mounted.root.unmount());
  mounted.element.remove();
  expect(validity).toHaveBeenLastCalledWith(false);
});
