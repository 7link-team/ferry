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
  history.replaceState({}, "", "/ops/hosts/hosts");
  host = document.createElement("div");
  document.body.append(host);
  root = createRoot(host);
  required = true;
  permissions = ["hosts.read"];
  vi.stubGlobal("matchMedia", () => ({ matches: false, addEventListener() {}, removeEventListener() {} }));
  vi.stubGlobal("ResizeObserver", class { observe() {} unobserve() {} disconnect() {} });
  vi.stubGlobal("fetch", vi.fn(async (url: string) => ({ ok: true, status: 200, json: async () => {
    if (url === "/api/setup") return { required, has_users: true, registration_enabled: !required };
    if (url === "/api/workspaces/ops/preferences") return {default_page_size:20,dictionaries:{}};
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
  expect(location.pathname).toBe("/ops/hosts/hosts");
});
it("does not ask existing users to initialize an already initialized platform", async () => {
  required = false;
  await act(async () => root.render(<App />));
  expect(host.textContent).not.toContain("初始化平台超级管理员");
  expect(host.textContent).not.toContain("稍后初始化，继续工作空间");
});

it("has one repository entry and no separate authorization tabs", async () => {
  required = false;
  permissions = ["repositories.read", "repositories.create", "credentials.read", "credentials.create"];
  history.replaceState({}, "", "/ops/build/repositories");
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
  history.replaceState({}, "", "/ops/deploy/config-templates");
  await act(async () => root.render(<App />));
  expect(host.querySelector('nav[aria-label="部署"]')?.textContent).toBe("应用部署");
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
  history.replaceState({}, "", "/ops/build/credentials");
  await act(async () => root.render(<App />));
  expect([...host.querySelectorAll('nav[aria-label="构建"] a')].map(a => a.textContent)).toEqual(["授权认证"]);
});
it("gives pipeline creation a full-page editor with its own action bar", async () => {
  required = false;
  permissions = ["pipelines.read", "pipelines.create", "pipelines.run"];
  history.replaceState({}, "", "/ops/build/pipelines/new");
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
  await act(async () => host.querySelector<HTMLButtonElement>('button[role="tab"][aria-selected="false"]')!.click());
  const jobs = parseActions(host.querySelector<HTMLTextAreaElement>('textarea[aria-label="GitHub Actions YAML"]')!.value).jobs;
  expect(jobs.find(job => job.id === "job_1")?.needs).toEqual(["build"]);
  expect(jobs.find(job => job.id === "job_2")).toBeUndefined();
  expect(jobs.find(job => job.id === "build")?.steps).toHaveLength(2);
});
it("moves pipeline group filtering into a left directory", async () => {
  required = false;
  permissions = ["pipelines.read", "pipeline-groups.read", "pipeline-groups.create", "pipeline-environments.read"];
  history.replaceState({}, "", "/ops/build/pipelines?page=3&at=120");
  await act(async () => root.render(<App />));
  const directory = host.querySelector('aside[aria-label="流水线分组"]')!;
  expect(directory.textContent).toContain("全部流水线");
  expect(directory.textContent).toContain("未分组");
  expect(directory.textContent).not.toContain("执行环境");
  expect(host.querySelector('nav[aria-label="流水线页面"]')).toBeNull();
  await act(async () => (Array.from(directory.querySelectorAll("button")).find(button => button.textContent === "未分组")!).click());
  expect(location.search).toContain("group_id=0");
  expect(location.search).not.toContain("page=");
  expect(location.search).not.toContain("at=");
  expect(host.querySelector('aside[aria-label="流水线分组"] [aria-current="page"]')?.textContent).toBe("未分组");
  await act(async () => (directory.querySelector<HTMLButtonElement>('button[aria-label="新建分组"]')!).click());
  expect(document.querySelector('[role="dialog"]')?.textContent).toContain("新建分组");
  expect(location.pathname).toBe("/ops/build/pipelines");
});
it("shows host groups inside the host list without a separate menu page", async () => {
  required = false;
  permissions = ["hosts.read", "host-groups.read", "host-groups.create"];
  history.replaceState({}, "", "/ops/host/hosts");
  await act(async () => root.render(<App />));
  expect(host.querySelector('nav[aria-label="主机页面"]')).toBeNull();
  const directory = host.querySelector('aside[aria-label="主机分组"]')!;
  expect(directory.textContent).toContain("全部主机");
  await act(async () => (directory.querySelector<HTMLButtonElement>('button[aria-label="新建分组"]')!).click());
  expect(document.querySelector('[role="dialog"]')?.textContent).toContain("新建分组");
  expect(location.pathname).toBe("/ops/host/hosts");
});
it("redirects old group pages to the corresponding directory", async () => {
  required = false;
  permissions = ["hosts.read", "host-groups.read"];
  history.replaceState({}, "", "/ops/host/host-groups/42/edit");
  await act(async () => root.render(<App />));
  expect(location.pathname).toBe("/ops/host/hosts");
  expect(location.search).toContain("group_id=42");
});
