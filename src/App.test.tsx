import { i18n } from "./i18n";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { App } from "./App";
import { setLocale } from "./locale";
import { parseActions } from "./actions-workflow";
import { RepositoryMonitor, repositoryEventList } from "./components/RepositoryMonitor";

vi.mock("./components/YamlEditor", () => ({
  YamlEditor: ({ value }: { value: string }) => <textarea aria-label="GitHub Actions YAML" value={value} readOnly />,
}));

let host: HTMLDivElement;
let root: Root;
let required: boolean;
let permissions: string[];
beforeEach(() => {
  void i18n.changeLanguage("zh-CN");
  localStorage.setItem("ui.locale", "zh-CN");
  Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
  history.replaceState({}, "", "/zh-CN/workspaces/2/nodes");
  setLocale("zh-CN", false);
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
  expect(host.querySelector<HTMLInputElement>('input[name="password"]')?.minLength).toBe(8);
  expect(host.querySelector<HTMLInputElement>('input[name="confirm_password"]')?.minLength).toBe(8);
  const defer = [...host.querySelectorAll("button")].find(b => b.textContent === "稍后初始化，继续工作空间")!;
  expect(defer).toBeTruthy();
  await act(async () => defer.click());
  expect(host.textContent).not.toContain("初始化平台超级管理员");
  expect(location.pathname).toBe("/zh-CN/workspaces/2/nodes");
});
it("does not ask existing users to initialize an already initialized platform", async () => {
  required = false;
  await act(async () => root.render(<App />));
  expect(host.textContent).not.toContain("初始化平台超级管理员");
  expect(host.textContent).not.toContain("稍后初始化，继续工作空间");
});
it("offers the eight-character minimum on the registration form", async () => {
  required = false;
  const original = vi.mocked(fetch).getMockImplementation()!;
  vi.stubGlobal("fetch", vi.fn(async (url: string) => url === "/api/me"
    ? { ok: false, status: 401, json: async () => ({ error: "unauthorized" }) }
    : original(url)));
  await act(async () => root.render(<App />));
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "还没有账号？注册")!.click());
  const password = host.querySelector<HTMLInputElement>('input[name="password"]')!;
  expect(password.minLength).toBe(8);
  expect(password.placeholder).toContain("至少 8");
});
it("shows alert rules and events for the current host", async () => {
  required = false;
  permissions = ["hosts.read", "alerts.read", "alerts.create"];
  history.replaceState({}, "", "/zh-CN/workspaces/2/nodes/42?tab=alerts");
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
      return { ok: true, status: 200, json: async () => ({ items: [{ name: second ? "second" : "first", full_name: second ? "org/second" : "org/first", url: `https://git.example.invalid/${second ? "second" : "first"}.git`, default_branch: "main" }], has_more: !second }) };
    }
    return original(url, options);
  }));
  history.replaceState({}, "", "/zh-CN/workspaces/2/code-connections/42");
  await act(async () => root.render(<App />));
  const tabs = host.querySelector('[role="tablist"][aria-label="仓库连接详情"]')!;
  expect(tabs.textContent).toBe("基本信息仓库列表监控通知事件记录代码洞察");
  expect(host.textContent).toContain("Team Codeup");
  expect(vi.mocked(fetch).mock.calls.some(([url]) => String(url).includes("/42/repositories"))).toBe(false);
  await act(async () => tabs.querySelectorAll<HTMLButtonElement>('[role="tab"]')[1]!.focus());
  expect(location.search).toContain("tab=repositories");
  expect(host.querySelector('[role="tabpanel"]:not([hidden])')?.textContent).toContain("org/first");
  expect(host.textContent).not.toContain("共 0 条");
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "分组")!.click());
  const namespace = [...host.querySelectorAll<HTMLButtonElement>('button')].find(button => button.textContent === "org")!;
  expect(namespace.textContent).toContain("org");
  await act(async () => namespace.click());
  expect([...host.querySelectorAll('tbody tr')].map(row => row.textContent).join(' ')).toContain("first");
  expect([...host.querySelectorAll('tbody tr')].map(row => row.textContent).join(' ')).toContain("second");
  expect(location.search).toContain("folder=org");
  await act(async () => [...host.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "列表")!.click());
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="下一页"]')!.click());
  expect(host.textContent).toContain("org/second");
  expect(host.querySelector<HTMLButtonElement>('button[aria-label="下一页"]')!.disabled).toBe(true);
  await act(async () => tabs.querySelectorAll<HTMLButtonElement>('[role="tab"]')[0]!.focus());
  expect(location.pathname).toBe("/zh-CN/workspaces/2/code-connections/42");
  expect(host.textContent).toContain("Team Codeup");
});

