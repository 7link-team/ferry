import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { App } from "./App";
import { parseActions } from "./actions-workflow";

vi.mock("./components/YamlEditor", () => ({
  YamlEditor: ({ value }: { value: string }) => <textarea aria-label="GitHub Actions YAML" value={value} readOnly />,
}));

let host: HTMLDivElement;
let root: Root;
let required: boolean;
let permissions: string[];
beforeEach(() => {
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState({}, "", "/workspaces/2/nodes");
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  required = true;
  permissions = ["hosts.read"];
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => {
    if (url === "/api/setup") return { required, has_users: true, registration_enabled: !required };
    if (url === "/api/workspaces/2/preferences") return {default_page_size:20,dictionaries:{}};
    if (url === "/api/me") return { user: { id: "1", name: "Existing owner", email: "owner@example.invalid", is_superadmin: false }, workspaces: [{ id: "2", name: "Existing workspace", slug: "ops", role_name: "拥有者", permissions }] };
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
  expect(location.pathname).toBe("/workspaces/2/nodes");
});
it("does not ask existing users to initialize an already initialized platform", async () => {
  required = false;
  await act(async () => root.render(<App />));
  expect(host.textContent).not.toContain("初始化平台超级管理员");
  expect(host.textContent).not.toContain("稍后初始化，继续工作空间");
});
it("shows alert rules and events for the current host", async () => {
  required = false;
  permissions = ["hosts.read", "alerts.read", "alerts.create"];
  history.replaceState({}, "", "/workspaces/2/nodes/42?tab=alerts");
  await act(async () => root.render(<App />));
  expect(host.querySelector('[role="tablist"][aria-label="主机报警记录"]')).not.toBeNull();
  expect(vi.mocked(fetch).mock.calls.some(([url]) => String(url).includes("/alert-rules?") && String(url).includes("target_ref=42"))).toBe(true);
  await act(async () => host.querySelector<HTMLButtonElement>('[role="tablist"][aria-label="主机报警记录"] button:last-child')!.click());
  expect(vi.mocked(fetch).mock.calls.some(([url]) => String(url).includes("/alert-events?") && String(url).includes("target_ref=42"))).toBe(true);
});

it("keeps connection details in basic information and paged repository tabs", async () => {
  required = false;
  permissions = ["repositories.read", "repositories.update"];
  const original = vi.mocked(fetch).getMockImplementation()!;
  vi.stubGlobal("fetch", vi.fn(async (url: string, options?: RequestInit) => {
    if (url === "/api/workspaces/2/code-connections/42") return { ok: true, status: 200, json: async () => ({ id: "42", name: "Team Codeup", provider: "codeup", credential_id: "11", organization: "org", version: 1 }) };
    if (url.startsWith("/api/workspaces/2/code-connections/42/repositories")) {
      const second = new URL(url, location.origin).searchParams.get("page") === "2";
      return { ok: true, status: 200, json: async () => ({ items: [{ name: second ? "second" : "first", full_name: second ? "org/second" : "org/first", url: "https://git.example.invalid/repo.git", default_branch: "main" }], has_more: !second }) };
    }
    return original(url, options);
  }));
  history.replaceState({}, "", "/workspaces/2/code-connections/42");
  await act(async () => root.render(<App />));
  const tabs = host.querySelector('[role="tablist"][aria-label="仓库连接详情"]')!;
  expect(tabs.textContent).toBe("基本信息仓库列表");
  expect(host.textContent).toContain("Team Codeup");
  expect(vi.mocked(fetch).mock.calls.some(([url]) => String(url).includes("/42/repositories"))).toBe(false);
  await act(async () => tabs.querySelectorAll<HTMLButtonElement>('[role="tab"]')[1]!.focus());
  expect(location.search).toContain("tab=repositories");
  expect(host.querySelector('[role="tabpanel"]:not([hidden])')?.textContent).toContain("org/first");
  expect(host.textContent).not.toContain("共 0 条");
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="下一页"]')!.click());
  expect(host.textContent).toContain("org/second");
  expect(host.querySelector<HTMLButtonElement>('button[aria-label="下一页"]')!.disabled).toBe(true);
  await act(async () => tabs.querySelectorAll<HTMLButtonElement>('[role="tab"]')[0]!.focus());
  expect(location.pathname).toBe("/workspaces/2/code-connections/42");
  expect(host.textContent).toContain("Team Codeup");
});

