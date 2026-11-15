import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ResourceForm } from "../../app/resources/ResourceForm";
import { i18n } from "../../shared/i18n/i18n";
import { agentsForms } from "./resource-config";

let root: Root, host: HTMLDivElement;
let requests: { url: string; method: string | undefined }[];
let agent: Record<string, unknown>;

beforeEach(() => {
  void i18n.changeLanguage("zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  requests = [];
  agent = {
    id: "7", name: "Legacy", role: "", platform_role_id: "0", description: "", owner_id: "1", owner_name: "Owner",
    runtime: "codex", model_profile_id: "5", model_profile_name: "Codex", model: "", host_id: "31", host_name: "build-1",
    cwd: "/srv/work", system_prompt: "", permission: "read_only", visibility: "workspace", state: "enabled", version: 2, revision_id: "9",
  };
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({ url, method: init.method });
    if (init.method === "DELETE" && url.includes("/ai-agents/7/host-pin")) {
      agent = { ...agent, host_id: "0", host_name: undefined, cwd: "", version: 3, revision_id: "10" };
      return { ok: true, status: 200, json: async () => ({ ok: true }) };
    }
    if (url.endsWith("/ai-agents/7")) return { ok: true, status: 200, json: async () => agent };
    return { ok: true, status: 200, json: async () => ({ items: [], total: 0, page: 1, page_size: 200 }) };
  }));
});
afterEach(async () => {
  await act(async () => root.unmount());
  host.remove();
  vi.unstubAllGlobals();
});

const render = async (permissions: string[]) => {
  const grants = new Set(permissions);
  const config = agentsForms({ workspace: "ops", permissions: grants } as never)["ai-agents"]!;
  await act(async () => root.render(<ResourceForm config={config} workspace="ops" permissions={grants} />));
};
const unpin = () => [...host.querySelectorAll("button")].find((b) => b.textContent === "取消固定");

it("shows a hidden legacy host pin and removes it with host administration", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-agents/7");
  await render(["ai-agents.read", "ai-agents.update", "hosts.update"]);
  expect(host.textContent).toContain("固定主机：build-1 · 目录：/srv/work");
  expect(host.textContent).toContain("取消固定后改用模型的执行主机");
  await act(async () => unpin()!.click());
  expect(requests.some((r) => r.method === "DELETE" && r.url.endsWith("/ai-agents/7/host-pin?version=2"))).toBe(true);
  expect(host.textContent).not.toContain("固定主机：");
});

it("only shows the pin without host administration, and nothing for an unpinned Agent", async () => {
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-agents/7");
  await render(["ai-agents.read", "ai-agents.update"]);
  expect(host.textContent).toContain("固定主机：build-1");
  expect(unpin()).toBeUndefined();
  await act(async () => root.unmount());
  root = createRoot(host);
  agent = { ...agent, host_id: "0", host_name: undefined, cwd: "" };
  await render(["ai-agents.read", "ai-agents.update", "hosts.update"]);
  expect(host.textContent).toContain("Legacy");
  expect(host.textContent).not.toContain("固定主机");
});

it("saving the edit form keeps a legacy pin", async () => {
  const bodies: Record<string, unknown>[] = [];
  const original = vi.mocked(fetch).getMockImplementation()!;
  vi.mocked(fetch).mockImplementation(async (url, init) => {
    if (init?.method === "PATCH" || init?.method === "PUT") bodies.push(JSON.parse(String(init.body)));
    if (String(url).includes("/ai-models?"))
      return { ok: true, status: 200, json: async () => ({ items: [{ id: "5", name: "Codex", models: [], default_model: "", state: "enabled" }], total: 1, page: 1, page_size: 200 }) } as Response;
    return original(url, init);
  });
  history.replaceState({}, "", "/zh-CN/workspaces/ops/ai-agents/7/edit");
  await render(["ai-agents.read", "ai-agents.update", "hosts.update", "ai-models.read"]);
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(bodies).toHaveLength(1);
  // Either the pin is sent as stored, or omitted (the server then keeps it); never cleared.
  for (const body of bodies) {
    if ("host_id" in body) expect(body).toMatchObject({ host_id: "31", cwd: "/srv/work", permission: "read_only" });
  }
});
