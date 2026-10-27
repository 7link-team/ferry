import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { App } from "./App";

let host: HTMLDivElement;
let root: Root;
let required: boolean;
let permissions: string[];
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState({}, "", "/ops/hosts/hosts");
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  required = true;
  permissions = ["hosts.read"];
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => {
    if (url === "/api/setup") return { required, has_users: true, registration_enabled: !required };
    if (url === "/api/workspaces/ops/preferences") return {default_page_size:20,dictionaries:{}};
    if (url === "/api/me") return { user: { id: "1", name: "Existing owner", email: "owner@example.invalid", is_superadmin: false }, workspaces: [{ id: "2", name: "Existing workspace", slug: "ops", role_name: "拥有者", permissions }] };
    if (url === "/api/workspaces/ops/repository-auth?page=1&page_size=20") return { items: ["github", "gitlab", "gitee", "codeup"].map(provider => ({id:provider,provider,web_available:false,state:"not-connected",account:"",expires_at:0,credential_id:null,setup:"尚未配置网页授权应用",last_error:""})), total:4, page:1, page_size:20 };
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

it("keeps authorization and token routes inside one repository menu", async () => {
  required = false;
  permissions = ["repositories.read", "credentials.read", "credentials.create"];
  history.replaceState({}, "", "/ops/build/repository-auth");
  await act(async () => root.render(<App />));
  const sidebar = host.querySelector('nav[aria-label="构建"]')!;
  expect([...sidebar.querySelectorAll("a")].map(a => a.textContent)).toEqual(["仓库", "授权认证"]);
  expect(sidebar.querySelector('[aria-current="page"]')?.textContent).toBe("仓库");
  expect(host.querySelector('nav[aria-label="仓库页面"]')?.textContent).toContain("平台授权");
  const web = [...host.querySelectorAll("button")].filter(b => b.textContent === "网页授权");
  expect(web).toHaveLength(3);
  expect(web.every(b => b.disabled && b.title.includes("尚未配置"))).toBe(true);
});
it("groups deployment history and configuration files under application deployment", async () => {
  required = false;
  permissions = ["deployments.read", "config-templates.read"];
  history.replaceState({}, "", "/ops/deploy/config-templates");
  await act(async () => root.render(<App />));
  expect(host.querySelector('nav[aria-label="部署"]')?.textContent).toBe("应用部署");
  const pages = host.querySelector('nav[aria-label="应用部署页面"]')!;
  expect(pages.textContent).toBe("应用部署部署历史应用配置文件");
  expect(host.textContent).toContain("nginx.conf");
  expect(pages.querySelector('[aria-current="page"]')?.textContent).toBe("应用配置文件");
});
it("retains a usable explanation and return path for unsupported detail URLs", async () => {
  required = false;
  permissions = ["credentials.read"];
  history.replaceState({}, "", "/ops/build/repository-auth/github");
  await act(async () => root.render(<App />));
  expect(host.textContent).toContain("此页面不支持当前操作");
  expect(host.textContent).not.toContain("还未实现");
  await act(async () => [...host.querySelectorAll("button")].find(b => b.textContent === "返回列表")!.click());
  expect(location.pathname).toBe("/ops/build/repository-auth");
});

it("keeps nested authorization reachable for a role without repository-read permission", async () => {
  required = false;
  permissions = ["credentials.read"];
  history.replaceState({}, "", "/ops/build/credentials");
  await act(async () => root.render(<App />));
  const repository = [...host.querySelectorAll('nav[aria-label="构建"] a')].find(a => a.textContent === "仓库") as HTMLAnchorElement;
  expect(repository.pathname).toBe("/ops/build/repository-auth");
  await act(async () => repository.click());
  expect(host.querySelector("main")?.textContent).not.toContain("没有访问此页面的权限");
  expect(host.querySelector("h1")?.textContent).toBe("平台授权");
});