it("scopes credential menus, lists and create payloads to the current module", async () => {
  required = false;
  permissions = ["credentials.read", "credentials.create"];
  for (const scope of ["host", "build", "deploy"]) {
    history.replaceState({}, "", `/workspaces/2/credentials?scope=${scope}`);
    await act(async () => { window.dispatchEvent(new PopStateEvent("popstate")); root.render(<App />); });
    expect(vi.mocked(fetch).mock.calls.some(([url]) => String(url).includes("/credentials?") && new URL(String(url), location.origin).searchParams.get("scope") === scope)).toBe(true);
    const create = [...host.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent?.trim() === "创建")!;
    await act(async () => create.click());
    expect(location.search).toContain(`scope=${scope}`);
    const form = host.querySelector("main form") ?? host.querySelector("form")!;
    const input = form.querySelector<HTMLInputElement>('input[type="text"]')!;
    await act(async () => { Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, `${scope} login`); input.dispatchEvent(new Event("input", { bubbles: true })); });
    await act(async () => form.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    const posted = [...vi.mocked(fetch).mock.calls].reverse().find(([url, options]) => String(url) === "/api/workspaces/2/credentials" && options?.method === "POST");
    expect(JSON.parse(String(posted?.[1]?.body)).scope).toBe(scope);
  }
});

