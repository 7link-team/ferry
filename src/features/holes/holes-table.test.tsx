import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { i18n } from "../../shared/i18n/i18n";
import { ResourceTable } from "../../app/resources/ResourceTable";
import { holesLists } from "./resource-config";

vi.mock("../../app/router", () => ({
  useRoute: () => ({ workspace: "ops", group: "host", item: "holes", mode: "list" }),
  useQueryValue: (_key: string, fallback: unknown) => [fallback, () => {}],
  useScrollSync: () => ({ current: null }),
  href: () => "/detail", navigate: vi.fn(),
}));
vi.mock("../../shared/api/resource", async (original) => ({
  ...(await original<typeof import("../../shared/api/resource")>()),
  listResource: vi.fn(async () => ({
    items: [{ id: "1", name: "Web", host_id: "3", host_name: "build-01", host_online: false, target_port: 5173, url: "/holes/abc/", state: "active",
      expires_at: 1, revoked_at: 0, probe_state: "unreachable", probe_error: "connection refused", probed_at: 0, active_connections: 0, created_by: "1", created_by_name: "Ada" }],
    total: 1, page: 1, page_size: 20,
  })),
}));

let root: Root;
let element: HTMLDivElement;
beforeEach(async () => {
  await i18n.changeLanguage("en-US");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  element = document.createElement("div"); document.body.append(element); root = createRoot(element);
});
afterEach(async () => { await act(async () => root.unmount()); element.remove(); localStorage.removeItem("ferry.refresh.holes"); vi.clearAllMocks(); });

const render = async () => {
  const config = holesLists({ workspace: "ops", navigate: () => {} }).holes!;
  await act(async () => root.render(<ResourceTable config={config} workspace="ops" permissions={new Set(["holes.read", "holes.update", "holes.delete"])} />));
};

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
