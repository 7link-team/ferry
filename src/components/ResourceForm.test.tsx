import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { ResourceForm } from "./ResourceForm";
import { formConfigs } from "../resources";
let root: Root, host: HTMLDivElement, requests: { url: string; method: string | undefined; body: string | undefined }[];
let stored: Record<string, unknown>;
const perms = new Set(["variables.read", "variables.update", "variables.reveal"]);
const mount = async (permissions = perms) => {
  await act(async () => root.render(<ResourceForm config={formConfigs("ops").variables!} workspace="ops" permissions={permissions} />));
};
const click = async (text: string) => {
  const button = [...host.querySelectorAll("button")].find(button => button.textContent === text);
  expect(button).toBeDefined();
  await act(async () => button!.click());
};
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  history.replaceState({}, "", "/ops/deploy/variables/10");
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  requests = [];
  stored = { id: "10", name: "db-url", kind: "secret", scope: "global", scope_ref: "0", has_value: true, version: 1 };
  vi.stubGlobal("fetch", vi.fn(async (url: string, init: RequestInit = {}) => {
    requests.push({ url, method: init.method, body: init.body as string });
    return { ok: true, status: 200, json: async () => url.endsWith("/reveal") ? { value: "secret-value\n$literal" } : stored };
  }));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
it("secret detail only fetches metadata; reveal is explicit, audited endpoint and can hide", async () => {
  await mount();
  expect(host.textContent).toContain("已设置");
  expect(requests).toHaveLength(1);
  expect(host.textContent).not.toContain("secret-value");
  await click("查看明文（记录审计）");
  expect(requests.at(-1)).toMatchObject({ url: "/api/workspaces/ops/variables/10/actions/reveal", method: "POST" });
  expect(host.querySelector('[aria-label="变量明文"]')?.textContent).toBe("secret-value\n$literal");
  await click("隐藏明文");
  expect(host.textContent).not.toContain("secret-value");
});
it("read permission never offers reveal", async () => {
  await mount(new Set(["variables.read"]));
  expect(host.textContent).not.toContain("查看明文");
  expect(requests).toHaveLength(1);
});
it("editing a secret omits its value unless replacement is explicitly enabled", async () => {
  history.replaceState({}, "", "/ops/deploy/variables/10/edit");
  await mount();
  expect(host.querySelectorAll("textarea")).toHaveLength(1); // description only
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  const unchanged = requests.find(r => r.method === "PATCH");
  expect(JSON.parse(unchanged!.body!)).not.toHaveProperty("value");
});
it("an explicitly selected empty replacement sends an empty string", async () => {
  history.replaceState({}, "", "/ops/deploy/variables/10/edit");
  await mount();
  await act(async () => (host.querySelector('[role="checkbox"]') as HTMLElement).click());
  expect(host.querySelectorAll("textarea")).toHaveLength(2);
  await act(async () => host.querySelector("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(JSON.parse(requests.find(r => r.method === "PATCH")!.body!)).toHaveProperty("value", "");
});
it("unmounting a revealed detail does not carry plaintext into another detail", async () => {
  await mount(); await click("查看明文（记录审计）");
  await act(async () => root.render(<div />));
  await mount();
  expect(host.textContent).not.toContain("secret-value");
  expect(requests.filter(r => r.url.endsWith("/reveal"))).toHaveLength(1);
});
