import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import { editGraphNode, emptyGraph, newNode, ports } from "./graph";
import { outputPortLabel } from "./WorkflowNodePicker";
import { WorkflowPreviewReview } from "./WorkflowPreviewReview";

const { api, listResource } = vi.hoisted(() => ({ api: vi.fn(), listResource: vi.fn() }));
vi.mock("../../shared/api/api", async (original) => ({ ...(await original<typeof import("../../shared/api/api")>()), api }));
vi.mock("../../shared/api/resource", async (original) => ({ ...(await original<typeof import("../../shared/api/resource")>()), listResource }));

const preview = (extra: Record<string, unknown>) => ({ id: "41", name: "Web", url: "/holes/tok/", state: "active", review_state: "waiting", ...extra });
let root: Root, host: HTMLDivElement;
beforeEach(async () => {
  await i18n.changeLanguage("en-US");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.clearAllMocks(); });

const render = async (canReview: boolean) => {
  await act(async () => root.render(<WorkflowPreviewReview workspace="ops" execution="9" node="preview" canReview={canReview} refresh="running:1" />));
};

it("opens the preview in a new tab and reviews it while the review is open", async () => {
  listResource.mockResolvedValue({ items: [preview({})], total: 1, page: 1, page_size: 20 });
  api.mockResolvedValue({});
  await render(true);
  expect(listResource).toHaveBeenCalledWith("ops", "holes", expect.objectContaining({ source_kind: "workflow", source_id: "9", source_ref: "preview" }), expect.anything());
  const link = host.querySelector("a")!;
  expect(link.href).toBe(new URL("/holes/tok/", window.location.origin).href);
  expect(link.target).toBe("_blank");
  expect(link.rel).toContain("noopener");
  expect(host.textContent).toContain("Waiting for review");
  const [approve, reject] = [...host.querySelectorAll("button")];
  expect(approve!.textContent).toBe("Approve");
  expect(reject!.textContent).toBe("Reject");
  listResource.mockResolvedValue({ items: [preview({ review_state: "approved" })], total: 1, page: 1, page_size: 20 });
  await act(async () => approve!.click());
  expect(api).toHaveBeenCalledWith("/workspaces/ops/holes/41/reviews", { method: "POST", body: JSON.stringify({ decision: "approved" }) });
  expect(host.textContent).toContain("Approved");
  expect(host.querySelectorAll("button")).toHaveLength(0);
});

it("hides review buttons without workflows.run and once the preview is gone", async () => {
  listResource.mockResolvedValue({ items: [preview({})], total: 1, page: 1, page_size: 20 });
  await render(false);
  expect(host.querySelector("a")).not.toBeNull();
  expect(host.querySelectorAll("button")).toHaveLength(0);
  listResource.mockResolvedValue({ items: [preview({ state: "revoked" })], total: 1, page: 1, page_size: 20 });
  await act(async () => root.unmount());
  root = createRoot(host);
  await render(true);
  expect(host.querySelector("a")).toBeNull();
  expect(host.querySelectorAll("button")).toHaveLength(0);
  expect(host.textContent).toContain("Revoked");
});

it("shows the exits of the chosen review mode and drops links that no longer exist", () => {
  const node = newNode(emptyGraph(), "host.preview", "Preview");
  expect(node.params.host_id).toBe("");
  expect(ports(node, "outputs").map(port => port.port)).toEqual(["main"]);
  const human = { ...node, params: { ...node.params, review: "human" } };
  expect(ports(human, "outputs").map(port => port.port)).toEqual(["approved", "rejected"]);
  expect(outputPortLabel(human, { port: "approved" })).toBe("Approved");
  expect(outputPortLabel(human, { port: "rejected" })).toBe("Rejected");
  const next = newNode({ ...emptyGraph(), nodes: [node] }, "core.no_op", "Next");
  const graph = { ...emptyGraph(), nodes: [node, next], connections: [{ from: { node: node.id, port: "main", index: 0 }, to: { node: next.id, port: "main", index: 0 } }] };
  expect(editGraphNode(graph, node.id, { params: human.params }).connections).toEqual([]);
});

it("edits the preview parameters in a form and asks for a review timeout only for human review", async () => {
  const { useState } = await import("react");
  const { WorkflowNodeParameters } = await import("./WorkflowNodeParameters");
  let current: Record<string, unknown> = {};
  function Harness() {
    const [node, setNode] = useState(() => newNode(emptyGraph(), "host.preview", "Preview"));
    current = node.params;
    return <WorkflowNodeParameters node={node} readOnly={false} invalid={() => {}} update={params => setNode(value => ({ ...value, params }))} />;
  }
  await act(async () => root.render(<Harness />));
  expect(host.querySelector('[aria-label="Review timeout (minutes, 1–1440)"]')).toBeNull();
  const review = host.querySelector<HTMLSelectElement>('select[aria-label="Acceptance"]')!;
  await act(async () => {
    review.value = "human";
    review.dispatchEvent(new Event("change", { bubbles: true }));
  });
  expect(current.review).toBe("human");
  expect(host.querySelector('[aria-label="Review timeout (minutes, 1–1440)"]')).not.toBeNull();
  expect(host.querySelector('textarea[aria-label="Service start command (optional)"]')?.getAttribute("maxlength")).toBe("4000");
});