it("has one repository entry and no separate authorization tabs", async () => {
  required = false;
  permissions = ["repositories.read", "repositories.create", "credentials.read", "credentials.create"];
  history.replaceState({}, "", "/workspaces/2/code-connections");
  await act(async () => root.render(<App />));
  const sidebar = host.querySelector('nav[aria-label="构建"]')!;
  expect([...sidebar.querySelectorAll("a")].map(a => a.textContent)).toEqual(["仓库连接", "授权认证"]);
  expect(sidebar.querySelector('[aria-current="page"]')?.textContent).toBe("仓库连接");
  expect(host.querySelector('nav[aria-label="仓库页面"]')).toBeNull();
  expect(host.textContent).not.toContain("平台授权");
});
it("groups deployment history and configuration files under application deployment", async () => {
  required = false;
  permissions = ["deployments.read", "config-templates.read"];
  history.replaceState({}, "", "/workspaces/2/config-templates");
  await act(async () => root.render(<App />));
  expect(host.querySelector('nav[aria-label="应用"]')?.textContent).toBe("应用部署");
  const pages = host.querySelector('nav[aria-label="应用部署页面"]')!;
  expect(pages.textContent).toBe("应用部署部署历史应用配置文件");
  expect(host.textContent).toContain("nginx.conf");
  expect(pages.querySelector('[aria-current="page"]')?.textContent).toBe("应用配置文件");
});
it("opens legacy repository authorization bookmarks as the unified repository list", async () => {
  required = false;
  permissions = ["repositories.read"];
  history.replaceState({}, "", "/ops/build/repository-auth/github");
  await act(async () => root.render(<App />));
  expect(host.querySelector("h1")?.textContent).toBe("仓库连接");
  expect(host.textContent).not.toContain("平台授权");
});
it("does not expose repository creation to a credentials-only role", async () => {
  required = false;
  permissions = ["credentials.read"];
  history.replaceState({}, "", "/workspaces/2/credentials");
  await act(async () => root.render(<App />));
  expect([...host.querySelectorAll('nav[aria-label="构建"] a')].map(a => a.textContent)).toEqual(["授权认证"]);
});
it("gives pipeline creation a full-page editor with its own action bar", async () => {
  required = false;
  permissions = ["pipelines.read", "pipelines.create", "pipelines.run"];
  history.replaceState({}, "", "/workspaces/2/pipelines/new");
  await act(async () => root.render(<App />));
  expect(host.querySelector('nav[aria-label="模块"]')).toBeNull();
  expect(host.querySelector('nav[aria-label="构建"]')).toBeNull();
  expect(host.querySelector('input[aria-label="流水线名称"]')).not.toBeNull();
  expect(host.querySelector('button[value="save-run"]')?.textContent).toContain("保存并执行");
  expect(host.querySelector('[aria-label="流水线画布"]')).not.toBeNull();
  const settings = [...host.querySelectorAll("button")].find(button => button.textContent === "流水线设置")!;
  expect(settings.parentElement?.querySelector('[role="tablist"]')).not.toBeNull();
  await act(async () => host.querySelector<HTMLButtonElement>('button[title="在此任务后添加串行任务"]')!.click());
  await act(async () => host.querySelector<HTMLButtonElement>('button[title="添加并行任务"]')!.click());
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>('[data-id="build"] button')].find(button => button.textContent?.trim() === "添加步骤")!.click());
  await act(async () => host.querySelector<HTMLButtonElement>('[data-id="job_2"] button[title="删除任务"]')!.click());
  expect(document.body.textContent).toContain('确认删除任务“新任务”？');
  await act(async () => [...document.body.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === '确认删除任务')!.click());
  await act(async () => host.querySelector<HTMLButtonElement>('button[role="tab"][aria-selected="false"]')!.click());
  const jobs = parseActions(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="GitHub Actions YAML"]')!.value).jobs;
  expect(jobs.find(job => job.id === "job_1")?.needs).toEqual(["build"]);
  expect(jobs.find(job => job.id === "job_2")).toBeUndefined();
  expect(jobs.find(job => job.id === "build")?.steps).toHaveLength(2);
});
it("moves pipeline group filtering into a left directory", async () => {
  required = false;
  permissions = ["pipelines.read", "pipeline-groups.read", "pipeline-groups.create", "pipeline-environments.read"];
  history.replaceState({}, "", "/workspaces/2/pipelines?page=3&at=120");
  await act(async () => root.render(<App />));
  const directory = host.querySelector('aside[aria-label="流水线分组"]')!;
  expect(directory.textContent).toContain("全部流水线");
  expect(directory.textContent).toContain("未分组流水线");
  expect(directory.textContent).not.toContain("执行环境");
  expect(host.querySelector('nav[aria-label="流水线页面"]')).toBeNull();
  await act(async () => (Array.from(directory.querySelectorAll("button")).find(button => button.textContent === "未分组流水线")!).click());
  expect(location.search).toContain("group_id=0");
  expect(location.search).not.toContain("page=");
  expect(location.search).not.toContain("at=");
  expect(host.querySelector('aside[aria-label="流水线分组"] [aria-current="page"]')?.textContent).toBe("未分组流水线");
  await act(async () => (directory.querySelector<HTMLButtonElement>('button[aria-label="新建分组"]')!).click());
  expect(document.querySelector('[role="dialog"]')?.textContent).toContain("新建分组");
  expect(location.pathname).toBe("/workspaces/2/pipelines");
});
it("reorders pipeline groups from the directory", async () => {
  required = false;
  permissions = ["pipelines.read", "pipeline-groups.read", "pipeline-groups.update"];
  history.replaceState({}, "", "/workspaces/2/pipelines");
  const original = vi.mocked(fetch);
  vi.stubGlobal("fetch", vi.fn(async (url: string, init?: RequestInit) => {
    if (url.includes("/pipeline-groups") && init?.method !== "PATCH") return { ok: true, status: 200, json: async () => ({ items: [
      { id: "1", name: "Alpha", position: 0, version: 1 },
      { id: "2", name: "Beta", position: 1, version: 1 },
    ], total: 2, page: 1, page_size: 200 }) };
    if (url.includes("/ordering")) return { ok: true, status: 200, json: async () => ({ ok: true }) };
    return original(url, init);
  }));
  await act(async () => root.render(<App />));
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="上移分组 Beta"]')!.click());
  const request = vi.mocked(fetch).mock.calls.find(([url, init]) => String(url).includes("pipeline-groups/ordering") && init?.method === "PATCH");
  expect(JSON.parse(request?.[1]?.body as string)).toEqual({ ids: ["2", "1"] });
});
it("shows host groups inside the host list without a separate menu page", async () => {
  required = false;
  permissions = ["hosts.read", "host-groups.read", "host-groups.create"];
  history.replaceState({}, "", "/workspaces/2/nodes");
  await act(async () => root.render(<App />));
  expect(host.querySelector('nav[aria-label="主机页面"]')).toBeNull();
  const directory = host.querySelector('aside[aria-label="主机分组"]')!;
  expect(directory.textContent).toContain("全部节点");
  expect(directory.textContent).toContain("未分组节点");
  await act(async () => (directory.querySelector<HTMLButtonElement>('button[aria-label="新建分组"]')!).click());
  expect(document.querySelector('[role="dialog"]')?.textContent).toContain("新建分组");
  expect(location.pathname).toBe("/workspaces/2/nodes");
});
it("redirects old group pages to the corresponding directory", async () => {
  required = false;
  permissions = ["hosts.read", "host-groups.read"];
  history.replaceState({}, "", "/workspaces/2/node-groups/42/edit");
  await act(async () => root.render(<App />));
  expect(location.pathname).toBe("/workspaces/2/nodes");
  expect(location.search).toContain("group_id=42");
});
