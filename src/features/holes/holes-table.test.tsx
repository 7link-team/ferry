import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import { ResourceTable } from "../../app/resources/ResourceTable";
import { api } from "../../shared/api/api";
import { holesLists } from "./resource-config";

vi.mock("../../app/router", () => ({
  useRoute: () => ({ workspace: "ops", group: "host", item: "holes", mode: "list" }),
  useQueryValue: (_key: string, fallback: unknown) => [fallback, () => {}],
  useScrollSync: () => ({ current: null }),
  href: () => "/detail", navigate: vi.fn(),
}));
const items = vi.hoisted(() => ({ rows: [] as Record<string, unknown>[] }));
const hole = (patch: Record<string, unknown> = {}) => ({ id: "1", name: "Web", host_id: "3", host_name: "build-01", host_online: false, target_port: 5173, url: "/holes/abc/", state: "active",
  expires_at: 1, revoked_at: 0, probe_state: "unreachable", probe_error: "connection refused", probed_at: 0, active_connections: 0, created_by: "1", created_by_name: "Ada",
  source_kind: "manual", source_id: "0", source_ref: "", review_state: "none", ...patch });
vi.mock("../../shared/api/resource", async (original) => ({
  ...(await original<typeof import("../../shared/api/resource")>()),
  listResource: vi.fn(async () => ({ items: items.rows, total: items.rows.length, page: 1, page_size: 20 })),
}));
vi.mock("../../shared/api/api", async (original) => ({
  ...(await original<typeof import("../../shared/api/api")>()),
  api: vi.fn(async () => ({ workflow_id: "44" })),
}));

let root: Root;
let element: HTMLDivElement;
beforeEach(async () => {
  await i18n.changeLanguage("en-US");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  element = document.createElement("div"); document.body.append(element); root = createRoot(element);
  items.rows = [hole()];
});
afterEach(async () => { await act(async () => root.unmount()); element.remove(); localStorage.removeItem("ferry.refresh.holes"); vi.clearAllMocks(); });

const navigate = vi.fn();
const render = async (permissions = ["holes.read", "holes.update", "holes.delete"]) => {
  const config = holesLists({ workspace: "ops", navigate, href: (route) => `/runs/${route.id}` }).holes!;
  await act(async () => root.render(<ResourceTable config={config} workspace="ops" permissions={new Set(permissions)} />));
};
const rowText = (index: number) => element.querySelectorAll("tbody tr")[index]!.textContent ?? "";

it("shows the source and review of manual and automatic previews and links to their owner", async () => {
  items.rows = [
    hole(),
    hole({ id: "2", source_kind: "pipeline", source_id: "81", source_ref: "build", review_state: "waiting" }),
    hole({ id: "3", source_kind: "workflow", source_id: "92", source_ref: "serve", review_state: "approved" }),
  ];
  await render(["holes.read", "holes.update", "pipelines.run"]);
  expect(rowText(0)).toContain("Manual");
  expect(rowText(0)).toContain("Not required");
  expect(rowText(1)).toContain("Waiting");
  expect(rowText(2)).toContain("Approved");
  const run = [...element.querySelectorAll("a")].find((link) => link.textContent === "Pipeline run · build")!;
  expect(run.getAttribute("href")).toBe("/runs/81");
  await act(async () => run.click());
  expect(navigate).toHaveBeenCalledWith({ workspace: "ops", group: "workflow", item: "runs", id: "81", mode: "view" }, {});
  const execution = [...element.querySelectorAll("button")].find((button) => button.textContent === "Workflow execution · serve")!;
  await act(async () => execution.click());
  expect(api).toHaveBeenCalledWith("/workspaces/ops/workflow-executions/92");
  expect(navigate).toHaveBeenLastCalledWith({ workspace: "ops", group: "workflow", item: "workflows", id: "44", mode: "view" }, { execution: "92" });
  const rows = element.querySelectorAll("tbody tr");
  expect(rows[1]!.querySelector('button[aria-label="Approve"]')).not.toBeNull();
  expect(rows[1]!.querySelector('button[aria-label="Reject"]')).not.toBeNull();
  expect(rows[2]!.querySelector('button[aria-label="Approve"]')).toBeNull();
  expect(rows[0]!.querySelector('[aria-label="Edit"]')).not.toBeNull();
  expect(rows[1]!.querySelector('[aria-label="Edit"]')).toBeNull();
  expect(rows[2]!.querySelector('[aria-label="Edit"]')).toBeNull();
});

it("hides review actions from users who cannot run the source", async () => {
  items.rows = [hole({ id: "2", source_kind: "workflow", source_id: "92", source_ref: "serve", review_state: "waiting" })];
  await render(["holes.read", "pipelines.run"]);
  expect(element.querySelector('button[aria-label="Approve"]')).toBeNull();
});

it("refreshes every 5 seconds by default and renders host status and probe errors", async () => {
  await render();
  expect(element.querySelector('[role="timer"]')?.textContent).toBe("5s");
  expect(element.textContent).toContain("build-01");
  expect(element.textContent).toContain("Offline");
  expect(element.textContent).toContain("connection refused");
  expect(element.querySelector('button[aria-label="Copy link"]')).not.toBeNull();
  expect(element.querySelector('button[aria-label="Revoke"]')).not.toBeNull();
});

it("keeps a stored refresh preference over the 5-second default", async () => {
  localStorage.setItem("ferry.refresh.holes", "30");
  await render();
  expect(element.querySelector('[role="timer"]')?.textContent).toBe("30s");
  localStorage.setItem("ferry.refresh.holes", "0");
  await act(async () => root.unmount());
  root = createRoot(element);
  await render();
  expect(element.querySelector('[role="timer"]')).toBeNull();
});
