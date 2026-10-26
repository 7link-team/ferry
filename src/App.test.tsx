import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { App } from "./App";

let host: HTMLDivElement;
let root: Root;
let required: boolean;
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState({}, "", "/ops/hosts/hosts");
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  required = true;
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => {
    if (url === "/api/setup") return { required, has_users: true, registration_enabled: !required };
    if (url === "/api/workspaces/ops/preferences") return {default_page_size:20,dictionaries:{}};
    if (url === "/api/me") return { user: { id: "1", name: "Existing owner", email: "owner@example.invalid", is_superadmin: false }, workspaces: [{ id: "2", name: "Existing workspace", slug: "ops", role_name: "拥有者", permissions: ["hosts.read"] }] };
    return { items: [], total: 0, page: 1, page_size: 20 };
  } })));
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });
it("shows initialization to an already authenticated legacy owner, who can defer without losing their route", async () => {
  await act(async () => root.render(<App />));
  expect(host.textContent).toContain("初始化平台超级管理员");
  const defer = [...host.querySelectorAll("button")].find(b => b.textContent === "稍后初始化，继续工作空间")!;
  expect(defer).toBeTruthy();
  await act(async () => defer.click());
  expect(host.textContent).not.toContain("初始化平台超级管理员");
  expect(location.pathname).toBe("/ops/hosts/hosts");
});
it("does not ask existing users to initialize an already initialized platform", async () => {
  required = false;
  await act(async () => root.render(<App />));
  expect(host.textContent).not.toContain("初始化平台超级管理员");
  expect(host.textContent).not.toContain("稍后初始化，继续工作空间");
});