it("opens monitoring settings with saved values and saves without navigating away", async () => {
  required = false;
  permissions = ["repositories.read", "repositories.update"];
  const original = vi.mocked(fetch).getMockImplementation()!;
  vi.stubGlobal("fetch", vi.fn(async (url: string, options?: RequestInit) => {
    if (url === "/api/workspaces/2/code-connections/42") return { ok: true, status: 200, json: async () => ({ id: "42", name: "Team Codeup", provider: "codeup", version: 1 }) };
    if (url === "/api/workspaces/2/code-connection-monitors/42") return { ok: true, status: 200, json: async () => ({ id: "42", provider: "codeup", enabled: true, all_repositories: false, repositories: "org/repo", events: ["push"], version: 3, has_secret: true, callback_path: "/api/repository-events/42" }) };
    return original(url, options);
  }));
  history.replaceState({}, "", "/zh-CN/workspaces/2/code-connections/42?tab=monitoring");
  await act(async () => root.render(<App />));
  expect(host.querySelector('[aria-label="仓库事件记录"]')).toBeNull();
  expect(host.textContent).toContain("已设置");
  await act(async () => host.querySelector<HTMLButtonElement>('section[aria-label="订阅配置"] button[aria-label="修改订阅配置"]')!.click());
  const dialog = document.querySelector('[role="dialog"]')!;
  expect(dialog.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("org/repo");
  expect(dialog.textContent).not.toContain("新增仓库");
  expect(dialog.textContent).not.toContain("删除仓库");
  await act(async () => dialog.querySelector<HTMLFormElement>("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  const request = vi.mocked(fetch).mock.calls.find(([url, init]) => url === "/api/workspaces/2/code-connection-monitors/42" && init?.method === "PATCH");
  expect(JSON.parse(request?.[1]?.body as string)).toMatchObject({ enabled: true, all_repositories: false, repositories: "org/repo", events: ["push"], version: 3 });
  expect(location.pathname).toBe("/zh-CN/workspaces/2/code-connections/42");
  expect(location.search).toContain("tab=monitoring");
});

it("edits Git monitoring through a named Runner selector and polling-only events", async () => {
  required = false;
  permissions = ["repositories.read", "repositories.update", "hosts.read", "hosts.terminal"];
  const original = vi.mocked(fetch).getMockImplementation()!;
  vi.stubGlobal("fetch", vi.fn(async (url: string, options?: RequestInit) => {
    if (url === "/api/workspaces/2/code-connection-monitors/42") return { ok: true, status: 200, json: async () => ({ id: "42", provider: "git", mode: "poll", enabled: true, runner_id: "7", runner_name: "Build Runner", all_repositories: false, repositories: "https://git.example.invalid/org/repo.git", events: ["push", "tag.updated", "merge"], poll_status: "等待检查", version: 4 }) };
    if (url.startsWith("/api/workspaces/2/nodes?")) {
      const page = new URL(url, location.origin).searchParams.get("page");
      return { ok: true, status: 200, json: async () => ({ items: page === "1" ? [{ id: "8", name: "SSH host", connection: "ssh" }] : [{ id: "7", name: "Build Runner", connection: "runner" }], total: 101 }) };
    }
    return original(url, options);
  }));
  history.replaceState({}, "", "/zh-CN/workspaces/2/code-connections/42?tab=monitoring");
  await act(async () => root.render(<RepositoryMonitor workspace="2" connectionId="42" provider="git" permissions={new Set(permissions)} />));
  expect(host.textContent).toContain("Build Runner");
  expect(host.textContent).toContain("等待检查");
  expect(host.textContent).not.toContain("SHA256");
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="修改订阅配置"]')!.click());
  const dialog = document.querySelector('[role="dialog"]')!;
  expect(dialog.querySelector<HTMLTextAreaElement>("textarea")?.value).toBe("https://git.example.invalid/org/repo.git");
  expect(dialog.querySelector('[role="combobox"]')?.textContent).toContain("Build Runner");
  expect(dialog.textContent).not.toContain("SSH host");
  expect(dialog.textContent).not.toContain("全部仓库");
  expect(dialog.textContent).not.toContain("监控类型");
  expect(dialog.textContent).not.toContain("Webhook 验证密钥");
  expect(dialog.textContent).not.toContain("合并完成");
  expect(dialog.textContent).not.toContain("新增仓库");
  expect(dialog.textContent).toContain("更新标签");
  expect(dialog.querySelectorAll('[role="checkbox"]')).toHaveLength(7); // Six events and the enable switch.
  expect(vi.mocked(fetch).mock.calls.filter(([url]) => String(url).includes("/nodes?")).map(([url]) => new URL(String(url), location.origin).searchParams.get("page"))).toEqual(["1", "2"]);
  await act(async () => dialog.querySelector<HTMLFormElement>("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  const request = vi.mocked(fetch).mock.calls.find(([url, init]) => url === "/api/workspaces/2/code-connection-monitors/42" && init?.method === "PATCH");
  expect(JSON.parse(request?.[1]?.body as string)).toMatchObject({ runner_id: "7", mode: "poll", all_repositories: false, poll_interval_minutes: 1, events: ["push", "tag.updated"], version: 4 });
});

it.each([["hosts.read"], ["hosts.terminal"]])("keeps Git monitoring read-only without both Runner permissions: %s", async permission => {
  history.replaceState({}, "", "/zh-CN/workspaces/2/code-connections/42?tab=monitoring");
  const original = vi.mocked(fetch).getMockImplementation()!;
  vi.stubGlobal("fetch", vi.fn(async (url: string, options?: RequestInit) => url === "/api/workspaces/2/code-connection-monitors/42"
    ? { ok: true, status: 200, json: async () => ({ id: "42", provider: "git", mode: "poll", runner_id: "7", runner_name: "Build Runner", events: ["push"] }) }
    : original(url, options)));
  await act(async () => root.render(<RepositoryMonitor workspace="2" connectionId="42" provider="git" permissions={new Set(["repositories.read", "repositories.update", permission])} />));
  expect(host.textContent).toContain("Build Runner");
  expect(host.textContent).toContain("需要主机读取及终端执行权限");
  expect(host.querySelector('button[aria-label="修改订阅配置"]')).toBeNull();
  expect(vi.mocked(fetch).mock.calls.some(([url]) => String(url).includes("/nodes?"))).toBe(false);
});

it("uses the Git event set only for Git connection filters", () => {
  const options = (provider: string) => repositoryEventList("2", "42", provider).filters?.find(filter => filter.key === "event")?.options;
  expect(options("git")).toEqual(expect.arrayContaining([{ value: "tag.updated", label: "更新标签" }]));
  expect(options("git")).toHaveLength(6);
  expect(options("codeup")).not.toEqual(expect.arrayContaining([expect.objectContaining({ value: "tag.updated" })]));
  expect(options("codeup")).toEqual(expect.arrayContaining([expect.objectContaining({ value: "pull_request" }), expect.objectContaining({ value: "merge" })]));
});

it.each([false, true])("blocks Git monitoring without a selected Runner when option loading fails: %s", async failed => {
  history.replaceState({}, "", "/zh-CN/workspaces/2/code-connections/42?tab=monitoring");
  const original = vi.mocked(fetch).getMockImplementation()!;
  vi.stubGlobal("fetch", vi.fn(async (url: string, options?: RequestInit) => {
    if (url === "/api/workspaces/2/code-connection-monitors/42") return { ok: true, status: 200, json: async () => ({ id: "42", provider: "git", mode: "poll", runner_id: "", repositories: "https://git.example.invalid/org/repo.git", events: ["push"] }) };
    if (url.startsWith("/api/workspaces/2/nodes?")) return { ok: !failed, status: failed ? 403 : 200, json: async () => failed ? { error: "Runner access denied" } : { items: [], total: 0 } };
    return original(url, options);
  }));
  await act(async () => root.render(<RepositoryMonitor workspace="2" connectionId="42" provider="git" permissions={new Set(["repositories.read", "repositories.update", "hosts.read", "hosts.terminal"])} />));
  await act(async () => host.querySelector<HTMLButtonElement>('button[aria-label="修改订阅配置"]')!.click());
  const dialog = document.querySelector('[role="dialog"]')!;
  expect(dialog.querySelector<HTMLButtonElement>('button[type="submit"]')?.disabled).toBe(true);
  if (failed) expect(dialog.querySelector('[role="alert"]')?.textContent).toContain("Runner access denied");
  await act(async () => dialog.querySelector<HTMLFormElement>("form")!.dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
  expect(vi.mocked(fetch).mock.calls.some(([, init]) => init?.method === "PATCH")).toBe(false);
});

it("scopes credential menus, lists and create payloads to the current module", async () => {
  required = false;
  permissions = ["credentials.read", "credentials.create"];
  for (const scope of ["host", "build", "deploy"]) {
    history.replaceState({}, "", `/zh-CN/workspaces/2/credentials?scope=${scope}`);
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
  history.replaceState({}, "", "/zh-CN/workspaces/2/code-connections");
  await act(async () => root.render(<App />));
  const sidebar = host.querySelector('nav[aria-label="构建"]')!;
  expect([...sidebar.querySelectorAll("a")].map(a => a.textContent)).toEqual(["仓库连接", "凭据"]);
  expect(sidebar.querySelector('[aria-current="page"]')?.textContent).toBe("仓库连接");
  expect(host.querySelector('nav[aria-label="仓库页面"]')).toBeNull();
  expect(host.textContent).not.toContain("平台授权");
});
it("groups deployment history and configuration files under application deployment", async () => {
  required = false;
  permissions = ["deployments.read", "config-templates.read"];
  history.replaceState({}, "", "/zh-CN/workspaces/2/config-templates");
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
  history.replaceState({}, "", "/zh-CN/ops/build/repository-auth/github");
  await act(async () => root.render(<App />));
  expect(host.querySelector("h1")?.textContent).toBe("仓库连接");
  expect(host.textContent).not.toContain("平台授权");
});
it("does not expose repository creation to a credentials-only role", async () => {
  required = false;
  permissions = ["credentials.read"];
  history.replaceState({}, "", "/zh-CN/workspaces/2/credentials");
  await act(async () => root.render(<App />));
  expect([...host.querySelectorAll('nav[aria-label="构建"] a')].map(a => a.textContent)).toEqual(["凭据"]);
});
it("gives pipeline creation a full-page editor with its own action bar", async () => {
  required = false;
  permissions = ["pipelines.read", "pipelines.create", "pipelines.run"];
  history.replaceState({}, "", "/zh-CN/workspaces/2/pipelines/new");
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
  history.replaceState({}, "", "/zh-CN/workspaces/2/pipelines?page=3&at=120");
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
  expect(location.pathname).toBe("/zh-CN/workspaces/2/pipelines");
});
it("reorders pipeline groups from the directory", async () => {
  required = false;
  permissions = ["pipelines.read", "pipeline-groups.read", "pipeline-groups.update"];
  history.replaceState({}, "", "/zh-CN/workspaces/2/pipelines");
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
  history.replaceState({}, "", "/zh-CN/workspaces/2/nodes");
  await act(async () => root.render(<App />));
  expect(host.querySelector('nav[aria-label="主机页面"]')).toBeNull();
  const directory = host.querySelector('aside[aria-label="主机分组"]')!;
  expect(directory.textContent).toContain("全部节点");
  expect(directory.textContent).toContain("未分组节点");
  await act(async () => (directory.querySelector<HTMLButtonElement>('button[aria-label="新建分组"]')!).click());
  expect(document.querySelector('[role="dialog"]')?.textContent).toContain("新建分组");
  expect(location.pathname).toBe("/zh-CN/workspaces/2/nodes");
});
it("redirects old group pages to the corresponding directory", async () => {
  required = false;
  permissions = ["hosts.read", "host-groups.read"];
  history.replaceState({}, "", "/zh-CN/workspaces/2/node-groups/42/edit");
  await act(async () => root.render(<App />));
  expect(location.pathname).toBe("/zh-CN/workspaces/2/nodes");
  expect(location.search).toContain("group_id=42");
});
